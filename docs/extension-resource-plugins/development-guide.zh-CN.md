# Navop 通用资源扩展开发指南

本文描述当前 Navop 通用资源扩展机制的实际开发方式。可运行参考实现位于：

```text
navop-extensions/extensions/composite/elasticsearch
```

适用场景包括 Elasticsearch、Kafka、RocketMQ、Nacos、Kubernetes、Docker、API 管理器等非 SQL 资源。

## 1. 核心模型

扩展由三个相互独立的部分组成：

1. **连接声明**：`contributes.connections` 描述连接表单、provider runtime 和可选 UI。
2. **Headless provider**：独立进程实现稳定的 resource/job/event/blob JSON-RPC 协议。
3. **可选 shell UI**：gpui-shell JavaScript 页面，只负责显示和交互，不负责连接持久化或 secret 管理。

连接统一保存为 Navop 的 `StoredConnection`，不会创建插件专属 profile 表。宿主自动提供：

- 连接名称
- Workspace
- Team
- Remark
- 云同步开关
- 创建、编辑、复制、删除
- 首页、侧边栏和 Quick Open
- 活跃连接保护
- 本地主密钥加密
- 云同步与团队归属

扩展只声明自己的业务字段，例如 URL、认证方式、用户名和 API Key。

## 2. 支持的扩展形态

### 2.1 Headless 连接

声明 `contributes.connections`，不设置 `shellViewId`。

用户打开连接时，Navop 激活 provider、执行 `resource/open`，并显示宿主提供的连接状态页。适合：

- 只被 Public MCP、Agent 或其他宿主能力消费的 provider
- 暂时没有自定义 UI 的中间件
- 只需要测试连接和保持资源生命周期的扩展

### 2.2 带 UI 的连接

连接声明设置 `shellViewId`，并声明对应 `contributes.shellViews`。

Navop 的打开顺序是：

```text
读取 StoredConnection
→ 激活 runtime
→ 宿主执行 resource/open
→ 将 provider resource id 包装为 mount-scoped opaque handle
→ 加载 gpui-shell 页面
→ 通过 navop.context 注入 opaque handle
```

shell UI 不会收到保存的 config 或 secret，也不应再次调用 `resource/open` 创建主连接。

### 2.3 独立 shell 页面

`shellViews` 也可以不关联连接，直接从扩展管理页打开。这种页面没有 `context.connection`，可用于帮助页、静态工具或不依赖保存连接的页面。

## 3. 项目目录

Native Rust composite 扩展建议使用以下结构：

```text
navop-extensions/
├── Cargo.toml
├── extensions/composite/my-provider/
│   ├── Cargo.toml
│   ├── extension.build.json
│   ├── extension.json
│   ├── src/
│   │   ├── main.rs
│   │   ├── server/
│   │   ├── state/
│   │   └── ...
│   ├── tests/
│   │   └── end_to_end.rs
│   ├── ui/
│   │   └── explorer.js
│   ├── icons/
│   ├── locales/
│   └── assets/
└── scripts/
```

在 workspace `Cargo.toml` 中注册 crate：

```toml
[workspace]
members = [
  "extensions/composite/my-provider",
]
```

扩展 crate 示例：

```toml
[package]
name = "my-provider"
version = "0.1.0"
edition.workspace = true

[dependencies]
extension-protocol = { workspace = true }
interprocess = { workspace = true, features = ["tokio"] }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true, features = ["macros", "net", "rt-multi-thread", "sync", "time"] }
uuid = { workspace = true }
```

## 4. 构建元数据

`extension.build.json` 用于 CI、打包、发布和本地安装：

```json
{
  "id": "my-provider",
  "kind": "composite",
  "language": "rust",
  "package": "my-provider",
  "binary": "my-provider",
  "path": "extensions/composite/my-provider",
  "targets": [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "i686-pc-windows-msvc"
  ],
  "releaseTagPrefix": "my-provider-v",
  "r2Prefix": "extensions/my-provider"
}
```

字段说明：

