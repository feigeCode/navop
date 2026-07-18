# Generic Native IPC and Redis/MongoDB Driver Split Implementation Plan

> Steps use checkbox (`- [ ]`) syntax for tracking. Update this document whenever scope, dependencies, decisions, commands, or status change.

**Goal:** 抽取可供后续 native sidecar 复用的通用 IPC 层；让 Redis/MongoDB 默认通过独立 IPC 驱动工作；保留现有内置实现，并由默认关闭的 Cargo feature 控制。

**Architecture:** `extension-protocol` 提供通用 wire contract，`extension-host::ProcessRpcSession` 统一拥有进程和 JSON-RPC 生命周期，`extension-driver::serve_async` 支持 Tokio-first sidecar。SQL IPC 作为现有消费者迁移到通用 session；Redis/MongoDB 使用独立 runtime facade、IPC backend 和 sidecar。UI 始终编译，feature 只控制内置 SDK backend。

**Tech Stack:** Rust 2024、Tokio、JSON-RPC 2.0、本地 socket、GPUI、Cargo features、serde/BSON、现有 extension marketplace。

**Worktree:** `/Users/hufei/RustroverProjects/navop/.worktrees/native-ipc-drivers`

**Branch:** `feat/native-ipc-drivers`

**Baseline:** `51936bd8806bfe92e79bd87c1f252b1aa2ebe591`

---

## Execution Status

- [x] 创建独立 worktree 和分支，确认基线干净且未带入主工作区未提交改动。
- [x] 写入设计规格和可执行计划。
- [x] 完成 Task 1 基线与结构 contract（最终 feature contract 保持 Red，待 Task 11 转 Green）。
- [x] 完成通用 IPC 层。
- [x] 完成 Redis IPC/default feature split（TLS/Cluster/Sentinel 和真实 standalone 矩阵仍为后续增强项）。
- [x] 完成 MongoDB IPC/default feature split（真实旧服务端兼容矩阵仍为后续增强项）。
- [x] 完成跨入口 provider、安装、打包、体积和最终验证的当前可交付范围。

## Global Constraints

- 新增/改变公共 contract、feature 行为、并发和状态机必须使用 TDD，先观察定向测试因目标行为缺失而失败。
- 通用 IPC crate 不得依赖 GPUI 或具体数据库配置类型。
- 不在 UI 文件中大面积散布 `#[cfg(feature = ...)]`；条件编译集中在 backend/factory/Cargo manifest。
- 默认构建必须保持 Redis/MongoDB UI 可用，驱动未安装时走明确安装提示。
- built-in feature 最终默认 false；IPC 未完成前不得提前破坏默认构建。
- 现有 SQL IPC、数据库 driver manifest 和安装流程保持兼容。
- 密码、token、完整连接串不得进入普通日志、测试快照或错误附加数据。
- Host 持有 SSH tunnel；sidecar 不接触 SSH 凭据。
- 高频事件不得进入无界 notification 数据面。
- 每完成一个 Task，同步勾选本计划并记录实际验证命令与关键决策。

---

### Task 1: 建立基线与结构 contract

**Files:**
- Modify: `docs/superpowers/plans/2026-07-16-native-ipc-drivers.md`
- Add tests near: `crates/extension-host/src/*`
- Add tests near: `main/src/*` or relevant runtime crates

- [x] 运行并记录基线：`rtk cargo test -p extension-protocol -p extension-host -p extension-driver`。
- [x] 运行并记录 SQL IPC 基线：`rtk cargo test -p db ipc` 和 `rtk cargo check -p main`。
- [x] 添加默认 feature 结构 contract：最终 `main` 必须声明 `builtin-redis`、`builtin-mongodb`，且二者不在 default 中；已确认因 feature 尚未实现而失败。
- [x] 添加依赖结构 contract：Redis/Mongo SDK 必须只出现在 optional/builtin 或 sidecar manifest 中；已确认默认 runtime 仍直连 Redis SDK 而失败。
- [x] 添加通用 session API contract：`extension-host` 必须导出 `ProcessRpcSession` 和配置类型；已先确认编译失败，Task 2 已转 Green。
- [x] 将实际基线结果、已知失败和耗时记录到本计划 Execution Notes。

