//! 核心算法：把「谁在忙、谁能接、哪里要出事」这三件事算清楚。
//! 这里是整个工具的价值所在，其余模块只负责搬运数据。

use serde::Serialize;

use crate::date;
use crate::model::*;
use crate::store::Store;

/// 技能匹配在派工评分中的占比
const W_SKILL: f64 = 0.45;
/// 剩余容量在派工评分中的占比
const W_CAPACITY: f64 = 0.30;
/// 预计人力成本在派工评分中的占比
const W_ECONOMY: f64 = 0.25;
/// 技能不匹配时的兜底得分（允许跨技能救火，但明显降权）
const SKILL_MISMATCH: f64 = 0.35;
/// 负载率超过该值判定为过载
const OVERLOAD: f64 = 1.0;
/// 负载率超过该值判定为吃紧
const BUSY: f64 = 0.85;
/// 临期预警的天数窗口
const DUE_SOON_DAYS: i64 = 3;

/// 单个成员的负载画像
#[derive(Debug, Clone, Serialize)]
pub struct Workload {
    pub member_id: u32,
    pub name: String,
    pub role: Role,
    /// 周容量工时
    pub capacity: f64,
    /// 未完成任务的预估工时之和
    pub assigned: f64,
    /// 负载率 = assigned / capacity
    pub ratio: f64,
    /// 未完成任务数
    pub open_tasks: usize,
    /// 名下最紧急的截止日期
    pub nearest_due: Option<String>,
}

/// 派工建议
#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    pub member_id: u32,
    pub name: String,
    /// 0~1 的综合得分
    pub score: f64,
    /// 考虑跨技能效率损失后的预计人力成本
    pub projected_cost: f64,
    /// 预期投资回报率 = (业务价值 - 人力成本) / 人力成本
    pub roi: f64,
    pub reason: String,
}

/// 未完成任务组合的经济性摘要
#[derive(Debug, Clone, Serialize)]
pub struct Economics {
    pub projected_cost: f64,
    pub business_value: f64,
    pub portfolio_roi: f64,
}

/// 预警条目
#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    /// danger / warn / info
    pub level: &'static str,
    pub text: String,
}

/// 团队健康度
#[derive(Debug, Clone, Serialize)]
pub struct Health {
    /// 0~100 综合分
    pub score: u32,
    /// 未逾期比例
    pub on_time: f64,
    /// 负载均衡度
    pub balance: f64,
    /// 派工覆盖率
    pub coverage: f64,
    pub comment: &'static str,
}

/// 统计每个成员的负载，未完成任务才计入
pub fn workloads(store: &Store) -> Vec<Workload> {
    store
        .members
        .iter()
        .map(|m| {
            let mine: Vec<&Task> = store
                .tasks
                .iter()
                .filter(|t| t.assignee == Some(m.id) && t.status.is_open())
                .collect();
            let assigned = mine.iter().fold(0.0, |acc, t| acc + t.estimate);
            let nearest_due = mine
                .iter()
                .filter_map(|t| date::parse(&t.due))
                .min()
                .map(date::format);
            Workload {
                member_id: m.id,
                name: m.name.clone(),
                role: m.role,
                capacity: m.weekly_hours,
                assigned,
                ratio: assigned / m.weekly_hours,
                open_tasks: mine.len(),
                nearest_due,
            }
        })
        .collect()
}

/// 对单个任务给出候选人排名（按得分从高到低）
pub fn suggest(store: &Store, task: &Task) -> Vec<Suggestion> {
    let loads = workloads(store);
    rank(store, task, &loads)
}