| 字段 | 说明 |
|---|---|
| `id` | 打包脚本使用的短 ID，通常等于目录名 |
| `kind` | 通用资源扩展固定为 `composite` |
| `language` | Native Rust provider 使用 `rust` |
| `package` | Cargo package 名 |
| `binary` | 构建出的二进制名，不含 Windows `.exe` |
| `path` | 源码目录 |
| `targets` | 发布目标列表 |
| `releaseTagPrefix` | GitHub/CNB release tag 前缀 |
| `r2Prefix` | R2 发布目录 |

## 5. extension.json

以下是一个完整的连接 + UI manifest：

```json
{
  "schema_version": 1,
  "id": "com.example.my-provider",
  "name": "My Provider",
  "version": "0.1.0",
  "publisher": "Example",
  "license": "Apache-2.0",
  "description": "My resource provider.",
  "categories": ["developer-tools"],
  "keywords": ["provider"],
  "engines": {
    "onetcli": ">=0.15.2",
    "gpui_shell": "0.2.0"
  },
  "api": {
    "extension": "1.0",
    "shell": "1.0"
  },
  "permissions": [
    "shell:exec",
    "spawn:./bin/my-provider",
    "net:tcp:*:443",
    "secrets:read:self.*"
  ],
  "runtime": {
    "ipc": [
      {
        "id": "main",
        "entry": {
          "command": "./bin/my-provider"
        },
        "transport": {
          "kind": "local_socket",
          "connect_timeout_ms": 5000
        },
        "auto_restart": true,
        "max_restart_attempts": 3,
        "shutdown_grace_ms": 2500
      }
    ]
  },
  "contributes": {
    "connections": [
      {
        "id": "default",
        "label": "My Provider",
        "description": "Connect to My Provider.",
        "runtimeId": "main",
        "resourceType": "my-provider",
        "shellViewId": "explorer",
        "form": {
          "tabs": [
            {
              "id": "general",
              "label": "General",
              "fields": [
                {
                  "id": "url",
                  "label": "URL",
                  "fieldType": "Text",
                  "required": true,
                  "defaultValue": "https://localhost:9443"
                },
                {
                  "id": "auth_type",
                  "label": "Authentication",
                  "fieldType": "Select",
                  "defaultValue": "none",
                  "options": [
                    { "value": "none", "label": "None" },
                    { "value": "token", "label": "Token" }
                  ]
                },
                {
                  "id": "token",
                  "label": "Token",
                  "fieldType": "Password",
                  "secret": true,
                  "required": true,
                  "visibleWhen": [
                    { "field": "auth_type", "equals": "token" }
                  ]
                }
              ]
            }
          ]
        }
      }
    ],
    "shellViews": [
      {
        "id": "explorer",
        "title": "My Provider",
        "entry": "ui/explorer.js",
        "surface": "tab",
        "singleton": false,
        "backends": {
          "main": "main"
        },
        "modules": [
          "context",
          "resource",
          "job",
          "event",
          "blob",
          "runtime",
          "log"
        ]
      }
    ]
  }
}
```

## 6. Manifest 规则

### 6.1 runtime.ipc

每个 IPC runtime 至少声明：

- `id`
- `entry.command`
- 与 command 完全对应的 `spawn:<command>` 权限

Native composite 当前要求包内只有一个 `runtime.ipc`。打包 Windows 版本时，脚本会自动将：

```text
./bin/my-provider
```

改为：

```text
./bin/my-provider.exe
```

并同步改写 `spawn:` 权限。

### 6.2 contributes.connections

必填字段：

| 字段 | 说明 |
|---|---|
| `id` | contribution 稳定 ID，会持久化到连接数据中 |
| `label` | 新建连接页面显示名称 |
| `runtimeId` | 指向当前扩展的 `runtime.ipc[].id` |
| `resourceType` | 传给 provider `resource/open` 的资源类型 |
| `form` | 声明式业务表单 |

可选字段：

- `description`
- `icon`
- `shellViewId`

