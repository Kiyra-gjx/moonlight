//! 经济决策层：把「谁来做、先做哪个、这周做多少」从经验判断变成可计算的经济问题。
//!
//! 三条主线：
//! 1. 成本侧——多学科协作会带来工时膨胀与沟通开销，超出产能的部分按加班溢价计价；
//! 2. 收益侧——业务收益随交付时间贴现，逾期再叠加延期损失，得到净现值 NPV；
//! 3. 决策侧——单任务用决策矩阵选净现值最优方案，多任务用 WSJF 排序、
//!    并在团队产能约束下做投资组合（资本预算）取舍。

use std::collections::HashMap;

use serde::Serialize;

use crate::date;
use crate::model::*;
use crate::store::Store;

/// 年化贴现率，代表公司内部资金的机会成本
pub const ANNUAL_DISCOUNT: f64 = 0.12;
/// 每周有效工作日
const WORK_DAYS: f64 = 5.0;
/// 跨学科承接时额外的沟通对齐工时
const COORDINATION_HOURS: f64 = 2.0;
/// 超出周产能的部分视为加班/外包，按此比例计入溢价成本
const OVERTIME_PREMIUM: f64 = 0.5;
/// 负载超过该倍数后进入疲劳区，边际成本进一步抬升
const STRAIN_THRESHOLD: f64 = 1.2;
/// 疲劳区部分追加的溢价
const STRAIN_PREMIUM: f64 = 1.5;
/// 负载超过该倍数视为不可持续，方案判定为不可行
pub const MAX_LOAD: f64 = 1.3;

/// 日贴现率
fn daily_rate() -> f64 {
    ANNUAL_DISCOUNT / 365.0
}

/// 跨学科工时膨胀系数：学科距离越远，同样的活儿要花越多工时。
/// 这是「多学科环境」在模型中的实际代价，而不只是一个标签。
pub fn discipline_factor(worker: Role, need: Role) -> f64 {
    use Role::*;
    if worker == need {
        return 1.0;
    }
    match (worker, need) {
        // 同属研发实现，上下游熟悉，膨胀最小
        (Frontend, Backend) | (Backend, Frontend) => 1.25,
        // 研发与运维共享工程基础设施
        (Frontend, Ops) | (Ops, Frontend) | (Backend, Ops) | (Ops, Backend) => 1.35,
        // 测试与研发的方法论差异较大
        (Qa, Frontend) | (Frontend, Qa) | (Qa, Backend) | (Backend, Qa) => 1.40,
        // 产品与任何技术学科之间跨度最大
        (Product, _) | (_, Product) => 1.70,
        _ => 1.50,
    }
}

/// 由某成员承担时的实际工时 = 基准工时 × 学科系数 + 跨学科沟通成本
pub fn effective_hours(task: &Task, member: &Member) -> f64 {
    let factor = discipline_factor(member.role, task.skill);
    let coord = if member.role == task.skill {
        0.0
    } else {
        COORDINATION_HOURS
    };
    task.estimate * factor + coord
}

/// 延期日损失：优先取任务上登记的值，未登记时按收益与优先级推导
pub fn delay_cost(task: &Task) -> f64 {
    if task.delay_cost_per_day > 0.0 {
        task.delay_cost_per_day
    } else {
        derive_delay_cost(task.value, task.priority)
    }
}

/// 缺省延期日损失：预期收益 × 日损失率 × 优先级权重
pub fn derive_delay_cost(value: f64, priority: Priority) -> f64 {
    value * DEFAULT_DELAY_RATE * (priority.weight() / 2.0)
}

/// WSJF = 延期成本 ÷ 任务规模，值越大越应该先做。
/// 相比人工拍定的 P0/P1/P2，它给出的是可解释的排序依据。
pub fn wsjf(task: &Task) -> f64 {
    delay_cost(task) / task.estimate.max(0.5)
}

/// 成员已排队的有效工时台账。派工是逐个决策的，
/// 每分配一个任务就要更新台账，后续决策才能看到真实的排队情况。
pub struct Ledger {
    hours: HashMap<u32, f64>,
}

