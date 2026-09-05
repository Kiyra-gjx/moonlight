//! 工程管理层：负载统计、一键派工、风险预警、团队健康度。
//! 成本与净现值相关的计算全部委托给 [`crate::economics`]。

use serde::Serialize;

use crate::date;
use crate::economics::{self, Ledger};
use crate::model::*;
use crate::store::Store;

/// 负载率超过该值判定为过载
const OVERLOAD: f64 = 1.0;
/// 负载率超过该值判定为吃紧
const BUSY: f64 = 0.85;
/// 临期预警的天数窗口
const DUE_SOON_DAYS: i64 = 3;

/// 单个成员的负载画像。工时按跨学科折算后的实际投入统计，
/// 因此一个前端去接后端的活会如实占用更多产能。
#[derive(Debug, Clone, Serialize)]
pub struct Workload {
    pub member_id: u32,
    pub name: String,
    pub role: Role,
    /// 周产能工时
    pub capacity: f64,
    /// 未完成任务的实际投入工时
    pub assigned: f64,
    /// 负载率 = assigned / capacity
    pub ratio: f64,
    /// 未完成任务数
    pub open_tasks: usize,
    /// 人力成本，元/小时
    pub hourly_cost: f64,
    /// 在手任务的人力成本合计
    pub committed_cost: f64,
    /// 名下最紧急的截止日期
    pub nearest_due: Option<String>,
}

/// 预警条目
#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    /// danger / warn / info
    pub level: &'static str,
    pub text: String,
}

/// 团队健康度（工程管理维度）
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
            let assigned = mine
                .iter()
                .fold(0.0, |acc, t| acc + economics::effective_hours(t, m));
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
                hourly_cost: m.hourly_cost,
                committed_cost: assigned * m.hourly_cost,
                nearest_due,
            }
        })
        .collect()
}

