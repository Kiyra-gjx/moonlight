//! 领域模型：研发成员与研发任务

use serde::{Deserialize, Serialize};

/// 研发角色，同时用作任务所需的技能标签
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Frontend,
    Backend,
    Qa,
    Product,
    Ops,
}

impl Role {
    /// 中文名，用于预警文案
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

/// 任务优先级，P0 最高
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    P0,
    P1,
    P2,
}

impl Priority {
    /// 排序与算法中使用的权重，越高越紧急
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
    pub role: Role,
    /// 每周可投入研发的工时（容量）
    pub weekly_hours: f64,
}

/// 研发任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: u32,
    pub title: String,
    /// 完成该任务需要的技能
    pub skill: Role,
    pub priority: Priority,
    pub status: Status,
    /// 负责人，None 表示尚未派工
    pub assignee: Option<u32>,
    /// 预估工时
    pub estimate: f64,
    /// 截止日期 `YYYY-MM-DD`
    pub due: String,
}

/// 新建成员的入参
#[derive(Debug, Deserialize)]
pub struct NewMember {
    pub name: String,
    pub role: Role,
    pub weekly_hours: f64,
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