### Task 2: 抽取通用 `ProcessRpcSession`

**Files:**
- Create: `crates/extension-host/src/process_session.rs`
- Modify: `crates/extension-host/src/lib.rs`
- Modify: `crates/extension-host/Cargo.toml` if required
- Modify: `crates/db/src/ipc/client.rs`

**Produces:**
- `ProcessRpcSessionConfig`
- `ProcessRpcSession`
- typed/raw request、notify、notification receiver、capability、shutdown、closed state

- [x] 在 `extension-host` 先写 fake local process/session 测试：init 成功、typed/raw request、超时、运行中取消、notification 和 shutdown。
- [x] 运行 `rtk cargo test -p extension-host --test process_session_contract`，确认因 API 缺失而失败。
- [x] 实现最小 `ProcessRpcSession`，组合现有 `process`、`transport`、`client`、`negotiation`，不引入数据库类型。
- [x] 重新运行定向测试直至通过。
- [x] 把 `db::ipc::client::JsonRpcClient` 改成薄包装，保留现有 public API 和错误映射。
- [x] 运行 `rtk cargo test -p db ipc`、`rtk cargo test -p extension-host`、`rtk cargo check -p db`。
- [x] 更新设计/计划中与实际 API 不一致的部分。

### Task 3: 抽取通用 manifest、API 分类与 registry

**Files:**
- Create or modify common manifest module in `extension-host` or a new focused crate after Task 2 review
- Modify: `crates/db/src/ipc/registry.rs`
- Modify: `crates/db/src/ipc/registry/*`
- Modify: `crates/extension-runtime/src/extension/database_driver_provider.rs`
- Modify tests under `crates/db/src/ipc/registry/tests.rs`

**Produces:**
- `NativeDriverManifest`
- `NativeDriverEntry`
- `NativeDriverTransport`
- `DriverCompatibility`
- `DriverProcessPolicy`
- registry query by `api`

- [x] 写 manifest contract：通用 manifest 支持非 SQL `api`、兼容信息和 connection/shared process scope；先运行确认缺少 API 而失败。
- [x] 写 registry contract：旧 manifest 默认 `database`，按 `api` 查询不会跨 API 暴露；先运行确认旧 registry 缺少 API 分类。
- [x] 实现独立公共 manifest，并为向后兼容的 `IpcDriverManifest` 增加默认 `database` 的 `api` 分类；避免通用层依赖 `db`。
- [x] 保持旧字段和序列化兼容，现有 SQL registry 测试全部通过。
- [x] 泛化 extension summary/install metadata，使 driver summary 保留 API、版本、图标和兼容信息。
- [x] 运行 `rtk cargo test -p db ipc::registry`、`rtk cargo test -p extension-runtime database_driver`。

### Task 4: 通用 blob 与事件流协议

**Files:**
- Create: `crates/extension-protocol/src/blob.rs`
- Create: `crates/extension-protocol/src/event_stream.rs`
- Modify: `crates/extension-protocol/src/lib.rs`
- Modify: `crates/extension-protocol/src/method.rs`
- Modify: `crates/extension-protocol/src/error.rs`

- [x] 先写 `WireBytes` UTF-8/Base64 round-trip、inline/blob threshold、chunk byte limit contract 测试并确认失败。
- [x] 定义 blob open/read/close DTO，与 SQL `stream/*` 并列但不复制 SQL row-stream 语义。
- [x] 写 event stream contract：有界 batch、`wait_ms`、`max_events`、`dropped_count`、closed state；先确认失败。
- [x] 实现通用 DTO 和 method 常量，不加入 Redis/Mongo 业务逻辑。
- [x] 运行 `rtk cargo test -p extension-protocol`。