/// 一键派工：按 WSJF 经济优先级排序，逐个选择净现值最优的可行方案。
///
/// 负载不均带来的加班溢价已内化进方案成本，人力上限由可持续产能上限兜底，
/// 因此不需要额外的均衡约束。
pub fn auto_assign(store: &mut Store) -> Vec<(u32, u32)> {
    if store.members.is_empty() {
        return Vec::new();
    }
    let mut ledger = Ledger::new(store);

    let mut pending: Vec<Task> = store
        .tasks
        .iter()
        .filter(|t| t.assignee.is_none() && t.status.is_open() && t.is_valued())
        .cloned()
        .collect();
    pending.sort_by(|a, b| {
        economics::wsjf(b)
            .partial_cmp(&economics::wsjf(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                date::parse(&a.due)
                    .unwrap_or(i64::MAX)
                    .cmp(&date::parse(&b.due).unwrap_or(i64::MAX)),
            )
    });

    let mut result = Vec::new();
    for task in pending {
        // 只在可持续产能范围内派工；全员都排不下时留空，交由增援或砍需求处理
        let Some(best) = economics::plans(store, &task, &ledger)
            .into_iter()
            .find(|p| p.feasible && p.npv > 0.0)
        else {
            continue;
        };
        // 台账立即入账，后续任务才能看到真实的排队与产能占用
        ledger.push(best.member_id, best.effective_hours);
        if let Some(t) = store.tasks.iter_mut().find(|t| t.id == task.id) {
            t.assignee = Some(best.member_id);
        }
        result.push((task.id, best.member_id));
    }
    result
}

/// 汇总风险预警，按严重程度排序。
/// 进度与资源类风险直接给出金额，便于判断值不值得干预。
pub fn alerts(store: &Store, loads: &[Workload]) -> Vec<Alert> {
    let today = date::today();
    let ledger = Ledger::new(store);
    let mut out = Vec::new();

    for t in store.tasks.iter().filter(|t| t.status.is_open()) {
        let owner = t
            .assignee
            .and_then(|id| store.member(id))
            .map(|m| m.name.as_str())
            .unwrap_or("待派工");

        match date::parse(&t.due) {
            Some(due) if due < today => {
                let days = today - due;
                out.push(Alert {
                    level: "danger",
                    text: format!(
                        "《{}》已逾期 {} 天，累计延期损失 ¥{:.0}（{}）",
                        t.title,
                        days,
                        days as f64 * economics::delay_cost(t),
                        owner
                    ),
                });
            }
            Some(due) if due - today <= DUE_SOON_DAYS && t.status == Status::Todo => {
                out.push(Alert {
                    level: "warn",
                    text: format!(
                        "《{}》还剩 {} 天到期但尚未开工，延期日损失 ¥{:.0}（{}）",
                        t.title,
                        due - today,
                        economics::delay_cost(t),
                        owner
                    ),
                });
            }
            _ => {}
        }

        if t.assignee.is_none() && t.priority == Priority::P0 {
            out.push(Alert {
                level: "danger",
                text: format!("P0 任务《{}》仍未派工", t.title),
            });
        }

        if !t.is_valued() {
            out.push(Alert {
                level: "info",
                text: format!("《{}》尚未估算业务收益，暂不参与经济决策", t.title),
            });
        }

        // 已派工任务的经济体检：跨学科溢价与投入产出倒挂
        if let Some(m) = t.assignee.and_then(|id| store.member(id)) {
            let plan = economics::plans(store, t, &ledger)
                .into_iter()
                .find(|p| p.member_id == m.id);
            if let Some(p) = plan {
                if p.cross_discipline {
                    let base = t.estimate * m.hourly_cost;
                    out.push(Alert {
                        level: "info",
                        text: format!(
                            "《{}》需要{}能力，由{}（{}）跨学科承接，成本上浮 ¥{:.0}（+{:.0}%）",
                            t.title,
                            t.skill.label(),
                            m.name,
                            m.role.label(),
                            p.labor_cost - base,
                            (p.labor_cost / base - 1.0) * 100.0
                        ),
                    });
                }
                if t.is_valued() && p.npv < 0.0 {
                    out.push(Alert {
                        level: "warn",
                        text: format!(
                            "《{}》当前方案净现值 -¥{:.0}，投入产出倒挂，建议缩减范围或重估收益",
                            t.title, -p.npv
                        ),
                    });
                }
            }
        }
    }

    // 产能缺口：在手任务的最省投入合计与团队周产能对比
    let capacity: f64 = loads.iter().map(|w| w.capacity).sum();
    let demand: f64 = store
        .tasks
        .iter()
        .filter(|t| t.status.is_open())
        .filter_map(|t| match t.assignee.and_then(|id| store.member(id)) {
            Some(m) => Some(economics::effective_hours(t, m)),
            // 未派工任务按最省工时的承接方式估算，使缺口结论保持保守
            None => store
                .members
                .iter()
                .map(|m| economics::effective_hours(t, m))
                .fold(None, |acc: Option<f64>, h| {
                    Some(acc.map_or(h, |a| a.min(h)))
                }),
        })
        .sum();
    if capacity > 0.0 && demand > capacity {
        let gap = demand - capacity;
        let per_head = capacity / loads.len() as f64;
        out.push(Alert {
            level: "danger",
            text: format!(
                "在手任务需 {:.0}h，团队周产能 {:.0}h，缺口 {:.0}h（约 {:.1} 人周），\
                 需增援、延期或按投资组合砍需求",
                demand,
                capacity,
                gap,
                gap / per_head.max(0.1)
            ),
        });
    }

    let has_pending = store
        .tasks
        .iter()
        .any(|t| t.assignee.is_none() && t.status.is_open());
    for w in loads {
        if w.ratio > OVERLOAD {
            let over = w.assigned - w.capacity;
            out.push(Alert {
                level: "danger",
                text: format!(
                    "{} 负载 {:.0}%（{:.1}h / {:.1}h），超出部分按加班计价约 ¥{:.0}",
                    w.name,
                    w.ratio * 100.0,
                    w.assigned,
                    w.capacity,
                    over * w.hourly_cost * 1.5
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
                text: format!(
                    "{} 当前无在手任务，闲置产能 {:.0}h 可承接待派工需求",
                    w.name, w.capacity
                ),
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

    let mut score = ((0.40 * on_time + 0.35 * balance + 0.25 * coverage) * 100.0).round() as u32;
    // 均衡并不等于健康：不能让所有人一起过载反而获得高分。
    let max_load = loads.iter().map(|w| w.ratio).fold(0.0, f64::max);
    let comment = if max_load > economics::MAX_LOAD {
        score = score.min(59);
        "已超过可持续产能上限，需要立即调整排期或增援"
    } else if max_load > OVERLOAD {
        score = score.min(74);
        "存在成员过载，需减少在手任务或重新派工"
    } else {
        match score {
            90..=100 => "节奏稳健，保持当前排期",
            75..=89 => "整体可控，关注个别预警",
            60..=74 => "存在明显瓶颈，建议重新派工",
            _ => "风险偏高，需要立即介入调整",
        }
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
            due: due.into(),
            assignee: None,
            value: 50_000.0,
            delay_cost_per_day: None,
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
    fn 跨学科承接应当占用更多产能() {
        let mut s = Store::default();
        let fe = 造成员(&mut s, "前端", Role::Frontend, 40.0);
        let t = 造任务(
            &mut s,
            "后端活",
            Role::Backend,
            Priority::P1,
            10.0,
            "2026-12-01",
        );
        s.patch_task(
            t,
            TaskPatch {
                assignee: Some(Some(fe)),
                ..Default::default()
            },
        );
        let w = &workloads(&s)[0];
        assert!(
            w.assigned > 10.0,
            "跨学科折算后应当大于基准工时: {}",
            w.assigned
        );
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
    fn 自动派工应当优先技能匹配的成员() {
        let mut s = Store::default();
        造成员(&mut s, "前端", Role::Frontend, 40.0);
        let be = 造成员(&mut s, "后端", Role::Backend, 40.0);
        let t = 造任务(
            &mut s,
            "接口开发",
            Role::Backend,
            Priority::P1,
            8.0,
            "2026-12-01",
        );
        auto_assign(&mut s);
        assert_eq!(s.task(t).unwrap().assignee, Some(be));
    }

    #[test]
    fn 自动派工不应把成员压过可持续上限() {
        let mut s = Store::default();
        造成员(&mut s, "独苗", Role::Backend, 10.0);
        for i in 0..6 {
            造任务(
                &mut s,
                &format!("T{i}"),
                Role::Backend,
                Priority::P1,
                5.0,
                "2026-12-01",
            );
        }
        auto_assign(&mut s);

        let w = &workloads(&s)[0];
        assert!(
            w.ratio <= crate::economics::MAX_LOAD,
            "负载 {:.2} 超出可持续上限",
            w.ratio
        );
        assert!(
            s.tasks.iter().any(|t| t.assignee.is_none()),
            "排不下的任务应当保持未派工"
        );
    }

    #[test]
    fn 需求超过产能时应当报出缺口() {
        let mut s = Store::default();
        造成员(&mut s, "独苗", Role::Backend, 10.0);
        for i in 0..5 {
            造任务(
                &mut s,
                &format!("T{i}"),
                Role::Backend,
                Priority::P1,
                5.0,
                "2026-12-01",
            );
        }
        let list = alerts(&s, &workloads(&s));
        assert!(list.iter().any(|a| a.text.contains("缺口")));
    }

    #[test]
    fn 逾期预警应当给出金额() {
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
        let overdue = list.iter().find(|a| a.text.contains("已逾期")).unwrap();
        assert!(overdue.text.contains("延期损失 ¥"));
        assert!(list.iter().any(|a| a.text.contains("仍未派工")));
    }

    #[test]
    fn 空团队健康度不应当为零() {
        let s = Store::default();
        assert_eq!(health(&s, &[]).score, 100);
    }

    #[test]
    fn 全员均匀过载也不能判为健康() {
        for (hours, cap) in [(12.0, 74), (14.0, 59)] {
            let mut s = Store::default();
            for name in ["甲", "乙"] {
                let id = 造成员(&mut s, name, Role::Backend, 10.0);
                let t = 造任务(
                    &mut s,
                    "在手任务",
                    Role::Backend,
                    Priority::P1,
                    hours,
                    &date::from_today(30),
                );
                s.patch_task(
                    t,
                    TaskPatch {
                        assignee: Some(Some(id)),
                        ..Default::default()
                    },
                );
            }
            let h = health(&s, &workloads(&s));
            assert_eq!(h.balance, 1.0);
            assert_eq!(h.score, cap);
            assert!(!h.comment.contains("稳健"));
        }
    }

    #[test]
    fn 自动派工应保留未估值和负净现值任务() {
        let mut s = Store::default();
        造成员(&mut s, "甲", Role::Backend, 40.0);
        for value in [0.0, 1.0] {
            s.add_task(NewTask {
                title: "待评估".into(),
                skill: Role::Backend,
                priority: Priority::P1,
                estimate: 8.0,
                due: date::from_today(30),
                assignee: None,
                value,
                delay_cost_per_day: None,
            });
        }
        assert!(auto_assign(&mut s).is_empty());
        assert!(s.tasks.iter().all(|t| t.assignee.is_none()));
    }
}