/// 基于给定负载快照给候选人打分，供 suggest 与 auto_assign 共用
fn rank(store: &Store, task: &Task, loads: &[Workload]) -> Vec<Suggestion> {
    let candidate_costs: Vec<(u32, f64)> = store
        .members
        .iter()
        .map(|m| (m.id, projected_cost(m, task)))
        .collect();
    let cheapest = candidate_costs
        .iter()
        .map(|(_, cost)| *cost)
        .fold(f64::INFINITY, f64::min);
    let mut out: Vec<Suggestion> = store
        .members
        .iter()
        .map(|m| {
            let load = loads.iter().find(|w| w.member_id == m.id);
            // 用「接下这个任务之后」的负载率打分，避免快满载的人被继续压任务
            let after = (load.map(|w| w.assigned).unwrap_or(0.0) + task.estimate) / m.weekly_hours;

            let skill_score = if m.role == task.skill {
                1.0
            } else {
                SKILL_MISMATCH
            };
            // 负载越低得分越高，1.5 倍容量以上直接归零
            let capacity_score = (1.0 - (after / 1.5).min(1.0)).max(0.0);
            let cost = candidate_costs
                .iter()
                .find(|(id, _)| *id == m.id)
                .map(|(_, cost)| *cost)
                .unwrap_or(0.0);
            let economy_score = if cost > 0.0 { cheapest / cost } else { 1.0 };
            let roi = if cost > 0.0 {
                (task.business_value - cost) / cost
            } else {
                0.0
            };
            let score =
                W_SKILL * skill_score + W_CAPACITY * capacity_score + W_ECONOMY * economy_score;

            let reason = if m.role == task.skill {
                format!(
                    "{}技能匹配，接单后负载 {:.0}%，成本 ¥{:.0}，ROI {:.0}%",
                    m.role.label(),
                    after * 100.0,
                    cost,
                    roi * 100.0
                )
            } else {
                format!(
                    "跨技能支援（{}→{}），接单后负载 {:.0}%，成本 ¥{:.0}，ROI {:.0}%",
                    m.role.label(),
                    task.skill.label(),
                    after * 100.0,
                    cost,
                    roi * 100.0
                )
            };
            Suggestion {
                member_id: m.id,
                name: m.name.clone(),
                score,
                projected_cost: cost,
                roi,
                reason,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

fn projected_cost(member: &Member, task: &Task) -> f64 {
    let productivity = if member.role == task.skill { 1.0 } else { 0.65 };
    task.estimate / productivity * member.hourly_cost
}

/// 汇总项目组合的预计人力成本、业务价值和 ROI。
/// 已派工任务按实际负责人计算，未派工任务取候选人中的最低预计成本。
pub fn economics(store: &Store) -> Economics {
    let open = store.tasks.iter().filter(|task| task.status.is_open());
    let mut cost = 0.0;
    let mut value = 0.0;
    for task in open {
        let task_cost = task
            .assignee
            .and_then(|id| store.member(id))
            .map(|member| projected_cost(member, task))
            .or_else(|| {
                store
                    .members
                    .iter()
                    .map(|member| projected_cost(member, task))
                    .reduce(f64::min)
            })
            .unwrap_or(0.0);
        cost += task_cost;
        value += task.business_value;
    }
    Economics {
        projected_cost: cost,
        business_value: value,
        portfolio_roi: if cost > 0.0 {
            (value - cost) / cost
        } else {
            0.0
        },
    }
}

/// 一键派工：把所有未派工的未完成任务分给当前最合适的人。
/// 按「优先级 → 截止日 → 工时」排序后贪心分配，每分配一个就更新负载快照，
/// 因此不会把同一批任务全部堆到同一个人身上。
pub fn auto_assign(store: &mut Store) -> Vec<(u32, u32)> {
    if store.members.is_empty() {
        return Vec::new();
    }
    let mut loads = workloads(store);

    let mut pending: Vec<Task> = store
        .tasks
        .iter()
        .filter(|t| t.assignee.is_none() && t.status.is_open())
        .cloned()
        .collect();
    pending.sort_by(|a, b| {
        b.priority
            .weight()
            .partial_cmp(&a.priority.weight())
            .unwrap()
            .then(
                date::parse(&a.due)
                    .unwrap_or(i64::MAX)
                    .cmp(&date::parse(&b.due).unwrap_or(i64::MAX)),
            )
            .then(b.estimate.partial_cmp(&a.estimate).unwrap())
    });

    let mut result = Vec::new();
    for task in pending {
        let Some(best) = rank(store, &task, &loads).into_iter().next() else {
            continue;
        };
        // 更新负载快照，让后续任务看到最新的忙闲情况
        if let Some(w) = loads.iter_mut().find(|w| w.member_id == best.member_id) {
            w.assigned += task.estimate;
            w.ratio = w.assigned / w.capacity;
            w.open_tasks += 1;
        }
        if let Some(t) = store.tasks.iter_mut().find(|t| t.id == task.id) {
            t.assignee = Some(best.member_id);
        }
        result.push((task.id, best.member_id));
    }
    if !result.is_empty() {
        store.save();
    }
    result
}

/// 汇总风险预警，按严重程度排序
pub fn alerts(store: &Store, loads: &[Workload]) -> Vec<Alert> {
    let today = date::today();
    let mut out = Vec::new();

    for t in store.tasks.iter().filter(|t| t.status.is_open()) {
        let owner = t
            .assignee
            .and_then(|id| store.member(id))
            .map(|m| m.name.as_str())
            .unwrap_or("待派工");
        match date::parse(&t.due) {
            Some(due) if due < today => out.push(Alert {
                level: "danger",
                text: format!("《{}》已逾期 {} 天（{}）", t.title, today - due, owner),
            }),
            Some(due) if due - today <= DUE_SOON_DAYS && t.status == Status::Todo => {
                out.push(Alert {
                    level: "warn",
                    text: format!(
                        "《{}》还剩 {} 天到期但尚未开工（{}）",
                        t.title,
                        due - today,
                        owner
                    ),
                })
            }
            _ => {}
        }
        if t.assignee.is_none() && t.priority == Priority::P0 {
            out.push(Alert {
                level: "danger",
                text: format!("P0 任务《{}》仍未派工", t.title),
            });
        }
        // 跨技能派工提示：能跑通但通常更慢
        if let Some(m) = t.assignee.and_then(|id| store.member(id)) {
            if m.role != t.skill {
                out.push(Alert {
                    level: "info",
                    text: format!(
                        "《{}》需要{}能力，当前由{}（{}）承担",
                        t.title,
                        t.skill.label(),
                        m.name,
                        m.role.label()
                    ),
                });
            }
        }
    }

    let has_pending = store
        .tasks
        .iter()
        .any(|t| t.assignee.is_none() && t.status.is_open());
    for w in loads {
        if w.ratio > OVERLOAD {
            out.push(Alert {
                level: "danger",
                text: format!(
                    "{} 负载 {:.0}%（{:.1}h / {:.1}h），建议转移任务",
                    w.name,
                    w.ratio * 100.0,
                    w.assigned,
                    w.capacity
                ),
            });
        } else if w.ratio > BUSY {
            out.push(Alert {
                level: "warn",
                text: format!("{} 负载 {:.0}%，接近满载", w.name, w.ratio * 100.0),
            });
        } else if w.open_tasks == 0 && has_pending {
            out.push(Alert {
                level: "info",
                text: format!("{} 当前无在手任务，可承接待派工需求", w.name),
            });
        }
    }

    let order = |l: &str| match l {
        "danger" => 0,
        "warn" => 1,
        _ => 2,
    };
    out.sort_by_key(|a| order(a.level));
    out
}

/// 团队健康度：按时率 40%、负载均衡 35%、派工覆盖 25%
pub fn health(store: &Store, loads: &[Workload]) -> Health {
    let today = date::today();
    let open: Vec<&Task> = store.tasks.iter().filter(|t| t.status.is_open()).collect();

    let on_time = if open.is_empty() {
        1.0
    } else {
        let late = open
            .iter()
            .filter(|t| date::parse(&t.due).map(|d| d < today).unwrap_or(false))
            .count();
        1.0 - late as f64 / open.len() as f64
    };

    let coverage = if open.is_empty() {
        1.0
    } else {
        open.iter().filter(|t| t.assignee.is_some()).count() as f64 / open.len() as f64
    };

    // 均衡度用负载率的标准差衡量，差异越小越健康
    let balance = if loads.len() < 2 {
        1.0
    } else {
        let mean = loads.iter().map(|w| w.ratio).sum::<f64>() / loads.len() as f64;
        let var = loads.iter().map(|w| (w.ratio - mean).powi(2)).sum::<f64>() / loads.len() as f64;
        (1.0 - var.sqrt().min(1.0)).max(0.0)
    };

    let score = ((0.40 * on_time + 0.35 * balance + 0.25 * coverage) * 100.0).round() as u32;
    let comment = match score {
        90..=100 => "节奏稳健，保持当前排期",
        75..=89 => "整体可控，关注个别预警",
        60..=74 => "存在明显瓶颈，建议重新派工",
        _ => "风险偏高，需要立即介入调整",
    };

    Health {
        score,
        on_time,
        balance,
        coverage,
        comment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 造成员(s: &mut Store, name: &str, role: Role, hours: f64) -> u32 {
        s.add_member(NewMember {
            name: name.into(),
            role,
            weekly_hours: hours,
            hourly_cost: 100.0,
        })
        .id
    }

    fn 造任务(s: &mut Store, title: &str, skill: Role, p: Priority, est: f64, due: &str) -> u32 {
        s.add_task(NewTask {
            title: title.into(),
            skill,
            priority: p,
            estimate: est,
            business_value: 1000.0,
            due: due.into(),
            assignee: None,
        })
        .id
    }

    #[test]
    fn 负载统计不计入已完成任务() {
        let mut s = Store::default();
        let a = 造成员(&mut s, "A", Role::Backend, 20.0);
        let t1 = 造任务(&mut s, "T1", Role::Backend, Priority::P1, 8.0, "2026-12-01");
        let t2 = 造任务(&mut s, "T2", Role::Backend, Priority::P1, 8.0, "2026-12-01");
        s.patch_task(
            t1,
            TaskPatch {
                assignee: Some(Some(a)),
                ..Default::default()
            },
        );
        s.patch_task(
            t2,
            TaskPatch {
                assignee: Some(Some(a)),
                status: Some(Status::Done),
                ..Default::default()
            },
        );
        let w = &workloads(&s)[0];
        assert_eq!(w.assigned, 8.0);
        assert_eq!(w.open_tasks, 1);
    }

    #[test]
    fn 派工优先技能匹配者() {
        let mut s = Store::default();
        造成员(&mut s, "前端", Role::Frontend, 40.0);
        let be = 造成员(&mut s, "后端", Role::Backend, 40.0);
        let t = 造任务(
            &mut s,
            "接口开发",
            Role::Backend,
            Priority::P0,
            8.0,
            "2026-12-01",
        );
        let task = s.task(t).unwrap().clone();
        assert_eq!(suggest(&s, &task)[0].member_id, be);
    }

    #[test]
    fn 同技能时应当选择更空闲的人() {
        let mut s = Store::default();
        let busy = 造成员(&mut s, "忙", Role::Backend, 20.0);
        let idle = 造成员(&mut s, "闲", Role::Backend, 20.0);
        let occupied = 造任务(
            &mut s,
            "占用",
            Role::Backend,
            Priority::P1,
            18.0,
            "2026-12-01",
        );
        s.patch_task(
            occupied,
            TaskPatch {
                assignee: Some(Some(busy)),
                ..Default::default()
            },
        );
        let t = 造任务(
            &mut s,
            "新活",
            Role::Backend,
            Priority::P1,
            4.0,
            "2026-12-01",
        );
        let task = s.task(t).unwrap().clone();
        assert_eq!(suggest(&s, &task)[0].member_id, idle);
    }

    #[test]
    fn 自动派工应当分散而非堆到一人() {
        let mut s = Store::default();
        造成员(&mut s, "A", Role::Backend, 10.0);
        造成员(&mut s, "B", Role::Backend, 10.0);
        for i in 0..4 {
            造任务(
                &mut s,
                &format!("T{i}"),
                Role::Backend,
                Priority::P1,
                5.0,
                "2026-12-01",
            );
        }
        let pairs = auto_assign(&mut s);
        assert_eq!(pairs.len(), 4);
        let loads = workloads(&s);
        assert!(loads.iter().all(|w| w.open_tasks == 2), "任务应当被均分");
    }

    #[test]
    fn 逾期与未派工的_p0_都应当报警() {
        let mut s = Store::default();
        造成员(&mut s, "A", Role::Backend, 40.0);
        造任务(
            &mut s,
            "逾期活",
            Role::Backend,
            Priority::P0,
            4.0,
            &date::from_today(-2),
        );
        let list = alerts(&s, &workloads(&s));
        assert!(list.iter().any(|a| a.text.contains("已逾期")));
        assert!(list.iter().any(|a| a.text.contains("仍未派工")));
    }

    #[test]
    fn 空团队健康度不应当为零() {
        let s = Store::default();
        assert_eq!(health(&s, &[]).score, 100);
    }

    #[test]
    fn 经济摘要应计算人力成本与_roi() {
        let mut s = Store::default();
        let member = 造成员(&mut s, "A", Role::Backend, 40.0);
        let task = 造任务(
            &mut s,
            "收益任务",
            Role::Backend,
            Priority::P1,
            10.0,
            "2026-12-01",
        );
        s.patch_task(
            task,
            TaskPatch {
                assignee: Some(Some(member)),
                business_value: Some(2000.0),
                ..Default::default()
            },
        );
        let e = economics(&s);
        assert_eq!(e.projected_cost, 1000.0);
        assert_eq!(e.business_value, 2000.0);
        assert_eq!(e.portfolio_roi, 1.0);
    }
}
