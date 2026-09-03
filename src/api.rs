//! HTTP 路由：把请求翻译成对 Store / insight 的调用

use std::sync::Mutex;

use serde::Serialize;
use serde_json::json;

use crate::economics;
use crate::http::{Request, Response};
use crate::insight;
use crate::model::*;
use crate::store::Store;

/// 前端一次拉取所需的全部数据
#[derive(Serialize)]
struct State<'a> {
    members: &'a [Member],
    tasks: &'a [Task],
    workloads: Vec<insight::Workload>,
    alerts: Vec<insight::Alert>,
    health: insight::Health,
    economics: economics::Summary,
    today: String,
}

/// 内嵌前端页面，编译后单个二进制即可运行
const INDEX_HTML: &str = include_str!("web/index.html");

pub fn route(store: &Mutex<Store>, req: Request) -> Response {
    let seg = req.segments();
    let m = req.method.as_str();

    match (m, seg.as_slice()) {
        ("GET", []) => Response::html(INDEX_HTML),

        ("GET", ["api", "state"]) => {
            let s = store.lock().unwrap();
            let workloads = insight::workloads(&s);
            let payload = State {
                members: &s.members,
                tasks: &s.tasks,
                alerts: insight::alerts(&s, &workloads),
                health: insight::health(&s, &workloads),
                economics: economics::summary(&s),
                workloads,
                today: crate::date::from_today(0),
            };
            ok(&payload)
        }

        ("POST", ["api", "members"]) => match serde_json::from_str::<NewMember>(&req.body) {
            Ok(input) if !input.name.trim().is_empty() => {
                let created = store.lock().unwrap().add_member(input);
                created_json(&created)
            }
            Ok(_) => bad("成员姓名不能为空"),
            Err(e) => bad(&format!("参数解析失败: {e}")),
        },

        ("DELETE", ["api", "members", id]) => match id.parse::<u32>() {
            Ok(id) if store.lock().unwrap().remove_member(id) => ok(&json!({ "removed": id })),
            Ok(_) => not_found("成员不存在"),
            Err(_) => bad("成员 id 非法"),
        },

        ("POST", ["api", "tasks"]) => match serde_json::from_str::<NewTask>(&req.body) {
            Ok(input) if input.title.trim().is_empty() => bad("任务标题不能为空"),
            Ok(input) if crate::date::parse(&input.due).is_none() => bad("截止日期格式应为 YYYY-MM-DD"),
            Ok(input) => {
                let created = store.lock().unwrap().add_task(input);
                created_json(&created)
            }
            Err(e) => bad(&format!("参数解析失败: {e}")),
        },

        ("PATCH", ["api", "tasks", id]) => {
            let Ok(id) = id.parse::<u32>() else {
                return bad("任务 id 非法");
            };
            match serde_json::from_str::<TaskPatch>(&req.body) {
                Ok(patch) => match store.lock().unwrap().patch_task(id, patch) {
                    Some(t) => ok(&t),
                    None => not_found("任务不存在"),
                },
                Err(e) => bad(&format!("参数解析失败: {e}")),
            }
        },

        ("DELETE", ["api", "tasks", id]) => match id.parse::<u32>() {
            Ok(id) if store.lock().unwrap().remove_task(id) => ok(&json!({ "removed": id })),
            Ok(_) => not_found("任务不存在"),
            Err(_) => bad("任务 id 非法"),
        },

        // 单任务决策矩阵：各候选方案的成本、工期、净现值、投资回报率
        ("GET", ["api", "tasks", id, "decision"]) => {
            let Ok(id) = id.parse::<u32>() else {
                return bad("任务 id 非法");
            };
            let s = store.lock().unwrap();
            match s.task(id) {
                Some(t) => ok(&economics::decide(&s, t)),
                None => not_found("任务不存在"),
            }
        }

        ("POST", ["api", "auto-assign"]) => {
            let pairs = insight::auto_assign(&mut store.lock().unwrap());
            let items: Vec<_> = pairs
                .into_iter()
                .map(|(task, member)| json!({ "task_id": task, "member_id": member }))
                .collect();
            ok(&json!({ "assigned": items.len(), "items": items }))
        }

        _ => not_found("接口不存在"),
    }
}

fn ok<T: Serialize>(v: &T) -> Response {
    match serde_json::to_string(v) {
        Ok(body) => Response::json(200, body),
        Err(e) => Response::text(500, &format!("序列化失败: {e}")),
    }
}

fn created_json<T: Serialize>(v: &T) -> Response {
    match serde_json::to_string(v) {
        Ok(body) => Response::json(201, body),
        Err(e) => Response::text(500, &format!("序列化失败: {e}")),
    }
}

fn bad(msg: &str) -> Response {
    Response::json(400, json!({ "error": msg }).to_string())
}

fn not_found(msg: &str) -> Response {
    Response::json(404, json!({ "error": msg }).to_string())
}