`id` 和字段 ID 不能包含 `:`、`/`、`\`。以下字段 ID 由宿主保留：

```text
name
workspace
remark
sync_enabled
team_id
```

### 6.3 表单字段

支持的 `fieldType`：

| 类型 | JSON 值 | 说明 |
|---|---|---|
| `Text` | string | 单行文本 |
| `Number` | number | 整数或浮点输入；manifest 默认值当前要求整数格式 |
| `Password` | 不进入公开 config | 必须同时声明 `secret: true` |
| `TextArea` | string | 多行文本 |
| `Select` | string | 必须提供唯一的 options |
| `Checkbox` | boolean | 默认值必须为 `true` 或 `false` |

规则：

- `Password` 必须设置 `secret: true`。
- `secret: true` 只能用于 `Password`。
- secret 不能声明非空 `defaultValue`。
- 有 secret 字段时，manifest 必须声明 `secrets:read:self.*`。
- Select option value 必须唯一。
- Select 的 `defaultValue` 必须存在于 options 中。
- `visibleWhen` 引用的字段必须存在。
- 多条 `visibleWhen` 按 AND 处理。
- 编辑连接时，secret 留空表示保留；点击 Clear 表示明确删除。

### 6.4 shellViewId

连接关联 shell view 时：

- shell view 必须存在。
- `singleton` 必须为 `false`，因为每个连接需要独立 mount。
- modules 必须包含 `context` 和 `resource`。
- `backends` 必须至少有一个 alias 指向连接使用的 `runtimeId`。

## 7. 权限

常用权限：

```json
[
  "spawn:./bin/my-provider",
  "net:tcp:api.example.com:443",
  "net:tcp:*:9200",
  "secrets:read:self.*",
  "shell:exec"
]
```

说明：

- `spawn:` 必须与 manifest 中的 command 完全一致。
- `net:tcp:<host>:<port>` 用于 `resource/open` endpoint preflight。
- `net:tcp:*:<port>` 允许该端口上的任意主机，属于高权限声明，应谨慎使用。
- `secrets:read:self.*` 只允许 provider 解析属于自身扩展连接的 secret reference。
- `shell:exec` 仅在声明 shell view 时需要。
- Native provider 与 shell UI 属于同一扩展信任主体；当前不提供完整 OS 网络 sandbox。

## 8. Provider 进程

### 8.1 连接宿主 socket

宿主启动 provider 后，通过环境变量提供 local socket 名称：

```text
ONETCLI_EXT_SOCKET
```

provider 应连接这个 socket，而不是自己监听端口：

```rust
use interprocess::local_socket::{
    GenericNamespaced, ToNsName,
    tokio::{Stream, prelude::*},
};

let socket_name = std::env::var("ONETCLI_EXT_SOCKET")?;
let name = socket_name.to_ns_name::<GenericNamespaced>()?;
let stream = Stream::connect(name).await?;
```

完整入口参考：

```text
extensions/composite/elasticsearch/src/server/mod.rs
```

### 8.2 JSON-RPC framing

使用 `extension_protocol::framing`：

```rust
use extension_protocol::{
    envelope::{Request, Response, RpcMessage},
    framing::{recv_msg_async, send_msg_async},
};
```

provider 循环：

```rust
while let Ok(message) = recv_msg_async::<_, RpcMessage>(&mut reader).await {
    let RpcMessage::Request(request) = message else { continue };
    let response = handle_request(request).await;
    send_msg_async(&mut writer, &RpcMessage::Response(response)).await?;
}
```

### 8.3 init

provider 必须响应 `init`，返回版本、API 和支持的方法：

```rust
use extension_protocol::{lifecycle::InitResult, method};

let result = InitResult::new(env!("CARGO_PKG_VERSION"))
    .with_api("extension", "1.0")
    .with_method(method::RESOURCE_OPEN)
    .with_method(method::RESOURCE_PING)
    .with_method(method::RESOURCE_INVOKE)
    .with_method(method::RESOURCE_CLOSE);