impl Ledger {
    /// 基于当前已派工的未完成任务建账
    pub fn new(store: &Store) -> Self {
        let mut hours: HashMap<u32, f64> = HashMap::new();
        for t in store.tasks.iter().filter(|t| t.status.is_open()) {
            if let Some(m) = t.assignee.and_then(|id| store.member(id)) {
                *hours.entry(m.id).or_insert(0.0) += effective_hours(t, m);
            }
        }
        Ledger { hours }
    }

    pub fn queued(&self, member_id: u32) -> f64 {
        self.hours.get(&member_id).copied().unwrap_or(0.0)
    }

    pub fn push(&mut self, member_id: u32, hours: f64) {
        *self.hours.entry(member_id).or_insert(0.0) += hours;
    }
}

/// 一个候选派工方案的完整经济画像
#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub member_id: u32,
    pub name: String,
    pub role: Role,
    /// 是否跨学科承接
    pub cross_discipline: bool,
    /// 学科工时膨胀系数
    pub factor: f64,
    /// 实际投入工时
    pub effective_hours: f64,
    /// 人力成本
    pub labor_cost: f64,
    /// 超产能部分的加班溢价
    pub overload_cost: f64,
    /// 总投入
    pub cost: f64,
    /// 预计完成日
    pub finish_date: String,
    /// 相对截止日的延期天数，负数表示提前
    pub delay_days: i64,
    /// 延期损失
    pub delay_loss: f64,
    /// 贴现后的收益现值
    pub present_value: f64,
    /// 净现值 = 收益现值 - 延期损失 - 总投入
    pub npv: f64,
    /// 投资回报率
    pub roi: f64,
    /// 投资回收天数：每提前一天交付可挽回一天的延期损失
    pub payback_days: f64,
    /// 接单后的产能占用率
    pub load_after: f64,
    /// 是否在可持续产能范围内
    pub feasible: bool,
    pub note: String,
}

/// 单任务决策矩阵
#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub task_id: u32,
    pub title: String,
    pub value: f64,
    pub wsjf: f64,
    /// 候选方案，按净现值降序
    pub plans: Vec<Plan>,
    /// 净现值最优方案
    pub recommended: Option<u32>,
    /// 成本最低方案
    pub cheapest: Option<u32>,
    /// 为什么不选最便宜的那个——机会成本说明
    pub rationale: String,
}

/// 为单个任务生成全部候选方案，按净现值降序
pub fn plans(store: &Store, task: &Task, ledger: &Ledger) -> Vec<Plan> {
    let today = date::today();
    let due = date::parse(&task.due).unwrap_or(today);
    let unit_delay = delay_cost(task);

    let mut out: Vec<Plan> = store
        .members
        .iter()
        .map(|m| {
            let hours = effective_hours(task, m);
            let queued = ledger.queued(m.id);
            let labor_cost = hours * m.hourly_cost;

            // 边际成本递增：超产能部分按加班计价，进入疲劳区后再追加溢价
            let total = queued + hours;
            let over = (total - m.weekly_hours).max(0.0);
            let strain = (total - m.weekly_hours * STRAIN_THRESHOLD).max(0.0);
            let overload_cost =
                over * m.hourly_cost * OVERTIME_PREMIUM + strain * m.hourly_cost * STRAIN_PREMIUM;
            let cost = labor_cost + overload_cost;

            // 完成日按「排队工时 + 本任务工时 ÷ 日产能」推算
            let daily = (m.weekly_hours / WORK_DAYS).max(0.1);
            let finish = today + (total / daily).ceil() as i64;
            let delay_days = finish - due;
            let delay_loss = (delay_days.max(0) as f64) * unit_delay;

            // 收益随交付时间贴现，交付越晚现值越低
            let wait = (finish - today).max(0) as f64;
            let present_value = task.value / (1.0 + daily_rate()).powf(wait);
            let npv = present_value - delay_loss - cost;
            let roi = if cost > 0.0 { npv / cost } else { 0.0 };
            let payback_days = if unit_delay > 0.0 {
                cost / unit_delay
            } else {
                f64::INFINITY
            };
            let load_after = total / m.weekly_hours;
            let feasible = load_after <= MAX_LOAD;

            let note = if !feasible {
                format!(
                    "产能占用将达 {:.0}%，超出可持续上限 {:.0}%，不建议采用",
                    load_after * 100.0,
                    MAX_LOAD * 100.0
                )
            } else if m.role == task.skill {
                format!("本学科承接，产能占用 {:.0}%", load_after * 100.0)
            } else {
                format!(
                    "{}支援{}，工时 ×{:.2} 并含 {:.0}h 沟通成本",
                    m.role.label(),
                    task.skill.label(),
                    discipline_factor(m.role, task.skill),
                    COORDINATION_HOURS
                )
            };

            Plan {
                member_id: m.id,
                name: m.name.clone(),
                role: m.role,
                cross_discipline: m.role != task.skill,
                factor: discipline_factor(m.role, task.skill),
                effective_hours: hours,
                labor_cost,
                overload_cost,
                cost,
                finish_date: date::format(finish),
                delay_days,
                delay_loss,
                present_value,
                npv,
                roi,
                payback_days,
                load_after,
                feasible,
                note,
            }
        })
        .collect();

    // 可行方案一律排在不可行方案之前，其次按净现值
    out.sort_by(|a, b| b.feasible.cmp(&a.feasible).then(desc(a.npv, b.npv)));
    out
}

