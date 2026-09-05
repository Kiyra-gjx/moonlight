//! 领域模型：研发成员与研发任务。
//! 任务同时带工时、截止日与收益、延期损失，分别供两层逻辑使用。

use serde::{Deserialize, Serialize};

/// 默认人力成本（元/小时）
fn default_hourly_cost() -> f64 {
    120.0
}

/// 延期日损失占预期收益的默认比例：每延一天损失 2%
pub const DEFAULT_DELAY_RATE: f64 = 0.02;

/// 研发角色，既是成员的专业学科，也是任务所需的学科能力
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Frontend,
    Backend,
    Qa,
    Product,
    Ops,
}

impl Role {
    /// 中文名，用于预警与决策说明
    pub fn label(&self) -> &'static str {
        match self {
            Role::Frontend => "前端",
            Role::Backend => "后端",
            Role::Qa => "测试",
            Role::Product => "产品",
            Role::Ops => "运维",
        }
    }
}

/// 任务优先级，P0 最高。表达业务主观诉求，排期顺序另由 WSJF 决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    P0,
    P1,
    P2,
}

impl Priority {
    /// 紧迫度权重，参与延期成本估算
    pub fn weight(&self) -> f64 {
        match self {
            Priority::P0 => 3.0,
            Priority::P1 => 2.0,
            Priority::P2 => 1.0,
        }
    }
}

/// 任务状态，对应看板的四列
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// 待办
    Todo,
    /// 进行中
    Doing,
    /// 待评审
    Review,
    /// 已完成
    Done,
}

impl Status {
    /// 是否仍占用成员工时
    pub fn is_open(&self) -> bool {
        !matches!(self, Status::Done)
    }
}

/// 团队成员
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: u32,
    pub name: String,
    /// 专业学科
    pub role: Role,
    /// 每周可投入研发的工时（产能）
    pub weekly_hours: f64,
    /// 综合人力成本，元/小时
    #[serde(default = "default_hourly_cost")]
    pub hourly_cost: f64,
}

/// 研发任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: u32,
    pub title: String,
    /// 完成该任务需要的学科能力
    pub skill: Role,
    pub priority: Priority,
    pub status: Status,
    /// 负责人，None 表示尚未派工
    pub assignee: Option<u32>,
    /// 预估工时（由本学科成员承担时的基准工时）
    pub estimate: f64,
    /// 截止日期 `YYYY-MM-DD`
    pub due: String,
    /// 预期业务收益，元。为 0 表示尚未估值，不参与经济决策
    #[serde(default, alias = "business_value")]
    pub value: f64,
    /// 每延期一天造成的损失，元/天
    #[serde(default = "missing_delay_cost")]
    pub delay_cost_per_day: f64,
}

impl Task {
    /// 是否已完成收益估值，未估值的任务不纳入投资组合
    pub fn is_valued(&self) -> bool {
        self.value > 0.0
    }
}

/// 新建成员的入参
#[derive(Debug, Deserialize)]
pub struct NewMember {
    pub name: String,
    pub role: Role,
    pub weekly_hours: f64,
    #[serde(default = "default_hourly_cost")]
    pub hourly_cost: f64,
}

/// 新建任务的入参
#[derive(Debug, Deserialize)]
pub struct NewTask {
    pub title: String,
    pub skill: Role,
    pub priority: Priority,
    pub estimate: f64,
    pub due: String,
    #[serde(default)]
    pub assignee: Option<u32>,
    #[serde(default, alias = "business_value")]
    pub value: f64,
    /// 缺省时按「预期收益 × 日损失率 × 优先级权重」自动推导
    #[serde(default)]
    pub delay_cost_per_day: Option<f64>,
}

/// 更新任务的入参，字段缺省表示不修改
#[derive(Debug, Default, Deserialize)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub skill: Option<Role>,
    pub priority: Option<Priority>,
    pub status: Option<Status>,
    pub estimate: Option<f64>,
    pub due: Option<String>,
    #[serde(alias = "business_value")]
    pub value: Option<f64>,
    pub delay_cost_per_day: Option<f64>,
    /// 双层 Option：外层缺省表示不改，内层 None 表示取消派工
    #[serde(default, deserialize_with = "double_option")]
    pub assignee: Option<Option<u32>>,
}

/// 区分「字段未出现」与「字段显式为 null」
fn double_option<'de, D>(de: D) -> Result<Option<Option<u32>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<u32>::deserialize(de).map(Some)
}

// 旧文件缺省字段仍按收益推导；显式零值表示没有延期损失。
fn missing_delay_cost() -> f64 {
    -1.0
}