```

只声明真实实现的方法。若支持 blob/job/event，应同时加入对应 method。

## 9. Resource 协议

### 9.1 resource/open

请求：

```rust
pub struct ResourceOpenParams {
    pub resource_type: String,
    pub config: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
}
```

宿主会把 manifest 表单的公开字段放入 `config`，并增加：

```json
{
  "credential_refs": {
    "token": "secret://self/42:token"
  }
}
```

provider 不应要求 shell UI 传入 secret 明文。

返回：

```rust
pub struct ResourceOpenResult {
    pub resource_id: String,
    pub capabilities: Vec<String>,
    pub metadata: Option<serde_json::Value>,
}
```

要求：

- 每次 open 返回不可预测、进程内唯一的 resource ID。
- provider 必须支持同时打开多个连接。
- provider 状态使用 `HashMap<ResourceId, Client>`，不能使用单个全局 client。
- open 应执行必要的连通性/版本校验。
- capabilities 使用领域 namespaced string，例如 `my-provider/item/list`。

### 9.2 resource/invoke

请求：

```rust
pub struct ResourceInvokeParams {
    pub resource_id: String,
    pub method: String,
    pub params: serde_json::Value,
}
```

领域 method 不加入公共 enum，使用稳定 namespaced string：

```text
my-provider/cluster/info
my-provider/item/list
my-provider/item/get
my-provider/search
```

返回 `ResourceInvokeResult { result: ResultRef }`。

### 9.3 ResultRef

```rust
pub enum ResultRef {
    Inline { value: serde_json::Value },
    Blob { id: String },
    EventStream { id: String },
}
```

- 小 JSON 使用 `Inline`。
- 超过约 4 MiB 的结果使用 `Blob`。
- 持续拉取事件使用 `EventStream`。

### 9.4 resource/ping 和 resource/close

`ping` 验证指定 resource 是否仍可用。

`close` 必须只清理该 resource 及其所属 job/blob，不得清空其他连接。关闭不存在的 ID 应返回稳定协议错误。

## 10. Secret

secret 保存流程：

```text
用户填写 Password 字段
→ Host 将值放入 ExtensionConnectionParams.secrets
→ ConnectionRepository 使用 Navop master key 加密
→ provider config 只获得 secret://self/<connection-id>:<field-id>
→ provider 通过 host/secret/resolve 解析
```

provider 反向请求示例：

```rust
use extension_protocol::{
    conn::SecretRef,
    envelope::{Request, RpcMessage},
    host::{ResolveSecretParams, ResolveSecretResult},
    method,
};

let request = Request::new(
    request_id,
    method::HOST_RESOLVE_SECRET,
    serde_json::to_value(ResolveSecretParams {
        secret_ref: SecretRef::new("secret://self/42:token"),
    })?,
);
```

必须校验返回 bytes 的编码和业务格式。不要记录 secret、secret response 或完整 credential config。

Test Connection 使用 `secret://self/test-<uuid>:<field>` 瞬态引用。测试结束后宿主会清除并覆写瞬态 secret 内存。

## 11. Job、Event 和 Blob

### 11.1 Job

长任务使用：

```text
job/start
job/status
job/cancel
job/result
job/close
```

状态只能是：

```text
queued
running
succeeded
failed
cancelled
```

job 必须记录所属 resource ID。关闭某个 resource 时，只取消和关闭其 job。

### 11.2 Event

有界事件流使用：

```text
event/open
event/read
event/close
```

`event/read` 返回：

```json
{
  "events": [],
  "closed": false,
  "dropped_count": 0
}
```

buffer 必须有界；溢出时累计 `dropped_count`，不能使用无界 notification 队列。

### 11.3 Blob

blob 数据使用 Base64 chunk：

```text
blob/read
blob/close
```

单次 `max_bytes` 上限为 4 MiB。caller 读到 `done: true` 后仍应显式 close。

## 12. gpui-shell UI

### 12.1 连接 context

连接关联 UI 通过以下 API 获取 context：

```js
import { current } from "navop.context";

const context = current();
const connection = context.connection;
const resource = connection.resource.handle;
```

结构：

```ts
{
  extensionId: string;
  viewId: string;
  backends: string[];
  connection: null | {
    id: number;
    name: string;
    contributionId: string;
    resourceType: string;
    resource: {
      handle: string;
      capabilities: string[];
      metadata: unknown;
    };
  };
}
```

`resource.handle` 是 mount-scoped opaque handle，不是 provider 原始 resource ID。不能跨 tab、跨 reload 或跨 provider generation 保存使用。

### 12.2 navop.resource