/// 输出完整决策矩阵，并解释推荐方案与最低成本方案的差异
pub fn decide(store: &Store, task: &Task) -> Decision {
    let ledger = Ledger::new(store);
    let plans = plans(store, task, &ledger);

    let best = plans.first().cloned();
    let cheap = plans.iter().min_by(|a, b| asc(a.cost, b.cost)).cloned();
    // 最早交付方案，完成日相同时取成本更低者
    let fast = plans
        .iter()
        .min_by(|a, b| a.delay_days.cmp(&b.delay_days).then(asc(a.cost, b.cost)))
        .cloned();

    // 无论推荐方案是否等于最低成本方案，都把「多花的钱能不能换回等值的时间」讲清楚
    let rationale = match (&best, &cheap, &fast) {
        (Some(b), Some(c), Some(f)) if b.member_id != c.member_id => format!(
            "最低成本方案是{}（总投入 ¥{:.0}，{} 交付）。改选{}多投入 ¥{:.0}，\
             可提前 {} 天交付、少承担 ¥{:.0} 延期损失，净现值高出 ¥{:.0}：\
             赶工溢价低于延期代价，这笔钱值得花。",
            c.name,
            c.cost,
            c.finish_date,
            b.name,
            b.cost - c.cost,
            (c.delay_days - b.delay_days).max(0),
            (c.delay_loss - b.delay_loss).max(0.0),
            b.npv - c.npv
        ),
        (Some(b), Some(_), Some(f)) if b.member_id != f.member_id => format!(
            "{}同时是成本最低与净现值最优方案。更快的选择是{}（可提前 {} 天交付），\
             但需多投入 ¥{:.0}，仅能挽回 ¥{:.0} 延期损失，净现值反而低 ¥{:.0}：\
             赶工溢价高于延期代价，不予采纳。",
            b.name,
            f.name,
            (b.delay_days - f.delay_days).max(0),
            (f.cost - b.cost).max(0.0),
            (b.delay_loss - f.delay_loss).max(0.0),
            b.npv - f.npv
        ),
        (Some(b), _, _) => format!(
            "{}在成本与工期上同时占优，净现值 ¥{:.0}，无需权衡。",
            b.name, b.npv
        ),
        _ => "团队暂无成员，无法生成派工方案。".to_string(),
    };

    Decision {
        task_id: task.id,
        title: task.title.clone(),
        value: task.value,
        wsjf: wsjf(task),
        recommended: best.as_ref().map(|p| p.member_id),
        cheapest: cheap.as_ref().map(|p| p.member_id),
        plans,
        rationale,
    }
}

/// 投资组合中的一条任务
#[derive(Debug, Clone, Serialize)]
pub struct PortfolioItem {
    pub task_id: u32,
    pub title: String,
    pub skill: Role,
    pub hours: f64,
    pub cost: f64,
    pub npv: f64,
    pub roi: f64,
    /// 单位工时净现值，投资组合的排序依据
    pub density: f64,
    /// 本周是否纳入
    pub selected: bool,
    pub reason: String,
}

