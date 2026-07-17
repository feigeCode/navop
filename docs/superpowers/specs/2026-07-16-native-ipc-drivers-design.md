# 通用 Native IPC 与 Redis/MongoDB 外置驱动设计

## 背景

Navop 当前已经有一套服务于外部 SQL 数据库驱动的进程 IPC：宿主启动驱动子进程，子进程通过本地 socket 回连，双方使用长度前缀 JSON-RPC 通讯，并通过 `init`、`conn/*`、`query/*`、`cursor/*` 等方法完成生命周期和数据库操作。

这套实现的底层能力已经具备复用价值，但宿主侧组合逻辑仍位于 `db::ipc::client`，manifest、错误映射和连接配置也与 SQL 数据库绑定；driver runtime 的主要接口是同步阻塞模型。Redis 和 MongoDB 当前仍直接链接 `redis`、`mongodb` Rust SDK，导致主程序体积增大，并使 MongoDB SDK 版本与服务端兼容范围被固定在主程序发布周期内。

本设计把“通用进程 IPC”与“数据库驱动协议”拆成不同层级。通用层未来可用于其他 native sidecar、provider 或高风险扩展，不再要求每接入一种 IPC 组件都重新实现进程、握手、超时、取消、通知和退出诊断。Redis/MongoDB 则在通用层之上定义各自稳定的领域协议和可独立发布的 sidecar。

## 目标

- 抽取与数据库无关的进程 RPC session，统一管理 spawn、socket、framing、init、请求、通知、取消、超时、shutdown、进程退出和 stderr 诊断。
- 保持现有 SQL IPC 行为与 `driver.json` 兼容，SQL 层改为复用通用 session。
- 增加适合 Tokio-first 驱动的异步 sidecar runtime，不破坏现有同步 SQL runtime。
- Redis 和 MongoDB 的 UI、表单、tab、连接存储与领域 facade 始终保留在主程序中。
- Redis/MongoDB 默认通过 IPC sidecar 工作。
- 当前内置 Redis/MongoDB SDK 实现保留，分别由 `builtin-redis`、`builtin-mongodb` feature 控制，默认均为 false。
- 默认 `main` 依赖图不包含 `redis` 和 `mongodb` SDK；MongoDB UI 可保留独立 `bson` 依赖。
- Redis UI、Public MCP、Agent 和 `onetcli_runtime` 共用同一个连接/provider 抽象，避免第二套直连路径。
- MongoDB 支持 modern/legacy 多驱动并存，并只在明确的 wire-version 不兼容时切换兼容驱动。
- 大 BSON 文档和 Redis 大值使用有界流式传输，不依赖单个大 JSON frame。
- 高频 Pub/Sub 和未来 change stream 使用有界、可取消的事件流，不直接写入无界 notification 队列。

## 非目标

- 首版不把 Redis/MongoDB UI 本身做成扩展。
- 首版不删除现有内置实现；只有最终稳定并完成长期回归后才评估移除。
- 首版不改变 `StoredConnection` 的既有 `ConnectionType::Redis`、`ConnectionType::MongoDB` 语义。
- 首版不要求所有外部 SQL 驱动迁移到异步 runtime。
- 首版不实现一个 sidecar 进程承载所有连接；先采用连接级进程隔离，接口预留共享进程策略。
- 首版不通过 silent fallback 掩盖认证、TLS、DNS、网络或权限错误。

## 核心约束

- `extension-protocol` 和 `extension-host` 的公共层不得依赖 `db`、`redis_view`、`mongodb_view`、GPUI 或 `one_core::storage::DbConnectionConfig`。
- 通用 session 不解释业务 method 和 result，只负责 JSON-RPC、生命周期与进程所有权。
- SSH tunnel 继续由 Host 创建，sidecar 只接收解析后的目标地址，不接收 SSH 私钥或跳板机凭据。
- 密码、token、连接串不得进入普通 tracing 字段或 stderr 诊断文本。
- feature 只控制 SDK backend，不控制 Redis/MongoDB UI 是否存在。
- feature 条件编译集中在 backend 模块、factory 和 Cargo manifest，不扩散到页面渲染代码。
- 现有 `extension-driver::serve` 同步 API 保持兼容；异步 API 使用新的并列入口。
- 所有公共 contract 和 feature 行为使用 TDD；必须先观察到定向测试因缺少目标行为而失败。

## 分层架构

```text
Navop UI / Public MCP / Agent / onetcli_runtime
                    │
          Redis/Mongo domain facade
                    │
          ConnectionBackendFactory
              ┌─────┴─────┐
              │           │
          IPC backend   Builtin backend
          默认编译       feature 控制
              │           │
      ProcessRpcSession  redis/mongodb SDK
              │
 extension-host + extension-protocol
              │ local socket + framed JSON-RPC
              │
    Redis/Mongo native sidecar
```

