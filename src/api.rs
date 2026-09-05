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

/// 写请求在副本中执行，落盘成功后再提交内存状态。
pub fn route(store: &Mutex<Store>, req: Request) -> Response {
    if matches!(req.method.as_str(), "POST" | "PATCH" | "DELETE") {
        let mut current = store.lock().unwrap();
        let staged = Mutex::new(current.clone());
        let response = route_inner(&staged, req);
        if response.status < 400 {
            let candidate = staged.into_inner().unwrap();
            if let Err(error) = candidate.save() {
                eprintln!("[moonlight] {error}");
                return Response::json(
                    500,
                    json!({"error":"保存失败，修改未生效，请检查数据目录后重试"}).to_string(),
                );
            }
            *current = candidate;
        }
        response
    } else {
        route_inner(store, req)
    }
}

fn route_inner(store: &Mutex<Store>, req: Request) -> Response {
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
            Ok(input) if input.name.trim().is_empty() => bad("成员姓名不能为空"),
            Ok(input) if input.name.chars().count() > 40 => bad("成员姓名不能超过 40 个字符"),
            Ok(input) if !positive(input.weekly_hours) => bad("周产能必须是大于 0 的数字"),
            Ok(input) if !positive(input.hourly_cost) => bad("小时成本必须是大于 0 的数字"),
            Ok(mut input) => {
                input.name = input.name.trim().to_string();
                let created = store.lock().unwrap().add_member(input);
                created_json(&created)
            }
            Err(e) => bad(&format!("参数解析失败: {e}")),
        },

        ("DELETE", ["api", "members", id]) => match id.parse::<u32>() {
            Ok(id) if store.lock().unwrap().remove_member(id) => ok(&json!({ "removed": id })),
            Ok(_) => not_found("成员不存在"),
            Err(_) => bad("成员 id 非法"),
        },

        ("POST", ["api", "tasks"]) => match serde_json::from_str::<NewTask>(&req.body) {
            Ok(input) if input.title.trim().is_empty() => bad("任务标题不能为空"),
            Ok(input) if input.title.chars().count() > 120 => bad("任务标题不能超过 120 个字符"),
            Ok(input) if crate::date::parse(&input.due).is_none() => {
                bad("截止日期格式应为 YYYY-MM-DD")
            }
            Ok(input) if !positive(input.estimate) => bad("预估工时必须是大于 0 的数字"),
            Ok(input) if !non_negative(input.value) => bad("预期收益必须是非负数"),
            Ok(input)
                if input
                    .delay_cost_per_day
                    .is_some_and(|value| !non_negative(value)) =>
            {
                bad("延期日损失必须是非负数")
            }
            Ok(mut input) => {
                let mut s = store.lock().unwrap();
                if input.assignee.is_some_and(|id| s.member(id).is_none()) {
                    return bad("负责人不存在");
                }
                input.title = input.title.trim().to_string();
                let created = s.add_task(input);
                created_json(&created)
            }
            Err(e) => bad(&format!("参数解析失败: {e}")),
        },

        ("PATCH", ["api", "tasks", id]) => {
            let Ok(id) = id.parse::<u32>() else {
                return bad("任务 id 非法");
            };
            match serde_json::from_str::<TaskPatch>(&req.body) {
                Ok(patch) => {
                    if patch.title.as_ref().is_some_and(|v| v.trim().is_empty()) {
                        return bad("任务标题不能为空");
                    }
                    if patch
                        .title
                        .as_ref()
                        .is_some_and(|v| v.chars().count() > 120)
                    {
                        return bad("任务标题不能超过 120 个字符");
                    }
                    if patch
                        .due
                        .as_ref()
                        .is_some_and(|v| crate::date::parse(v).is_none())
                    {
                        return bad("截止日期格式应为 YYYY-MM-DD");
                    }
                    if patch.estimate.is_some_and(|v| !positive(v)) {
                        return bad("预估工时必须是大于 0 的数字");
                    }
                    if patch.value.is_some_and(|v| !non_negative(v)) {
                        return bad("预期收益必须是非负数");
                    }
                    if patch.delay_cost_per_day.is_some_and(|v| !non_negative(v)) {
                        return bad("延期日损失必须是非负数");
                    }
                    let mut s = store.lock().unwrap();
                    if patch
                        .assignee
                        .flatten()
                        .is_some_and(|member_id| s.member(member_id).is_none())
                    {
                        return bad("负责人不存在");
                    }
                    match s.patch_task(id, patch) {
                        Some(t) => ok(&t),
                        None => not_found("任务不存在"),
                    }
                }
                Err(e) => bad(&format!("参数解析失败: {e}")),
            }
        }

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

fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
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

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, path: &str, body: &str) -> Request {
        Request {
            method: method.to_string(),
            path: path.to_string(),
            body: body.to_string(),
        }
    }

    #[test]
    fn 保存失败返回错误且不修改内存恢复后可以持久化() {
        let dir =
            std::env::temp_dir().join(format!("moonlight-transaction-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        std::fs::write(&path, r#"{"members":[],"tasks":[]}"#).unwrap();
        let store = Mutex::new(Store::load(&path).unwrap());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
        std::fs::write(&dir, "阻止创建目录").unwrap();
        let body = r#"{"name":"A","role":"backend","weekly_hours":40,"hourly_cost":100}"#;
        assert_eq!(
            route(&store, request("POST", "/api/members", body)).status,
            500
        );
        assert!(store.lock().unwrap().members.is_empty());
        std::fs::remove_file(&dir).unwrap();
        assert_eq!(
            route(&store, request("POST", "/api/members", body)).status,
            201
        );
        let saved = Store::load(&path).unwrap();
        assert_eq!(saved.members.len(), 1);
        assert_eq!(saved.members[0].id, 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 创建任务应拒绝不存在的真实日期() {
        let store = Mutex::new(Store::default());
        let response = route(
            &store,
            request(
                "POST",
                "/api/tasks",
                r#"{"title":"T","skill":"backend","priority":"p1","estimate":4,"value":1000,"due":"2026-02-31"}"#,
            ),
        );
        assert_eq!(response.status, 400);
        assert!(store.lock().unwrap().tasks.is_empty());
    }

    #[test]
    fn 成员成本与工时应为正数() {
        let store = Mutex::new(Store::default());
        let response = route(
            &store,
            request(
                "POST",
                "/api/members",
                r#"{"name":"A","role":"backend","weekly_hours":0,"hourly_cost":-1}"#,
            ),
        );
        assert_eq!(response.status, 400);
        assert!(store.lock().unwrap().members.is_empty());
    }

    #[test]
    fn 更新任务应拒绝空标题和非法负责人() {
        let mut raw = Store::default();
        let task = raw.add_task(NewTask {
            title: "任务".into(),
            skill: Role::Backend,
            priority: Priority::P1,
            estimate: 4.0,
            due: "2026-12-01".into(),
            assignee: None,
            value: 1_000.0,
            delay_cost_per_day: None,
        });
        let store = Mutex::new(raw);

        let empty = route(
            &store,
            request(
                "PATCH",
                &format!("/api/tasks/{}", task.id),
                r#"{"title":"  "}"#,
            ),
        );
        let missing = route(
            &store,
            request(
                "PATCH",
                &format!("/api/tasks/{}", task.id),
                r#"{"assignee":999}"#,
            ),
        );

        assert_eq!(empty.status, 400);
        assert_eq!(missing.status, 400);
        assert_eq!(store.lock().unwrap().task(task.id).unwrap().title, "任务");
    }
}