/// 团队周产能约束下的投资组合
#[derive(Debug, Clone, Serialize)]
pub struct Portfolio {
    pub capacity_hours: f64,
    pub used_hours: f64,
    pub total_cost: f64,
    pub total_npv: f64,
    pub weighted_roi: f64,
    pub deferred_count: usize,
    /// 暂缓任务本周产生的延期损失
    pub deferred_loss: f64,
    pub items: Vec<PortfolioItem>,
}

/// 经济总览
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    /// 在手任务总投入
    pub total_cost: f64,
    /// 在手任务预期收益
    pub total_value: f64,
    pub total_npv: f64,
    pub roi: f64,
    /// 已逾期任务累计产生的沉没损失
    pub sunk_delay_loss: f64,
    /// 多学科协作带来的额外成本
    pub cross_overhead: f64,
    /// 尚未估值、无法参与经济决策的任务数
    pub unvalued: usize,
    pub portfolio: Portfolio,
}

/// 在团队总产能约束下，按单位工时净现值贪心选择本周要做的任务。
/// 这是一个资本预算问题：预算是工时，回报是净现值。
pub fn portfolio(store: &Store) -> Portfolio {
    let capacity: f64 = store.members.iter().map(|m| m.weekly_hours).sum();
    let ledger = Ledger::new(store);

    // 每个任务取其最优承接方案作为该任务的成本与收益基准
    let mut items: Vec<PortfolioItem> = store
        .tasks
        .iter()
        .filter(|t| t.status.is_open() && t.is_valued())
        .filter_map(|t| {
            let best = plans(store, t, &ledger).into_iter().next()?;
            let density = if best.effective_hours > 0.0 {
                best.npv / best.effective_hours
            } else {
                0.0
            };
            Some(PortfolioItem {
                task_id: t.id,
                title: t.title.clone(),
                skill: t.skill,
                hours: best.effective_hours,
                cost: best.cost,
                npv: best.npv,
                roi: best.roi,
                density,
                selected: false,
                reason: String::new(),
            })
        })
        .collect();

    items.sort_by(|a, b| desc(a.density, b.density));

    let mut used = 0.0;
    let mut total_cost = 0.0;
    let mut total_npv = 0.0;
    let mut deferred_count = 0;
    let mut deferred_loss = 0.0;

    for item in items.iter_mut() {
        if item.npv <= 0.0 {
            item.reason = format!("净现值为负（¥{:.0}），建议重估收益或缩减范围", item.npv);
            deferred_count += 1;
            continue;
        }
        if used + item.hours <= capacity {
            item.selected = true;
            used += item.hours;
            total_cost += item.cost;
            total_npv += item.npv;
            item.reason = format!("单位工时净现值 ¥{:.0}/h，优先投入", item.density);
        } else {
            item.reason = format!("本周产能不足（还需 {:.1}h），顺延", item.hours);
            deferred_count += 1;
            if let Some(t) = store.task(item.task_id) {
                deferred_loss += delay_cost(t) * WORK_DAYS;
            }
        }
    }

    let weighted_roi = if total_cost > 0.0 {
        total_npv / total_cost
    } else {
        0.0
    };

    Portfolio {
        capacity_hours: capacity,
        used_hours: used,
        total_cost,
        total_npv,
        weighted_roi,
        deferred_count,
        deferred_loss,
        items,
    }
}