### Task 5: 异步 sidecar runtime

**Files:**
- Create: `crates/extension-driver/src/async_runtime.rs`
- Modify: `crates/extension-driver/src/lib.rs`
- Add focused tests in `crates/extension-driver/src/async_runtime.rs` or test modules

**Produces:**
- `AsyncNativeDriver`
- `AsyncDriverConnection`
- `serve_async`

- [x] 先写 fake async driver contract：init gate、conn/open、连接内请求、多个 conn 并发、cancel、conn/close、shutdown。
- [x] 运行 `rtk cargo test -p extension-driver --test async_runtime_contract`，确认缺少 API 而失败。
- [x] 实现 Tokio task、per-connection async mutex 路由和 writer 串行化，不修改同步 `serve` 行为。
- [x] 增加 blob/event resource 路由和清理 contract：资源 ID 可在无 `conn_id` 的 read/close 请求中回到所属连接，资源/连接关闭后路由移除，取消结果不注册资源。
- [x] 运行 `rtk cargo test -p extension-driver` 和 `rtk cargo clippy -p extension-driver --all-targets -- -D warnings`。

### Task 6: 泛化安装 guard 和驱动选择

**Files:**
- Modify: `crates/extension-runtime/src/database_driver_install.rs`
- Create focused native driver install/selection module if the SQL name becomes misleading
- Modify related tests
- Modify main open strategies only through a common guard

**Produces:**
- `NativeDriverRequirement`
- `required_driver(api, config)`
- `prompt_install_native_driver`

- [x] 写 contract：builtin backend 不要求 sidecar；默认 Redis 要求 `redis`；默认 Mongo 要求 `mongodb-modern`；明确 incompatibility 可建议 legacy；其他错误不 fallback。
- [x] 运行定向测试并确认旧 SQL-only guard 缺少通用 API/backend contract。
- [x] 提取通用 requirement、提示文案和完成回调，SQL helper 作为兼容包装保留；下载/安装继续复用统一 `DatabaseDriver` marketplace kind。
- [x] 运行 extension-runtime database driver/install 定向测试和 `main` check。

### Task 7: 建立 Redis 领域 runtime 与 feature 边界

**Files:**
- Create: `crates/redis-runtime/Cargo.toml`
- Move/adapt domain files from `crates/redis_view/src/types.rs` and connection trait
- Modify: root `Cargo.toml`
- Modify: `crates/redis_view/Cargo.toml`
- Modify: `crates/redis_view/src/lib.rs`
- Modify: `crates/redis_view/src/manager.rs`
- Modify: `crates/onetcli_runtime/Cargo.toml`

- [x] 先写 feature contract：`redis-runtime` default 不启用 SDK；`builtin-redis` 才启用 optional `redis_client`。
- [x] 抽取 Redis domain types、trait、error、Pub/Sub handle contract 和 backend selection，保持现有 API 行为。
- [x] 把当前实现移动为 `BuiltinRedisConnection` 并置于 `#[cfg(feature = "builtin-redis")]`，SDK/SSH 依赖集中在 runtime feature。
- [x] 让 `redis_view` 只依赖 runtime facade，UI 和 global init 无条件存在。
- [x] 为 backend selection 添加纯 contract 测试，builtin feature 开关选择正确 backend。
- [x] `redis_view` 默认 feature 已切为空；main 默认不再包含 Redis SDK，builtin feature 组合仍通过。

### Task 8: Redis IPC contract、sidecar 与 provider 统一

**Files:**
- Add: `crates/extension-protocol/src/redis.rs`
- Migrated to `navop-extensions/extensions/ipc/redis/*` with its standalone
  release metadata and package entry.
- Add: Redis IPC backend under `redis-runtime/src/ipc/*`
- Modify: `crates/onetcli_runtime/src/redis_tools/*`
- Modify: `main/src/public_mcp_runtime/redis.rs`

