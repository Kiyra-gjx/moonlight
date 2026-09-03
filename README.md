# Moonlight

企业研发团队内部管理工具。核心逻辑用 Rust 实现，交互界面为内嵌的单页 Web 控制台，
编译产物是一个不依赖外部资源的独立二进制。

## 能力

- **任务看板**：待办 / 进行中 / 待评审 / 已完成四列，拖拽卡片即可流转状态。
- **成员负载**：按未完成任务的预估工时实时计算每个人的负载率，超载与吃紧分级标色。
- **智能派工**：综合「技能匹配度」与「接单后的负载率」给候选人打分；
  一键派工会按优先级和截止日贪心分配，并在每次分配后刷新负载快照，避免任务堆到同一个人身上。
- **风险预警**：逾期、临期未开工、P0 未派工、成员过载、跨技能承接、人员闲置。
- **团队健康度**：按时率 40% + 负载均衡 35% + 派工覆盖 25%，给出 0~100 分与处置建议。

## 运行

```bash
cargo run --release
# 默认监听 http://127.0.0.1:8088，数据落在 data/moonlight.json
```

可选参数：

```bash
cargo run --release -- --port 9000 --data /path/to/team.json
```

首次启动若数据文件不存在，会自动写入一份演示数据。

## 测试

```bash
cargo test
```

## 结构

| 文件 | 职责 |
| --- | --- |
| `src/model.rs` | 领域模型：成员、任务、角色、优先级、状态 |
| `src/store.rs` | 内存数据 + JSON 落盘 |
| `src/insight.rs` | 负载统计、派工评分、风险预警、健康度 |
| `src/http.rs` | 极简 HTTP/1.1 服务 |
| `src/api.rs` | 路由与参数校验 |
| `src/web/index.html` | 内嵌前端页面 |

## 接口

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/api/state` | 成员、任务、负载、预警、健康度全量快照 |
| POST | `/api/members` | 新增成员 |
| DELETE | `/api/members/{id}` | 移除成员，其名下任务退回未派工 |
| POST | `/api/tasks` | 新建任务 |
| PATCH | `/api/tasks/{id}` | 局部更新，`assignee: null` 表示取消派工 |
| DELETE | `/api/tasks/{id}` | 删除任务 |
| GET | `/api/tasks/{id}/suggest` | 该任务的候选人排名与理由 |
| POST | `/api/auto-assign` | 一键派工 |