```ts
export function invoke(
  handle: string,
  method: string,
  params?: unknown,
): Promise<
  | { kind: "inline"; value: unknown }
  | { kind: "blob"; handle: string }
  | { kind: "event_stream"; handle: string }
>;

export function ping(handle: string): Promise<void>;
export function close(handle: string): Promise<void>;
```

模块也保留 `open(backend, resourceType, config)`，供独立工具页创建额外资源；连接主 UI 应使用 Host 已注入的 handle。

### 12.3 navop.job

```ts
start(resource, method, params?)
status(handle)
cancel(handle)
result(handle)
close(handle)
```

### 12.4 navop.event

```ts
open(resource, kind, capacity?)
read(handle, maxEvents?, waitMs?)
close(handle)
```

### 12.5 navop.blob

```ts
read(handle, maxBytes?)
close(handle)
```

`read` 返回 Base64 string，UI 自己解码。

### 12.6 navop.runtime

```ts
info(backend)
```

返回 backend、runtime ID 和当前 generation。

### 12.7 navop.log

```ts
debug(message)
info(message)
warn(message)
error(message)
```

单条日志上限 16 KiB。不要输出 secret、完整 token 或未脱敏连接 config。

### 12.8 最小 UI

```js
import { View, div } from "gpui";
import { v_flex } from "gpui-base";
import { current } from "navop.context";
import { invoke } from "navop.resource";

export default class Explorer extends View {
  init(_props, cx) {
    this.context = current();
    this.data = null;
    this.error = null;
    cx.spawn(async (cx) => {
      try {
        const handle = this.context.connection.resource.handle;
        const result = await invoke(handle, "my-provider/item/list", {});
        this.data = result.kind === "inline" ? result.value : result;
      } catch (error) {
        this.error = error.message;
      }
      cx.notify();
    });
  }

  render() {
    return v_flex()
      .size_full()
      .p(16)
      .child(this.error || JSON.stringify(this.data, null, 2));
  }
}
```

gpui-shell 状态应在 `init` 中创建，不要在 `render` 中创建 `InputState` 等 retained state。

## 13. 生命周期与清理

### 13.1 打开

```text
activate runtime
→ resource/open
→ 创建 mount session
→ 注册 opaque handle
→ 加载 shell script
```

若 resource open、policy 装配、JS link 或 view load 失败，宿主会先关闭 session resources，再释放 activation。

### 13.2 关闭

```text
unload shell view
→ cancel/close jobs
→ close events
→ close blobs
→ close resources
→ deactivate activation lease
```

Drop 只作为兜底，正常路径必须显式关闭。

### 13.3 Provider restart

handle 与 provider generation 绑定。provider restart 后，当前页面会进入失败状态：

```text
Provider restarted. Close and reopen this connection.
```

当前不会自动重放连接或写操作。用户重新打开连接后，宿主创建新 resource 和新 mount handle。

## 14. 测试

Provider 至少覆盖：

1. `init` 方法协商。
2. open/ping/invoke/close。
3. 多 resource 隔离。
4. 关闭一个 resource 不影响另一个。
5. secret permission 和 resolve。
6. network permission。
7. 大结果 blob 生命周期。
8. job 成功、取消、结果和 close。
9. event 有界读取和 close。
10. shutdown 清理。

Elasticsearch 示例：

```bash
cargo check -p elasticsearch-provider --all-targets
cargo test -p elasticsearch-provider --test end_to_end -- --test-threads=1
```

宿主相关验证：

```bash
cargo test -p extension-runtime
cargo test -p main
cargo run -p main
```

扩展仓库脚本测试：

```bash
node --test tests/scripts.test.mjs
```

## 15. 本地开发和安装

在 `navop-extensions` 仓库执行：

```bash
bash scripts/install-local-composite-extensions.sh my-provider
```

该脚本会：

1. 根据 host triple 选择 target。
2. 执行 release build。
3. 打包 extension.json、provider、UI 和资源。
4. 执行 package verifier。
5. 备份旧安装。
6. 安装到 Navop composite extension 目录。

默认安装目录：

```text
~/.config/navop/extensions/composite/<manifest-id>
```

自定义安装目录：

```bash
NAVOP_COMPOSITE_EXTENSION_DIR=/tmp/navop-extensions \
  bash scripts/install-local-composite-extensions.sh my-provider
```