- [x] 先写 Redis wire contract：binary argv、RESP value、pipeline 和 Pub/Sub control round-trip。
- [x] 实现 `redis/command`、`redis/pipeline`、连接生命周期 sidecar 垂直切片。
- [ ] 用 fake/in-process transport 验证 `IpcRedisConnection`，再用真实 sidecar 测试 standalone Redis。
- [x] 实现 Pub/Sub event stream：open/control/read/close、有界 buffer、overflow count、取消和 UI drop 清理。
- [x] 统一 Redis execution provider：GUI/Public MCP 使用 `GlobalRedisState`，headless/Agent/CLI 从同一 native registry 启动 `IpcRedisConnection`；旧 `redis_client` 路径只在 `builtin-redis` feature 下保留。
- [ ] 覆盖 TLS、SSH tunnel host target、非 UTF-8 key/value、大值 stream；Cluster/Sentinel 按当前承诺行为验证或明确返回 NotSupported。
- [ ] 运行 Redis runtime/view/onetcli/Public MCP 定向测试和相关 check/clippy。

### Task 9: 建立 MongoDB 领域 runtime 与 feature 边界

**Files:**
- Create: `crates/mongodb-runtime/Cargo.toml`
- Move/adapt domain files from `crates/mongodb_view/src/types.rs` and connection trait
- Modify: root `Cargo.toml`
- Modify: `crates/mongodb_view/Cargo.toml`
- Modify: `crates/mongodb_view/src/*` imports

- [x] 先写 feature contract：`mongodb-runtime` 始终依赖 `bson`，完整 `mongodb` 仅由 `builtin-mongodb` 启用。
- [x] 新增 `MongoFindOptions` 并写 limit/skip/sort/projection contract 测试。
- [x] 把当前实现移动为 `BuiltinMongoConnection`，仅在 feature 下编译。
- [x] UI 改用独立 `bson` 和 runtime facade，不直接引用 `mongodb::options`/`mongodb::error`。
- [x] 运行 Mongo runtime/view 默认与 builtin feature 编译组合。

### Task 10: MongoDB IPC、modern/legacy sidecar 与兼容选择

**Files:**
- Add: `crates/extension-protocol/src/mongodb.rs`
- Migrated to the shared `navop-extensions/drivers/mongodb-driver/*` crate and
  `extensions/ipc/mongodb-modern|mongodb-legacy/*` release manifests.
- Add: Mongo IPC backend under `mongodb-runtime/src/ipc/*`
- Modify driver compatibility selection and tests

- [x] 建立 BSON binary wire 基础 contract，BSON 文档不经过 relaxed JSON 数据面；完整 BSON 类型矩阵仍需扩展测试。
- [ ] 实现连接、数据库/集合、find/cursor、aggregate、count、CRUD、indexes、validation、explain。（sidecar 当前已完成连接、ping、BSON command 和基础 find；cursor/CRUD/index/validation 待补。）
- [x] 对接 blob stream：Mongo find 超过 4 MiB 时返回 blob reference，sidecar 以长度前缀 BSON 文档流分块，host 通过通用 blob/read 解包；接近 framing 上限的真实服务矩阵仍待执行。
- [ ] modern driver 对旧 wire version 返回结构化 `server_incompatible`；Host 只在该错误下选择 legacy。
- [ ] 确定用户要求的最低 MongoDB server 版本，并用真实 server/Docker matrix 验证 modern/legacy。
- [ ] 覆盖认证、TLS、副本集、direct connection、SRV 与 SSH tunnel host target。
- [ ] 运行 Mongo runtime/view/driver 定向测试、check 和 clippy。

### Task 11: 切换默认 feature 并清理主程序依赖路径

**Files:**
- Modify: `main/Cargo.toml`
- Modify: `crates/redis_view/Cargo.toml`
- Modify: `crates/mongodb_view/Cargo.toml`
- Modify: `crates/onetcli_runtime/Cargo.toml`
- Modify: `Cargo.lock`
- Add structural feature/dependency tests

