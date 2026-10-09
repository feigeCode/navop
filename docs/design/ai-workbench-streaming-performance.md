# AI Workbench 流式性能预算

适用范围：`crates/ai_chat_view` 的 ACP 事件链路（agent 子进程 → 连接层 → broadcast → 视图转录）。
参照：waku `docs/performance.md`（GPUI 同构客户端的流式纪律）。

## 预算

| 环节 | 预算 | 依据 |
|---|---|---|
| 事件批处理延迟（`apply_runtime_events` 单批） | ≤ 16ms | 一帧 60fps 的全部时间；超了就掉帧 |
| broadcast 通道容量 | 4096（`broadcast::channel(4096)`） | 512 在长工具调用回放时会 Lagged；见 `acp_replay_retries` |
| Lagged 自愈重载次数 | ≤ 1 次（成功归零） | 防慢 agent 无限重载；重载本身走 `on_runtime_events_dropped` |
| 每事件转录落盘 | 不允许在事件批内同步 IO | 落盘走 `persist_acp_session` 快照点，不逐事件 |
| 会话列表查询 | 摘要行不含 `snapshot_json` | 实测 356 行合计 76.8MB；见 `list_summaries_by_archived` 注释 |

## 已固化的机制（不要退回去）

1. **事件泵批量收**：`spawn_event_pump` 用 `collect_ready_runtime_events` 把就绪事件合成一批，一次 `update` 处理——避免每个 session/update 一次 GPUI 实体锁。
2. **批内先落地、批后重同步**：丢事件（Lagged）时先处理本批再 `on_runtime_events_dropped`，终态误判窗口最小。
3. **大快照不进内存**：侧栏只查摘要列，19MB 单条快照不随 `list_summaries` 加载。
4. **回放窗口**：`acp_history_replay` 有界，连接收掉即清（`reset_acp_client_session`）。

## 压测方法（本机基准）

```bash
# fake agent 以满速率推 session/update（agent_message_chunk）
cargo test -p ai_chat_view --test acp_connection -- --nocapture
# 观测：测试日志无 Lagged 告警；视图批处理耗时打点（tracing target=ai_chat_view::agent_view）
```

基准红线：10k chunks/轮次，零 Lagged、零丢失终态（`acp_replay_retries` 不触发第二次）。

## 改动前先看

- 动 `broadcast` 容量或批处理结构 → 重跑 `acp_connection.rs` 全量 + 本地 10k chunk 冒烟。
- 动 `persist_acp_session` / `list_summaries` → 用真实数据库（含 300+ 会话）确认查询不回落全快照。