/// 汇总全局经济指标
pub fn summary(store: &Store) -> Summary {
    let today = date::today();
    let ledger = Ledger::new(store);

    let mut total_cost = 0.0;
    let mut total_value = 0.0;
    let mut total_npv = 0.0;
    let mut sunk_delay_loss = 0.0;
    let mut cross_overhead = 0.0;
    let mut unvalued = 0;

    for t in store.tasks.iter().filter(|t| t.status.is_open()) {
        if !t.is_valued() {
            unvalued += 1;
        }
        total_value += t.value;

        // 已逾期任务的沉没损失与派工与否无关，先单独累计
        if let Some(due) = date::parse(&t.due) {
            if due < today {
                sunk_delay_loss += (today - due) as f64 * delay_cost(t);
            }
        }

        // 未派工任务按其最优方案估算，已派工任务按实际负责人估算
        let plan = match t.assignee.and_then(|id| store.member(id)) {
            Some(m) => plans(store, t, &ledger)
                .into_iter()
                .find(|p| p.member_id == m.id),
            None => plans(store, t, &ledger).into_iter().next(),
        };
        if let Some(p) = plan {
            total_cost += p.cost;
            total_npv += p.npv;
            if p.cross_discipline {
                // 与本学科承接相比多付出的那部分
                let base = t.estimate * p.cost / p.effective_hours.max(0.01);
                cross_overhead += p.cost - base;
            }
        }
    }

    let roi = if total_cost > 0.0 {
        total_npv / total_cost
    } else {
        0.0
    };

    Summary {
        total_cost,
        total_value,
        total_npv,
        roi,
        sunk_delay_loss,
        cross_overhead,
        unvalued,
        portfolio: portfolio(store),
    }
}

fn desc(a: f64, b: f64) -> std::cmp::Ordering {
    b.partial_cmp(&a).unwrap_or(std::cmp::Ordering::Equal)
}