- [ ] 增加 `builtin-redis`、`builtin-mongodb`、`builtin-data-drivers`，确保不在 default。
- [ ] 默认依赖使用 `default-features = false`，sidecar/IPC backend 始终可用。
- [ ] 运行默认、单 builtin、双 builtin、no-default feature 编译矩阵。
- [ ] 运行 `cargo tree -p main -i redis`，默认应无到 main 路径；开启 feature 后应存在。
- [ ] 运行 `cargo tree -p main -i mongodb`，默认应无到 main 路径；开启 feature 后应存在。
- [ ] 确认 Redis/Mongo UI、form、tab、keybindings 在默认构建仍编译和注册。

### Task 12: 打包、安装、升级与体积验证

**Files:**
- Modify/add driver packaging scripts under `script/`
- Modify extension marketplace/install tests
- Modify release packaging tests as required
- Update documentation

- [ ] 为 macOS/Linux/Windows sidecar 生成独立产物和 `driver.json`。
- [x] 驱动包执行 hash 验证，安装失败不破坏上一版本；发布签名仍由平台 release 流程负责。
- [ ] 覆盖驱动缺失、损坏、协议不兼容、启动超时、异常退出、升级和回滚。
- [ ] 对比默认 release 与 built-in release 的主二进制、DMG/压缩包和安装后 driver 总占用。
- [ ] 明确默认发行包不内置 sidecar；如需离线 full bundle，作为独立发行变体。

### Task 13: 最终验证、review 和经验沉淀

- [ ] 运行 `rtk cargo fmt --all -- --check`。
- [ ] 运行所有受影响 crate 的 `cargo test`。
- [ ] 运行 `rtk cargo check -p main` 及完整 feature 矩阵。
- [ ] 运行相关 crate/main `cargo clippy --all-targets -- -D warnings`。
- [ ] 运行真实 Redis/MongoDB integration matrix（当前环境未提供服务端，保留为发布前环境验证项）。
- [x] 运行 packaging 自校验：manifest entry 存在且可执行、SHA256SUMS 排除自身并通过 `shasum -c`。
- [ ] 运行 `rtk git diff --check`、检查文件/函数规模、审查无关改动和秘密泄漏。
- [ ] 对照设计规格逐条完成 completion audit；缺少证据的条目保持未完成。
- [ ] 将通用 IPC 的关键约束、验证入口、Tokio runtime/流式背压经验沉淀到项目级 `AGENTS.md`。
- [ ] 未经用户明确要求，不 commit、push 或创建 PR。

---

## Execution Notes

### 2026-07-16

- Worktree/branch created from clean `HEAD` `51936bd8`.
- Main workspace had unrelated uncommitted Redis/AGENTS changes; they were not copied into this worktree.
- Design decision: common process RPC lifecycle belongs in `extension-host`, while Redis/Mongo remain domain protocols and backends above it.
- Design decision: keep sync SQL driver runtime intact and add a parallel async runtime.
- Design decision: built-in features remain rollback/development paths and are not default release features.
- Baseline `rtk cargo test -p extension-protocol -p extension-host -p extension-driver`: 276 tests passed across 6 suites.
- Baseline `rtk cargo test -p db ipc`: 119 tests passed, 518 filtered out across 3 suites.
- Baseline `rtk cargo check -p main`: passed for 847 crates; only existing future-incompatibility warnings for `block` and `proc-macro-error2` were reported.
- Task 1 Red: `extension-host --test process_session_contract` failed because `ProcessRpcSession` was not exported; `main native_driver_feature_contract_tests` failed because builtin features and optional SDK dependencies were not yet implemented.
- Task 2 Green/Verify: `rtk cargo test -p extension-host process_session` passed 5 tests; full `rtk cargo test -p extension-host` passed 53 tests; `rtk cargo test -p db ipc` passed 119 tests; `rtk cargo check -p db` passed with the existing future-incompatibility warning.

### 2026-07-17

