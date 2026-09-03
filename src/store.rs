//! 存储层：全量数据常驻内存，每次写操作后落盘为一个 JSON 文件。
//! 团队规模下数据量极小，这样最简单也最容易人工查看与备份。

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::date;
use crate::model::*;

/// 落盘的数据快照
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    next_member_id: u32,
    #[serde(default)]
    next_task_id: u32,
    #[serde(skip)]
    path: PathBuf,
}

impl Store {
    /// 从文件加载；文件不存在时生成一份演示数据
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        match fs::read_to_string(&path) {
            Ok(text) => {
                let mut s = serde_json::from_str::<Store>(&text)
                    .map_err(|e| format!("{} 格式损坏（{e}），原文件已保留", path.display()))?;
                s.path = path;
                // 兼容没有 next_* 字段或计数器落后的旧数据，避免新增记录撞号。
                s.next_member_id = s
                    .next_member_id
                    .max(s.members.iter().map(|m| m.id).max().unwrap_or(0));
                s.next_task_id = s
                    .next_task_id
                    .max(s.tasks.iter().map(|t| t.id).max().unwrap_or(0));
                Ok(s)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                let mut s = Store {
                    path,
                    ..Default::default()
                };
                s.seed();
                s.save();
                Ok(s)
            }
            Err(e) => Err(format!("无法读取 {}: {e}", path.display())),
        }
    }

    /// 写回磁盘，失败时只告警不中断服务；未指定路径时为纯内存模式
    pub fn save(&self) {
        if self.path.as_os_str().is_empty() {
            return;
        }
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                let tmp = self.path.with_extension("json.tmp");
                let result = fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, &self.path));
                if let Err(e) = result {
                    let _ = fs::remove_file(&tmp);
                    eprintln!("[moonlight] 数据落盘失败: {e}");
                }
            }
            Err(e) => eprintln!("[moonlight] 数据序列化失败: {e}"),
        }
    }

    pub fn member(&self, id: u32) -> Option<&Member> {
        self.members.iter().find(|m| m.id == id)
    }

    pub fn task(&self, id: u32) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn add_member(&mut self, input: NewMember) -> Member {
        self.next_member_id += 1;
        let m = Member {
            id: self.next_member_id,
            name: input.name,
            role: input.role,
            weekly_hours: input.weekly_hours.max(1.0),
            hourly_cost: input.hourly_cost.max(1.0),
        };
        self.members.push(m.clone());
        self.save();
        m
    }

    /// 删除成员，并把其名下任务退回未派工状态
    pub fn remove_member(&mut self, id: u32) -> bool {
        let before = self.members.len();
        self.members.retain(|m| m.id != id);
        if self.members.len() == before {
            return false;
        }
        for t in self.tasks.iter_mut().filter(|t| t.assignee == Some(id)) {
            t.assignee = None;
        }
        self.save();
        true
    }

    pub fn add_task(&mut self, input: NewTask) -> Task {
        self.next_task_id += 1;
        let value = input.value.max(0.0);
        let t = Task {
            id: self.next_task_id,
            title: input.title,
            skill: input.skill,
            priority: input.priority,
            status: Status::Todo,
            assignee: input.assignee.filter(|id| self.member(*id).is_some()),
            estimate: input.estimate.max(0.5),
            due: input.due,
            value,
            // 未显式登记延期损失时，按收益与优先级推导一个基准值
            delay_cost_per_day: input
                .delay_cost_per_day
                .filter(|v| *v > 0.0)
                .unwrap_or_else(|| crate::economics::derive_delay_cost(value, input.priority)),
        };
        self.tasks.push(t.clone());
        self.save();
        t
    }

    pub fn patch_task(&mut self, id: u32, patch: TaskPatch) -> Option<Task> {
        // 先校验负责人是否存在，避免写入悬空 id
        let assignee_valid = match patch.assignee {
            Some(Some(mid)) => self.member(mid).is_some(),
            _ => true,
        };
        let t = self.tasks.iter_mut().find(|t| t.id == id)?;
        if let Some(v) = patch.title {
            t.title = v.trim().to_string();
        }
        if let Some(v) = patch.skill {
            t.skill = v;
        }
        if let Some(v) = patch.priority {
            t.priority = v;
        }
        if let Some(v) = patch.status {
            t.status = v;
        }
        if let Some(v) = patch.estimate {
            t.estimate = v.max(0.5);
        }
        if let Some(v) = patch.due {
            t.due = v;
        }
        if let Some(v) = patch.value {
            t.value = v.max(0.0);
        }
        if let Some(v) = patch.delay_cost_per_day {
            t.delay_cost_per_day = v.max(0.0);
        }
        if let Some(v) = patch.assignee {
            if assignee_valid {
                t.assignee = v;
            }
        }
        let updated = t.clone();
        self.save();
        Some(updated)
    }

    pub fn remove_task(&mut self, id: u32) -> bool {
        let before = self.tasks.len();
        self.tasks.retain(|t| t.id != id);
        if self.tasks.len() == before {
            return false;
        }
        self.save();
        true
    }

    /// 首次启动时写入一组演示数据，避免打开界面是一片空白
    fn seed(&mut self) {
        // 姓名、学科、周产能、人力成本（元/小时，全成本口径：含社保公积金与管理分摊）
        // 团队里没有运维学科，运维类任务只能跨学科承接，用于演示能力缺口的代价
        let members = [
            ("林洲", Role::Backend, 32.0, 520.0),
            ("周舟", Role::Frontend, 30.0, 430.0),
            ("陆晚", Role::Qa, 28.0, 330.0),
            ("何言", Role::Backend, 24.0, 380.0),
            ("苏念", Role::Product, 20.0, 450.0),
        ];
        for (name, role, hours, cost) in members {
            self.add_member(NewMember {
                name: name.to_string(),
                role,
                weekly_hours: hours,
                hourly_cost: cost,
            });
        }

        // 标题、学科、优先级、工时、截止日偏移、负责人、预期业务收益（元，季度可量化口径）
        let tasks = [
            (
                "鉴权网关灰度放量",
                Role::Backend,
                Priority::P0,
                12.0,
                2,
                Some(1),
                45_000.0,
            ),
            (
                "工时看板前端重构",
                Role::Frontend,
                Priority::P1,
                16.0,
                6,
                Some(2),
                26_000.0,
            ),
            (
                "发布流水线回归用例",
                Role::Qa,
                Priority::P1,
                10.0,
                4,
                Some(3),
                20_000.0,
            ),
            (
                "消息推送重复投递修复",
                Role::Backend,
                Priority::P0,
                8.0,
                1,
                None,
                32_000.0,
            ),
            (
                "季度需求池梳理",
                Role::Product,
                Priority::P2,
                6.0,
                12,
                Some(5),
                9_000.0,
            ),
            (
                "数据库慢查询治理",
                Role::Backend,
                Priority::P1,
                14.0,
                -1,
                Some(4),
                28_000.0,
            ),
            // 测试同学跨学科接前端活，用于演示学科距离带来的工时与成本上浮
            (
                "移动端埋点补齐",
                Role::Frontend,
                Priority::P2,
                9.0,
                9,
                Some(3),
                12_000.0,
            ),
            (
                "灰度配置中心接入",
                Role::Ops,
                Priority::P1,
                10.0,
                8,
                None,
                16_000.0,
            ),
            (
                "用户增长看板取数",
                Role::Backend,
                Priority::P2,
                12.0,
                15,
                None,
                10_000.0,
            ),
            // 投入远大于收益，用于演示净现值为负时的砍需求决策
            (
                "老旧接口文档整理",
                Role::Product,
                Priority::P2,
                20.0,
                20,
                None,
                2_000.0,
            ),
            (
                "客服工单导出优化",
                Role::Frontend,
                Priority::P1,
                11.0,
                5,
                None,
                15_000.0,
            ),
            (
                "压测环境扩容",
                Role::Ops,
                Priority::P1,
                8.0,
                7,
                None,
                11_000.0,
            ),
            (
                "埋点数据质量校验",
                Role::Qa,
                Priority::P2,
                10.0,
                18,
                None,
                7_000.0,
            ),
        ];
        for (title, skill, priority, estimate, due_offset, assignee, value) in tasks {
            self.add_task(NewTask {
                title: title.to_string(),
                skill,
                priority,
                estimate,
                due: date::from_today(due_offset),
                assignee,
                value,
                delay_cost_per_day: None,
            });
        }
        // 让演示数据的状态更有层次
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == 1) {
            t.status = Status::Doing;
        }
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == 3) {
            t.status = Status::Review;
        }
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == 5) {
            t.status = Status::Done;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 空仓库() -> Store {
        Store::default()
    }

    #[test]
    fn 删除成员应当退回其名下任务() {
        let mut s = 空仓库();
        let m = s.add_member(NewMember {
            name: "测试".into(),
            role: Role::Backend,
            weekly_hours: 40.0,
            hourly_cost: 100.0,
        });
        let t = s.add_task(NewTask {
            title: "任务".into(),
            skill: Role::Backend,
            priority: Priority::P1,
            estimate: 4.0,
            due: "2026-12-01".into(),
            assignee: Some(m.id),
            value: 10_000.0,
            delay_cost_per_day: None,
        });
        assert!(s.remove_member(m.id));
        assert_eq!(s.task(t.id).unwrap().assignee, None);
    }

    #[test]
    fn 补丁应当只修改出现的字段() {
        let mut s = 空仓库();
        let t = s.add_task(NewTask {
            title: "任务".into(),
            skill: Role::Qa,
            priority: Priority::P2,
            estimate: 4.0,
            due: "2026-12-01".into(),
            assignee: None,
            value: 10_000.0,
            delay_cost_per_day: None,
        });
        let patch = TaskPatch {
            status: Some(Status::Doing),
            ..Default::default()
        };
        let updated = s.patch_task(t.id, patch).unwrap();
        assert_eq!(updated.status, Status::Doing);
        assert_eq!(updated.title, "任务");
        assert_eq!(updated.estimate, 4.0);
    }

    #[test]
    fn 派工到不存在的成员应当被忽略() {
        let mut s = 空仓库();
        let t = s.add_task(NewTask {
            title: "任务".into(),
            skill: Role::Ops,
            priority: Priority::P1,
            estimate: 4.0,
            due: "2026-12-01".into(),
            assignee: Some(999),
            value: 10_000.0,
            delay_cost_per_day: None,
        });
        assert_eq!(t.assignee, None);
    }

    fn 临时文件(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "moonlight-{name}-{}-{}.json",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ))
    }

    #[test]
    fn 损坏的数据文件不应被覆盖() {
        let path = 临时文件("corrupt");
        fs::write(&path, "{broken-json").unwrap();

        let result = Store::load(&path);

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{broken-json");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn 加载旧数据后应修复编号计数器() {
        let path = 临时文件("legacy");
        fs::write(
            &path,
            r#"{
              "members": [{"id": 41, "name": "旧成员", "role": "backend", "weekly_hours": 40, "hourly_cost": 100}],
              "tasks": []
            }"#,
        )
        .unwrap();

        let mut store = Store::load(&path).unwrap();
        let created = store.add_member(NewMember {
            name: "新成员".into(),
            role: Role::Qa,
            weekly_hours: 40.0,
            hourly_cost: 100.0,
        });

        assert_eq!(created.id, 42);
        fs::remove_file(path).unwrap();
    }
}