安装后启动 Navop：

```bash
cargo run -p main
```

## 16. 手工打包

先构建目标二进制：

```bash
cargo build -p my-provider --release --target aarch64-apple-darwin
```

打包：

```bash
bash scripts/package-composite-extension.sh \
  my-provider \
  aarch64-apple-darwin \
  target/extension-artifacts \
  0.1.0
```

验证：

```bash
bash scripts/verify-composite-package.sh \
  target/extension-artifacts/my-provider-composite-aarch64-apple-darwin.tar.gz
```

包结构：

```text
extension.json
bin/my-provider
ui/explorer.js
icons/...
locales/...
assets/...
```

## 17. 发布

使用统一 release 脚本：

```bash
node scripts/release-driver.mjs \
  my-provider \
  0.1.0 \
  --target aarch64-apple-darwin \
  --artifact-dir artifacts
```

发布前更新仓库根 `manifest.json`：

```json
{
  "id": "com.example.my-provider",
  "kind": "composite",
  "name": "My Provider",
  "version": "0.1.0",
  "release_tag": "my-provider-v0.1.0",
  "description": "My resource provider.",
  "file_extensions": [],
  "manifest": "my-provider/manifest.json"
}
```

CI、Release、R2 upload 和 Windows x86 backfill 已识别 `extensions/composite`。

## 18. 发布检查清单

- [ ] `extension.json` 可被 Navop parser 加载。
- [ ] `verify-composite-package.sh` 通过。
- [ ] `spawn:` 权限与 command 完全一致。
- [ ] 所有网络 endpoint 都有对应权限。
- [ ] secret 字段使用 `Password + secret: true`。
- [ ] manifest 声明 `secrets:read:self.*`。
- [ ] connection shell view 非 singleton。
- [ ] connection shell view 包含 `context` 和 `resource`。
- [ ] shell backend 指向 connection runtime。
- [ ] provider 支持多 resource。
- [ ] resource close 不影响其他连接。
- [ ] job/blob/event 均有显式 close。
- [ ] shell UI 使用 `context.connection.resource.handle`。
- [ ] shell UI 不读取保存 config 或 secret。
- [ ] provider E2E 通过。
- [ ] `cargo test -p extension-runtime` 通过。
- [ ] `cargo test -p main` 通过。
- [ ] 扩展脚本测试通过。
- [ ] 本地安装后 Navop 能发现扩展。
- [ ] 真实目标服务完成一次 Test、Save、Open、Close 冒烟验证。

## 19. 当前边界

当前版本尚未提供：

- 插件专属的宿主级 SidebarContribution。
- 插件内部宿主级多内容 TabContainer。
- 自动恢复 provider restart 前的 resource/job。
- 任意 UI tree 协议。
- provider 网络访问的完整 OS sandbox。
- 扩展 connection schema 自动迁移 hook。
- shell 脚本访问保存 config 或 secret 明文。

需要数据库式左树和多内容区时，当前推荐在一个 shell view 内实现 split layout。未来宿主级 workspace 应作为独立 surface 演进，不应重新引入插件专属连接存储。

## 20. 参考文件

宿主：

```text
crates/extension-runtime/src/extension/manifest/contributes/connection.rs
crates/extension-runtime/src/registration.rs
crates/core/src/storage/models.rs
crates/core/src/storage/repository.rs
main/src/extension_connection_form.rs
main/src/extension_connection_tab.rs
main/src/shell_plugin_host/
main/src/universal_plugins.rs
```

示例扩展：

```text
navop-extensions/extensions/composite/elasticsearch/extension.json
navop-extensions/extensions/composite/elasticsearch/src/server/
navop-extensions/extensions/composite/elasticsearch/src/state/
navop-extensions/extensions/composite/elasticsearch/ui/explorer.js
navop-extensions/extensions/composite/elasticsearch/tests/end_to_end.rs
```

打包发布：

```text
navop-extensions/scripts/package-composite-extension.sh
navop-extensions/scripts/verify-composite-package.sh
navop-extensions/scripts/install-local-composite-extensions.sh
navop-extensions/scripts/release-driver.mjs
```