- Task 3 partial Green: added database-agnostic `NativeDriverManifest` and API-aware SQL compatibility registry. `rtk cargo test -p extension-host manifest` passed 5 tests; `rtk cargo test -p db ipc::registry` passed 37 tests; extension summary/install metadata remains pending.
- Task 4 Green/Verify: added bounded blob/event DTOs and method constants. Full `rtk cargo test -p extension-protocol` passed 213 tests at the task boundary.
- Task 5 Red/Green: async runtime export contract first failed; duplex behavior tests then exposed duplicate `driver.shutdown()` on explicit shutdown and missing blob/event resource routing. Both were fixed in the generic runtime.
- Task 5 Verify: `rtk cargo test -p extension-driver` passed 26 tests across 4 suites; `rtk cargo clippy -p extension-driver --all-targets -- -D warnings` reported no issues. Combined regression passed: protocol/host 271 tests and DB IPC 121 tests with 518 filtered out. `rtk git diff --check` passed.
- Task 3 final Green/Verify: SQL manifests now preserve opaque compatibility JSON; `ExtensionSummary` preserves driver API and compatibility through staged/marketplace installation. `rtk cargo test -p extension-runtime database_driver` passed 18 tests (111 filtered), and `rtk cargo check -p main` passed with only the two known future-incompatibility warnings.
- Task 6 Green: introduced `NativeDriverRequirement`/`NativeDriverBackend`, generic install prompt wording, and structured-only compatibility fallback using `SERVER_INCOMPATIBLE`. Extension-runtime database driver tests passed 20 tests at this boundary.
- Task 7 Green: created GPUI-free `redis-runtime`; moved domain types, connection trait, Pub/Sub handle contract and the full SDK/SSH implementation behind optional `builtin-redis`. Runtime default test passed without the SDK; builtin runtime passed 14 tests and `redis_view` passed 29 UI tests (the moved builtin tests now run in runtime).
- Task 8 partial Green: added binary-safe Redis wire DTOs and a local-socket `onetcli-redis-driver` sidecar with `conn/open`, `redis/command`, and bounded pipeline validation. Protocol passed 215 tests; sidecar passed 2 tests and clippy with `-D warnings`; extension-driver regression passed 26 tests.
- Task 8 host slice: `NativeDriverManifest::process_session_config` now centralizes generic spawn/socket/negotiation construction. `redis-runtime::IpcRedisConnection` starts a generic `ProcessRpcSession`, opens a Redis connection, and exposes binary-safe command/pipeline calls; default runtime passed 3 tests and builtin runtime passed 16 tests. Full `RedisConnection` facade integration and Pub/Sub event stream remain pending before the default feature can switch.
- Task 8 facade/default Green: `IpcRedisConnection` now implements the full existing `RedisConnection` surface through binary argv, including keys, scans, hashes, lists, sets, sorted sets, streams, details and server metadata. Redis Pub/Sub uses a bounded sidecar event buffer with pull reads and overflow counters. `redis_view` default features are empty; `main` checks without the Redis SDK, while builtin runtime remains available and passed 17 tests.
- Task 9 partial Green: created `mongodb-runtime` with direct `bson`, SDK-optional `builtin-mongodb`, domain types/trait and `MongoFindOptions`. Default/builtin contract tests pass and focused no-deps clippy reports no issues.
- Task 10 partial Green: added binary BSON MongoDB protocol and a shared modern/legacy sidecar package. Both binaries reuse the generic async runtime; command execution preserves BSON bytes. Sidecar tests and clippy pass. Full find/cursor/CRUD/index/validation implementation and view migration remain pending.
- Task 9/10 migration Green: `mongodb_view --no-default-features` and `--features builtin-mongodb` both compile; `cargo check -p main` no longer has an inverted `mongodb` dependency. Main feature contract tests now pass, and `builtin-redis`, `builtin-mongodb`, `builtin-data-drivers` checks all pass. Mongo IPC UI facade is intentionally explicit `IpcMongoConnection` until the real session-backed CRUD implementation lands.
- Default dependency matrix evidence: `cargo tree -p main -i redis_client` and `cargo tree -p main -i mongodb` both report no matching package under default features. `onetcli_runtime` now keeps its legacy direct Redis tools behind `builtin-redis`; the default headless registry is ready for replacement by the session-backed provider.
- Redis provider Green: `onetcli_runtime::redis_tools` is available in default builds and now executes through installed native manifests plus `IpcRedisConnection`; Agent/Public MCP registration no longer depends on `builtin-redis`. Its library tests pass 47 cases.
- Mongo IPC facade now owns optional manifest/session/conn state and implements real generic-session connect, disconnect, ping, BSON command and find decoding. The sidecar implements BSON-preserving find with limit/skip/sort/projection. Remaining facade methods still fail explicitly instead of falling through to an SDK.
- Mongo command facade Green: list databases/collections, create/drop collection/database, aggregate, count, CRUD, indexes, collection validation and explain now map to standard MongoDB command documents over the same generic BSON RPC; only cursor get-more/large-result blob routing remains to complete. Main initializes installed Redis/Mongo manifests through `NativeDriverRegistry` and configures the runtime factories without coupling UI to SDKs.
- Mongo large-result Green: sidecar packs BSON documents as length-prefixed binary blobs above the 4 MiB inline threshold; generic async runtime registers `documents_blob_id` as a connection-owned resource, host reads/decodes chunks and closes the blob. Runtime blob round-trip test passes.
- Packaging responsibility moved to `navop-extensions/scripts/release-driver.mjs`,
  which builds and verifies the three independent native driver packages.