### 通用协议层：`extension-protocol`

保持 JSON-RPC envelope、错误、framing 和 lifecycle；增加与业务无关的公共资源类型：

- `blob/open`、`blob/read`、`blob/close` 或兼容现有 `stream/read` 的通用 blob contract；
- `event/open`、`event/read`、`event/close` 的有界事件流 contract；
- 通用 `WireBytes`、`BlobRef`、`EventStreamId`；
- 通用连接丢失、恢复和 driver warning notification。

Redis/MongoDB 的 method 常量和 DTO 分别位于独立模块，不能塞进 SQL row/schema 类型。

### 通用宿主层：`extension-host::ProcessRpcSession`

新增与业务无关的 session：

```rust
pub struct ProcessRpcSession {
    handle: JsonRpcClientHandle,
    negotiated: ExtensionSession,
    owner: Mutex<Option<ProcessRpcSessionOwner>>,
    notifications: Mutex<Option<NotificationReceiver>>,
    request_timeout: Duration,
    shutdown_grace_ms: u32,
}

struct ProcessRpcSessionOwner {
    client: JsonRpcClient,
    process: ProcessHandle,
}
```

配置：

```rust
pub struct ProcessRpcSessionConfig {
    pub spawn: SpawnConfig,
    pub negotiation: NegotiationConfig,
    pub request_timeout: Duration,
    pub shutdown_grace_ms: u32,
}
```

公共能力：

- `start(config)`；
- `request<T>()` / `request_value()`；
- `notify()`；
- `take_notifications()`；
- `supports()` / `declares_method()`；
- `is_closed()`；
- `shutdown()`；
- Drop 时关闭 handle 并 kill child。

`db::ipc::client::JsonRpcClient` 变为薄包装，只负责：

- 把 `IpcDriverManifest` 与 `DbConnectionConfig` 翻译成 `SpawnConfig`；
- 提供 `database: 1.0` negotiation；
- 把 `HostError` 映射为 `DbError`。

### 通用 manifest 与 registry

从 SQL manifest 中抽取公共部分：

```rust
pub struct NativeDriverManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api: String,
    pub protocol_version: String,
    pub entry: NativeDriverEntry,
    pub transport: NativeDriverTransport,
    pub methods: Vec<String>,
    pub capabilities: Vec<String>,
    pub compatibility: DriverCompatibility,
    pub process: DriverProcessPolicy,
    pub ui: NativeDriverUi,
    pub manifest_dir: PathBuf,
}
```

旧 SQL `driver.json` 没有 `api` 时默认 `database`。新通用 manifest 缺少 `api` 时默认 `native`，避免意外注册为 SQL driver。SQL 专属的 dialect、connection policy、database capabilities 和 database UI 继续位于向后兼容的 `IpcDriverManifest`；公共进程/transport contract 由 `extension-host::NativeDriverManifest` 独立承载，后续消费者不再依赖 `db`。

registry 可以按 `api` 查询：

- `database`；
- `redis`；
- `mongodb`；
- 后续其他 native API。

### 异步 driver runtime

新增 `extension_driver::serve_async`，使用 Tokio request task 和每连接 async mutex 路由：同一连接串行、不同连接并发，不在同步 worker 中嵌套 runtime。同步 `serve` 保持不变。

异步 runtime 负责：

- `init` 门控；
- 多 `conn_id` 注册；
- connless request；
- 请求取消；
- blob/event resource ID 到连接的通用路由与清理；
- shutdown 和连接清理；
- writer 串行化。

首版 sidecar manifest 使用 `process.scope = connection`。后续 `shared` 策略可让同一 `(driver_id, version)` 进程承载多个连接，但不作为首版完成条件。

## Redis 领域层

新增无 GPUI 依赖的 `redis-runtime` crate，承载：

- `RedisConnection` trait；
- `RedisConnectionConfig`、`RedisError` 和领域类型；
- `IpcRedisConnection`；
- feature 控制的 `BuiltinRedisConnection`；
- `RedisConnectionFactory`；
- headless/provider 接口。

`redis_view` 只保留 GPUI state 和 UI，`onetcli_runtime` 也依赖 `redis-runtime`，不再直接依赖 `redis` SDK。

Redis wire 以二进制安全 argv 和 RESP value 为核心：

```text
redis/command
redis/pipeline
redis/pubsub/open
redis/pubsub/control
redis/pubsub/read
redis/pubsub/close
```

Pub/Sub 使用可取消长轮询事件流。notification 只传低频 `conn/lost`、`conn/restored` 和 warning。