fn asc(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 成员(s: &mut Store, name: &str, role: Role, hours: f64, cost: f64) -> u32 {
        s.add_member(NewMember {
            name: name.into(),
            role,
            weekly_hours: hours,
            hourly_cost: cost,
        })
        .id
    }

    fn 任务(s: &mut Store, title: &str, skill: Role, est: f64, due: &str, value: f64) -> u32 {
        s.add_task(NewTask {
            title: title.into(),
            skill,
            priority: Priority::P1,
            estimate: est,
            due: due.into(),
            assignee: None,
            value,
            delay_cost_per_day: None,
        })
        .id
    }

    #[test]
    fn 跨学科应当抬高工时与成本() {
        let mut s = Store::default();
        let be = 成员(&mut s, "后端", Role::Backend, 40.0, 100.0);
        let pd = 成员(&mut s, "产品", Role::Product, 40.0, 100.0);
        let t = 任务(&mut s, "接口", Role::Backend, 10.0, &date::from_today(30), 50000.0);
        let task = s.task(t).unwrap();

        let own = effective_hours(task, s.member(be).unwrap());
        let cross = effective_hours(task, s.member(pd).unwrap());
        assert_eq!(own, 10.0);
        assert!(cross > own * 1.5, "产品跨到后端应当明显更贵: {cross}");
    }

    #[test]
    fn 交付越晚净现值越低() {
        let mut s = Store::default();
        成员(&mut s, "快", Role::Backend, 40.0, 200.0);
        成员(&mut s, "慢", Role::Backend, 5.0, 200.0);
        let t = 任务(&mut s, "活", Role::Backend, 10.0, &date::from_today(3), 60000.0);
        let task = s.task(t).unwrap();
        let list = plans(&s, task, &Ledger::new(&s));

        let fast = list.iter().find(|p| p.name == "快").unwrap();
        let slow = list.iter().find(|p| p.name == "慢").unwrap();
        assert!(fast.npv > slow.npv);
        assert!(slow.delay_loss > 0.0, "慢的方案应当产生延期损失");
    }

    #[test]
    fn 延期成本高时应当接受更贵但更快的方案() {
        let mut s = Store::default();
        // 便宜但产能低，交付慢；贵但产能高，交付快
        成员(&mut s, "便宜", Role::Backend, 5.0, 80.0);
        let 贵 = 成员(&mut s, "昂贵", Role::Backend, 40.0, 300.0);
        let t = 任务(&mut s, "紧急活", Role::Backend, 10.0, &date::from_today(2), 200000.0);
        let d = decide(&s, s.task(t).unwrap());

        assert_eq!(d.recommended, Some(贵));
        assert_ne!(d.recommended, d.cheapest, "推荐方案不应等于最低成本方案");
        assert!(d.rationale.contains("延期代价"), "决策说明应当解释时间与成本的权衡");
    }

    #[test]
    fn 最优即最省时也应当解释为何不赶工() {
        let mut s = Store::default();
        // 稳：产能低、交付慢但便宜；快而贵：产能高、交付快但时薪是前者的九倍
        let 稳 = 成员(&mut s, "稳", Role::Backend, 10.0, 100.0);
        成员(&mut s, "快而贵", Role::Backend, 40.0, 900.0);
        // 延期损失很小，不值得为提前交付支付赶工溢价
        let t = 任务(&mut s, "常规活", Role::Backend, 10.0, &date::from_today(30), 8_000.0);
        let d = decide(&s, s.task(t).unwrap());

        assert_eq!(d.recommended, Some(稳));
        assert_eq!(d.recommended, d.cheapest);
        assert!(d.rationale.contains("延期代价"), "应当解释为何不选更快的方案");
    }

    #[test]
    fn wsjf_应当让高延期成本的小任务排前面() {
        let mut s = Store::default();
        let 小而急 = 任务(&mut s, "小而急", Role::Backend, 2.0, &date::from_today(5), 100000.0);
        let 大而缓 = 任务(&mut s, "大而缓", Role::Backend, 40.0, &date::from_today(5), 120000.0);
        assert!(wsjf(s.task(小而急).unwrap()) > wsjf(s.task(大而缓).unwrap()));
    }

    #[test]
    fn 不可行方案应当排在可行方案之后() {
        let mut s = Store::default();
        let 满 = 成员(&mut s, "已满", Role::Backend, 10.0, 50.0);
        成员(&mut s, "有空", Role::Backend, 40.0, 400.0);
        // 先把「已满」压到接近上限
        let 占位 = 任务(&mut s, "占位", Role::Backend, 12.0, &date::from_today(30), 50_000.0);
        s.patch_task(
            占位,
            TaskPatch {
                assignee: Some(Some(满)),
                ..Default::default()
            },
        );
        let t = 任务(&mut s, "新活", Role::Backend, 6.0, &date::from_today(30), 50_000.0);
        let list = plans(&s, s.task(t).unwrap(), &Ledger::new(&s));

        let 满方案 = list.iter().find(|p| p.member_id == 满).unwrap();
        assert!(!满方案.feasible, "已满成员的方案应当判定为不可行");
        assert_ne!(list[0].member_id, 满, "不可行方案不应排在首位");
        assert!(满方案.overload_cost > 0.0, "超产能应当产生溢价成本");
    }

    #[test]
    fn 投资组合应当受产能约束并按密度取舍() {
        let mut s = Store::default();
        成员(&mut s, "A", Role::Backend, 10.0, 100.0);
        任务(&mut s, "高回报", Role::Backend, 5.0, &date::from_today(30), 90000.0);
        任务(&mut s, "低回报", Role::Backend, 5.0, &date::from_today(30), 20000.0);
        任务(&mut s, "挤不进", Role::Backend, 5.0, &date::from_today(30), 15000.0);

        let p = portfolio(&s);
        assert_eq!(p.capacity_hours, 10.0);
        assert!(p.used_hours <= p.capacity_hours);
        assert_eq!(p.items.iter().filter(|i| i.selected).count(), 2);
        assert_eq!(p.items[0].title, "高回报", "应当按单位工时净现值排序");
        assert_eq!(p.deferred_count, 1);
    }

    #[test]
    fn 净现值为负的任务不应当纳入组合() {
        let mut s = Store::default();
        成员(&mut s, "A", Role::Backend, 40.0, 500.0);
        // 收益远低于人力投入
        任务(&mut s, "赔本活", Role::Backend, 20.0, &date::from_today(30), 1000.0);
        let p = portfolio(&s);
        assert!(!p.items[0].selected);
        assert!(p.items[0].reason.contains("净现值为负"));
    }

    #[test]
    fn 未估值任务应当被统计但不进组合() {
        let mut s = Store::default();
        成员(&mut s, "A", Role::Backend, 40.0, 100.0);
        任务(&mut s, "没估值", Role::Backend, 5.0, &date::from_today(10), 0.0);
        let sum = summary(&s);
        assert_eq!(sum.unvalued, 1);
        assert!(sum.portfolio.items.is_empty());
    }
}