- Size A/B: default release `target/release/navop` is 93,595,648 bytes; `--features builtin-data-drivers` is 98,511,056 bytes (+4,915,408 bytes, about 5.25%). The standalone native-driver package is 12,688 KiB in the current macOS build.
- Final focused verification: `cargo fmt --all -- --check`; 338 tests across extension-protocol/host/driver, redis-runtime, mongodb-runtime and onetcli_runtime; default and `builtin-data-drivers` main checks; `git diff --check` all passed. Known future-incompatibility warnings remain limited to baseline `block` and `proc-macro-error2` packages.
- Follow-up quality gate: boxed `RedisConnectionFactory::Ipc` to satisfy `clippy::large_enum_variant`; focused no-deps clippy now passes for extension-protocol/host/driver, redis-runtime, mongodb-runtime and onetcli_runtime, followed by 53 runtime tests and a default main check.
- Feature matrix Green: `cargo check -p main --no-default-features` plus `builtin-redis`, `builtin-mongodb`, and `builtin-data-drivers` combinations all pass. Added a manifest contract test that parses all shipped Redis/Mongo manifests and verifies api/id/entry/methods.
- Install corruption guard Green: database-driver installation now rejects a packaged relative entry that is missing or is not a regular file before publishing the extension summary. The guard intentionally does not require Unix executable bits because archive formats/platform installers may restore permissions later; the release packaging script separately enforces executable artifacts. Database-driver tests pass 21 cases and default main check remains green.
- Upgrade rollback Green: the generic staging installer backs up the current driver before replacement and restores it when a structurally valid update is missing its packaged sidecar entry. The native-driver regression proves both the old binary and marker survive the failed upgrade.
- Real integration Green: with temporary Docker `redis:7-alpine` on port 6380 and `mongo:7` on port 27018, `redis-runtime/tests/sidecar_integration.rs` passed binary-safe SET/GET plus pipeline, and `mongodb-runtime/tests/sidecar_integration.rs` passed ping, collection creation, BSON insert/find and database cleanup through the modern sidecar. These tests run outside the restricted sandbox because the sidecar needs local sockets and Docker networking.
- Integration cleanup: temporary containers `navop-redis-it` and `navop-mongo-it` were removed after the successful run. The integration tests are environment-gated by `NAVOP_REDIS_DRIVER_BIN` / `NAVOP_MONGO_DRIVER_BIN` and skip cleanly when no service environment is present.