## MongoDB 领域层

新增无 GPUI 依赖的 `mongodb-runtime` crate，承载：

- `MongoConnection` trait；
- `MongoConnectionConfig`、`MongoError`；
- 项目自有 `MongoFindOptions`，移除 `mongodb::options::FindOptions` 对 UI contract 的污染；
- `IpcMongoConnection`；
- feature 控制的 `BuiltinMongoConnection`；
- `MongoConnectionFactory`；
- BSON wire 编解码与 driver selection。

`bson` 始终是领域依赖，完整 `mongodb` SDK 仅属于 built-in feature 和 sidecar binary。

MongoDB wire 使用 BSON bytes/Base64，小对象 inline，大对象转 blob stream。正式 wire 不使用 relaxed Extended JSON，以保持 ObjectId、Int32/Int64、Decimal128、DateTime、Timestamp、Regex、Binary subtype、MinKey/MaxKey 无损。

modern/legacy driver 使用相同 API，不同 manifest compatibility。Host 只在结构化 `server_incompatible` 错误中自动选择兼容 driver；其他错误原样返回。

## Feature 语义

主程序：

```toml
[features]
default = ["wasm-components"]
builtin-redis = ["redis_view/builtin-redis", "onetcli_runtime/builtin-redis"]
builtin-mongodb = ["mongodb_view/builtin-mongodb"]
builtin-data-drivers = ["builtin-redis", "builtin-mongodb"]
```

最终行为：

| 构建 | Redis | MongoDB |
|---|---|---|
| 默认 | IPC | IPC modern/legacy |
| `builtin-redis` | 内置 SDK | IPC |
| `builtin-mongodb` | IPC | 内置 SDK |
| `builtin-data-drivers` | 内置 SDK | 内置 SDK |

内置 feature 开启时仍编译 IPC 代码，便于开发期 A/B 测试；feature 决定默认 factory backend。正式发行流程不得使用 `--all-features` 代替默认构建。

## 安装与回退

现有数据库驱动 marketplace/download/install UI 泛化为 native driver 安装入口，按 `api` 和 `driver_id` 查询。默认 Redis driver id 为 `redis`；MongoDB 默认 `mongodb-modern`，旧服务端明确不兼容时提示安装 `mongodb-legacy`。

驱动未安装不影响应用启动。连接表单保存仍可进行；测试连接或打开 tab 时触发安装提示。安装失败、driver crash 或协议不兼容必须展示可诊断错误，不能回退到未编译的 built-in backend。

## 大数据与背压

现有 frame 上限为 16 MiB，不能直接承载接近上限的 MongoDB BSON 文档或大型 Redis value。通用 blob stream 必须：

- 使用固定最大 chunk；
- 每次 read 有明确 byte 上限；
- 支持 cancel/close；
- driver 和 Host 都有总大小限制；
- 无消费者时停止缓存；
- 连接关闭和 driver shutdown 时释放资源。

事件流必须：

- 使用有界 ring/buffer；
- read 支持 `max_events` 和 `wait_ms`；
- 定义 overflow 策略并返回 dropped count；
- UI 关闭后取消 long-poll 并关闭 stream。

## 安全

- sidecar 是本机 native code，安装包必须校验 hash，并沿用现有签名/marketplace 信任链。
- Host 不记录 credentials；错误 `data.extra` 也不得包含密码或完整连接串。
- 随机本地 socket 名继续使用；进程 Drop/timeout 时强制清理。
- 驱动 compatibility 和 API version 必须在 init/manifest 两层校验。
- legacy MongoDB driver 在安装和显示时标记 EOL 风险。

## 验收标准

- 默认 `main` 构建和依赖树不包含 `redis`、`mongodb` SDK；开启对应 feature 后依赖恢复。
- Redis/MongoDB UI、表单、tab 和连接类型在默认构建中仍存在。
- SQL IPC 完整复用通用 session，并通过原有测试。
- fake sidecar contract 覆盖 spawn、init、并发请求、取消、超时、notification、shutdown、异常退出和 stderr。
- async runtime 覆盖多连接、连接内路由、取消、blob/event 清理和 shutdown。
- Redis UI、Public MCP、Agent、`onetcli_runtime` 不再存在默认直连 `redis` SDK 路径。
- Redis 支持二进制值、Pub/Sub 背压和大值 stream。
- MongoDB BSON 特殊类型无损，modern/legacy 选择只由结构化兼容错误触发。
- Host 负责 SSH tunnel，sidecar 不依赖 SSH SDK。
- feature 矩阵、定向测试、相关 crate check/test/clippy、格式、diff review 和 release 体积 A/B 验证完成。
