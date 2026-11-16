# navop × nsh 页面 ABI v2（唯一事实源）

> 状态：实现中的目标契约。v1（`navop.context.current` / `navop.theme.*` / `navop.i18n.*` /
> `navop.log.*`（call）/ `navop.event.*`（轮询）/ `__NAVOP_ERROR__` 信封）整体删除，不做兼容层。
> 已发布扩展只有两个，页面一次性重写。

## 1. 原则

1. **`host.params` 是启动事实**：扩展 id、route、backends、连接，全部在首帧前由宿主
   `HostChannel::set_params` 写入，页面同步读，不 await。
2. **`host.on` 是事实变化**：只有「运行时才会变、且页面必须感知」的东西走推送。
3. **`host.call` 是命令**：有应答、可失败、带结构化错误码、页面自定超时。
4. **`host.post` 是通知**：日志这类「不需要应答、丢了也不影响正确性」的走单向通道。
5. **nsh 已有的子系统不再包一层**：
   - 主题 token / 明暗：宿主 `AppView::set_theme` + `set_appearance` 写入 nsh，
     页面用 `token()`、`bg-card` 之类的类名、`dark:`、`colorScheme()`（`nsh` 导出）。
     `navop.theme.*` 删除。
   - 语言：宿主 `AppView::set_locale` + `setColorScheme` 同级的 surface 环境，
     页面用 `nsh` 的 `locale()` / `Intl`。`navop.i18n.*` 删除。
   - 错误：`nsh` 的 `JsError::Fields`（`code` + 任意字段）。`__NAVOP_ERROR__` 删除。
6. **能力只有一个权威源**：`extension.json` 的 `modules` + `permissions`。
   页面不再声明 `capabilities.host`（nsh 打包产出的 `capabilities.json` 由宿主覆盖）；
   宿主把 `granted` 与 `declared` 都设为同一份派生结果，`harden = true` 恒开。

## 2. `host.params`

首帧前写入，运行期不变。页面在模块顶层读一次即可。

```jsonc
{
  "apiVersion": 2,
  "extension": {
    "id": "com.navop.docker",
    "title": "Docker"
  },
  "route": "containers",       // 本次打开的初始路由；无路由时为 null
  "backends": ["docker"],      // 已激活的 alias 列表，顺序同 shellApp.backends
  "connection": null,          // 非连接型 app 为 null；连接型 app 见下
  "context": null,             // 宿主 UI 触发打开时的触发场景；见下
  "surface": "tab",            // "tab"（默认）或 "panel"（宿主侧栏面板）
  "panel": null,               // surface == "panel" 时为面板 id，否则 null
  "anchor": null               // surface == "panel" 时为锚点 id：
                               //   "terminal-sidebar" | "db-sidebar"
}
```

`connection`（连接型页面，如 docker/containers）：

```jsonc
{
  "connectionId": "…",
  "name": "本地 Docker",
  "contributionId": "docker-local",
  "resourceType": "docker",
  "resourceHandle": "resource-…",   // 已由宿主打开的 provider 资源句柄
  "alias": "docker",
  "config": { /* 非敏感表单值 */ }
}
```

约束：

- `config` 只含非敏感字段（`connection.rs::ui_connection_context` 的现有语义）。
- `backends` 是**已激活**的 runtime alias（`shellApp.backends` 的 key）；激活失败时 app 根本不会被创建（宿主显示错误页）。
- `route` 是宿主给的**初始路由**，语义由 app 自己定义（侧栏页签、NavStack 的根、深链 id 都行）。连接型 app 用 `connections[].route`，工具箱打开为 `null`，`navop.dev.openApp` 可显式指定。
- app 里的页面不得假设 `connection` 存在；`null` 表示「这次打开没有绑定连接」。
- `surface` / `panel` / `anchor` 只在面板场景有值：`anchor` 是宿主挂载点，
  目前有终端侧栏 `terminal-sidebar` 与数据库侧栏 `db-sidebar`；tab 场景三者
  分别是 `"tab"` / `null` / `null`。同一 app 可以既开 tab 又开面板，宿主给
  每个 surface 起独立 `AppView` 实例（`params` 各自独立）。

`context`（宿主 UI 触发的打开，当前是 DB 树右键菜单 `kind: "nsh"` 命令与
`App.open` 的 `context` 参数）：

```jsonc
{
  "commandId": "example.inspect_table",
  "connectionId": "conn-…",
  "databaseType": "mysql",
  "driverId": null,
  "node": { "id": "…", "name": "users", "type": "table" }
}
```

仅启动时注入；同一 `(extension, route)` 复用 tab 时不会重放（同 key 视为同一
页面场景）。页面自定义 `App.open` 的 `context` 参数形状不限，宿主原样透传。

数据库侧栏面板（`anchor: "db-sidebar"`）也带 `context`，形状是「当前连接」而不是
菜单节点——宿主在打开面板的那一刻按所在标签页的活动连接填：

```jsonc
{ "connectionId": 12, "connectionType": "database", "params": { "db_type": "mysql" } }
```

页面用它确定自己挂在哪条连接上（想列库/表就接着走 `Db.*`）。切到另一条
连接时宿主会重建面板视图（新 `AppView`、新 `params`），缓存里的旧视图不会跨连接
复用。

## 3. 事件（`host.on`）

宿主 → 页面只有一组事件名。

### 3.1 `navop.event` — provider 事件流

订阅后宿主持续推送，页面拿 `host.on("navop.event", …)` 收。通道随 tab 关闭自动断开；
`host.on` 在渲染期注册的监听随组件卸载自动退订（nsh 语义）。

```jsonc
{ "handle": "event-…", "cursor": 42, "events": [ { "seq": 42, "data": { … } } ] }
{ "handle": "event-…", "cursor": 43, "done": true }                   // 流正常结束
{ "handle": "event-…", "cursor": 43, "error": { "code": "…", "message": "…", "retryable": false } }
```

- `cursor` 单调递增，页面重连时把它作为 `since` 传回 `navop.event.subscribe`。
- 单个批次上限由宿主定（当前 64 条）；积压超过缓冲区时宿主先发一个
  `error.code = "EVENT_OVERFLOW"`（retryable）并结束该订阅，页面用 `since` 续订。

## 4. 调用

两种机制，各管一类：

- **`host.call`（第 4.1–4.6 节）**：provider 资源、作业、大对象、事件流、运行时——
  字符串名字的 RPC。返回 Promise，失败以 `JsError` reject，页面按 `e.code` /
  `e.retryable` / `e.details` 读取（`details` 是对象或 `undefined`）。
- **`nativeModule("navop")`（第 4.7–4.10 节）**：navop 自己的四个能力面
  （`App` / `Ui` / `Terminal` / `Db`）——nsh 的强类型宿主模块。同步方法直接返回，
  `async` 的返回 Promise；失败以 `Error(message)` reject，消息以稳定的错误码开头
  （`PERMISSION_DENIED: …` / `INVALID_ARGUMENT: …`），没有 `e.code` 字段。

```ts
const api = nativeModule<NavopNative>("navop");
await api.Ui.notify({ title: "导出完成" });
const { connections } = await api.Db.listConnections();
```

### 4.1 上下文

**没有取上下文的调用**：`host.params` 就是全部启动事实。连接变化（provider 重启、
连接断开）由宿主处理——连接型 app 看到「provider 已重启，请重新打开这条连接」，
非连接型 app remount 并重新写 `params`；所以 app 生命周期内不存在「params 变旧」的
情况，也不需要 `context.current` 这样的重取接口。

### 4.2 资源

| 名字 | 参数 | 返回 |
|---|---|---|
| `navop.resource.open` | `{ backend?, resourceType, config }` | `{ handle }` |
| `navop.resource.invoke` | `{ handle, method, params?, options? }` | provider 结果（inline 值或 `{ blob: handle }`） |
| `navop.resource.ping` | `{ handle }` | `{ ok: true }` |
| `navop.resource.close` | `{ handle }` | `null` |

`options`：`{ timeout?: number }`（毫秒，`0` 表示不超时；**不传就是 nsh 的 30 秒默认**）。
超过 30 秒的 provider 方法必须显式传 `timeout`（建议 `0`，由 provider 自己的超时决定）。

### 4.3 作业

| 名字 | 参数 | 返回 |
|---|---|---|
| `navop.job.start` | `{ handle, method, params?, options? }` | `{ job }` |
| `navop.job.status` | `{ job }` | `{ state, progress? }` |
| `navop.job.cancel` | `{ job }` | `null` |
| `navop.job.result` | `{ job, options? }` | 结果 |
| `navop.job.close` | `{ job }` | `null` |

### 4.4 大对象

| 名字 | 参数 | 返回 |
|---|---|---|
| `navop.blob.read` | `{ handle, maxBytes? }` | `{ data: number[], eof: boolean }` |
| `navop.blob.close` | `{ handle }` | `null` |

### 4.5 事件流

| 名字 | 参数 | 返回 |
|---|---|---|
| `navop.event.subscribe` | `{ handle?, backend?, kind, params?, since? }` | `{ handle, cursor }` |
| `navop.event.unsubscribe` | `{ handle }` | `null` |

订阅成功后事件走 `navop.event` 推送（§3.1），不再有 `read` 轮询。

`handle` 写 `"host"` 表示订阅**宿主事件**（宿主进程里的事实变化，不需要
provider 资源句柄），此时 `kind` 就是 topic 名，返回的句柄形如
`host:{topic}:{n}`：

| topic | 时机 | 事件体 |
|---|---|---|
| `db.treeChanged` | `Db.execute` 成功执行写/结构变更语句，或 `Db.refreshTree` 成功后 | `{ connectionId, sessionId?, reason: "execute" | "refresh" }` |

宿主事件没有回放：订阅之前发生的收不到，`since` 不参与（游标从 0 起）；
溢出（单 topic 缓冲 64 条）同样先推 `EVENT_OVERFLOW` 再结束订阅。退订时
`navop.event.unsubscribe` 对 `host:` 句柄只停本地的泵，不会去关 provider 的流。

### 4.6 运行时

| 名字 | 参数 | 返回 |
|---|---|---|
| `navop.runtime.info` | `{ backend? }` | `{ backend, runtimeId, generation }` |

### 4.7 导航（`App`）

| 方法 | 参数 | 返回 |
|---|---|---|
| `App.open` | `{ extensionId?, route?, context? }` | `{ ok: true }` |

打开本扩展（`extensionId` 缺省）或其他已安装扩展的 shell app tab。`route` 进
目标页面 `host.params.route`；`context`（任意 JSON 对象）进
`host.params.context`。目标 tab 按 `(extensionId, route)` 复用，已有 tab 只
激活、不重放 context。打开动作排到下一帧，所以应答只表示“已入队”；窗口不可用
时以参数错误 reject。

### 4.8 UI（`Ui`）

| 方法 | 参数 | 返回 |
|---|---|---|
| `Ui.notify` | `{ title, message?, level? }` | `{ ok: true }` |

宿主右下角 toast 通知。`level` 取 `info`（默认）/`success`/`warning`/`error`。
应答是乐观的：找不到可用窗口时通知被丢弃、仍返回 ok（通知不属于必须送达的
状态）。需要同步进度条的长任务请走 `navop.job`；`startProgress` 形态的资源
句柄暂不提供。

### 4.9 终端（`Terminal`）

| 方法 | 参数 | 返回 |
|---|---|---|
| `Terminal.list` | — | `{ sessions: TerminalSession[] }` |
| `Terminal.read` | `{ sessionId?, lines? }` | `TerminalReadResult` |
| `Terminal.write` | `{ sessionId?, text }` | `{ sessionId, sent: true, bytes }` |
| `Terminal.wait` | `{ sessionId?, previous?, lines?, timeoutMs? }` | `TerminalReadResult` + `changed` |
| `Terminal.exec` | `{ command, sessionId?, submit?, waitForOutput?, readyTimeoutMs?, timeoutMs? }` | `TerminalExecResult` |
| `Terminal.sshExec` | `{ sessionId, command, cwd?, env?, timeoutMs?, mode? }` | `TerminalSshExecResult` |

`TerminalSession`：`{ sessionId, connectionId, title, host, kind, connected,
cwd, rows, cols }`（`kind` 为 `"Local"`/`"Ssh"`）。`TerminalReadResult`：
`{ sessionId, text, returnedLines, availableLines, historySize, screenLines,
columns, altScreen, displayOffset, cursorRow, cursorColumn }`。

会话与读/写句柄复用 Public MCP 终端注册表（与 AI 工具同一通道）；只放行
已连接的本地/SSH 会话。`sessionId` 缺省取第一个已连接的会话。`write` 是
高权限动作（向用户终端输入），安装期能力面会提示；会话未连接或锁定时报错。
宿主主动推送（cwd 变化/命令结束）暂未提供：要“跟着输出走”用 `wait`
长轮询——`previous` 传上一轮拿到的 `text`，宿主每 150ms 比对一次，变化就回
`changed: true`；等满 `timeoutMs`（默认 2000，范围 200..15000）没有新输出就
回 `changed: false`（不是错误，可以接着再等）。

`Terminal.exec` 把命令**打进一个已经开着的终端**：经过用户的 shell，复用它的
cwd、环境和别名，本地与 SSH 终端都适用。`submit: false` 只写入不回车；
`waitForOutput: false` 只提交不等输出；`completion` 说明这次是怎么结束的
（`observed_output` 输出静止 / `shell_integration_exit` shell 集成报了退出码 /
`submitted_only` 没等 / `timed_out` 超时）。页面被关掉或扩展停用时，执行会
跟着取消。返回 `TerminalExecResult`：`{ sessionId, command, submitted,
completion, exitCode, output, truncated, capturedBytes, discardedBytes,
durationMs, commandId }`。

`Terminal.sshExec` 是**一次独立的远端调用**（只支持已暴露的 SSH 会话）：
stdout / stderr 分开返回，可以带 `cwd`、`env`，`mode: "background"` 会立刻
返回并给 `commandId`。返回 `TerminalSshExecResult`：`{ sessionId, status,
stdout, stderr, exitCode, durationMs, timedOut, commandId, startedAtMs }`，其中
`status` 为 `running|exited|failed|cancel_requested|cancelled|timed_out`。
前台执行会阻塞到命令结束，宿主把它放在 blocking 线程上，不会卡住页面。
（后台命令的 poll/output/cancel 还没接到能力面，需要时先说一声。）

### 4.10 数据库（`Db`）

| 方法 | 参数 | 返回 |
|---|---|---|
| `Db.listConnections` | — | `{ connections: [{ id, name, driver, database }] }` |
| `Db.openSession` | `{ connectionId, database? }` | `{ sessionId }` |
| `Db.execute` | `{ sessionId, sql, maxRows? }` | `RowBatch` |
| `Db.listDatabases` | `{ sessionId? , connectionId? }` | `{ databases }` |
| `Db.listSchemas` | `{ sessionId? , connectionId?, database }` | `{ schemas }` |
| `Db.listTables` | `{ sessionId? , connectionId?, database?, schema? }` | `{ tables: [Table] }` |
| `Db.describeTable` | `{ sessionId? , connectionId?, database?, schema?, table }` | `{ columns, indexes, foreignKeys }` |
| `Db.listFunctions` | `{ sessionId? , connectionId?, database?, schema? }` | `{ functions: [Function] }` |
| `Db.closeSession` | `{ sessionId }` | `{ ok: true }` |
| `Db.refreshTree` | `{ connectionId? }` | `{ ok: true, refreshed }` |

元数据方法（`listDatabases`/`listSchemas`/`listTables`/`describeTable`/`listFunctions`）
的数据源两种给法：`sessionId`（借一个已开会话的连接）或 `connectionId`（直接指定，
宿主内部自己开临时会话）；`database` 省略时用连接配置里的默认库。它们都是只读
元数据，统一要求 `db:schema:<conn|*>` 权限（与 WASM 侧一致），不占用 `db:read`。

形状：

```ts
Table    { name, kind: "table" | "view" | "foreign_table", schema?, comment?, engine?, charset?, collation? }
Column   { name, typeName, nullable, primaryKey, autoIncrement, defaultValue?, comment?, charset?, collation? }
Index    { name, columns: string[], unique, primary, typeName? }
ForeignKey { name, columns: string[], refTable, refSchema?, refColumns: string[], onDelete, onUpdate }
Function { name, schema?, returnType?, parameters: string[], identityArguments?, definition?, comment? }
```

`describeTable` 在宿主侧用**一个会话**把列、索引、外键一次取全（走
`load_table_metadata_direct`），页面不用先 `openSession` 再逐项查。

`RowBatch`：`{ columns: [{ name, typeName, nullable }], rows, nextCursor }`
（`bytes` 列编码为 `number[]`）。SQL 按语句逐条分级（读/写/DDL/管理），
权限沿用清单 `db:*` 声明，与 WASM 侧同一套 `ExtensionDbGateway` 校验；
`sessionId` 是宿主内会话句柄（`dbs-N`），页面卸载不保证清理，显式
`closeSession` 是好习惯。`refreshTree` 会真的刷新宿主里已打开的树（`refreshed` 是刷新的连接根数量，`0` 表示这棵树没打开）；页面对应的树按连接根整体重载，不需要自己拼节点 id。

### 4.11 开发者工具（仅内置扩展）

9 个名字与参数同 v1，但 `openView` 已改为 `openApp`：`navop.dev.{list,open,reload,watch,remove,logs,openApp,pickDirectory,pickResult}`。

`navop.dev.openApp` 的参数为 `{ extensionId, route? }`（对应旧 `openView` 的 `{ extensionId, viewId }`），返回 `null`。

**门禁**：只有 `extension.json` 里 `kind` 为内置（宿主 catalog 判定）且 id 在白名单
（当前 `com.navop.devtools`）的扩展才会注册这些 handler；第三方扩展声明 `dev` 模块
在安装期被 `shell_validation` 拒绝。页面侧看到的是 `PERMISSION_DENIED`。

## 5. 通知（`host.post`）

| 名字 | 参数 |
|---|---|
| `navop.log` | `{ level: "debug" \| "info" \| "warn" \| "error", message: string, fields?: object }` |

- 无应答；`message` 上限 16 KiB（超出截断并记一条宿主日志）。
- 未声明 `log` 模块的扩展，宿主不注册该名字，post 被静默丢弃（nsh 语义）。

## 6. 错误

`NavopError` → `nsh_core::element::JsError::Fields`：

```jsonc
{
  "code": "STALE_HANDLE",     // 12 个稳定码之一
  "message": "…",
  "retryable": false,         // 字段，缺省 false
  "details": { … }            // 字段，缺省不出现
}
```

稳定码（不变）：`INVALID_ARGUMENT` / `PERMISSION_DENIED` / `BACKEND_NOT_FOUND` /
`BACKEND_START_FAILED` / `RUNTIME_UNAVAILABLE` / `INVALID_HANDLE` / `STALE_HANDLE` /
`REQUEST_CANCELLED` / `REQUEST_TIMEOUT` / `RESULT_TOO_LARGE` / `PROTOCOL_ERROR` /
`EXTENSION_UNLOADED`。

页面写法：

```ts
try {
  await host.call("navop.resource.invoke", { handle, method: "docker/container/list" });
} catch (e) {
  if (e.code === "STALE_HANDLE") …; // e.retryable, e.details 同样可直接读
}
```

「模块没声明」与「参数错」必须可区分：前者 `PERMISSION_DENIED`（`details.module` 指出缺的模块），
后者 `INVALID_ARGUMENT`。

## 7. 模块（`extension.json`）

```jsonc
"shellApp": { "modules": ["resource"] }
```

- 可用模块：`resource` / `job` / `event` / `blob` / `runtime` / `log` / `dev` / `app` / `ui` / `terminal` / `db` / `ai`。
- `resource|job|event|blob` 仍要求 `backends` 非空。
- `app` / `ui` / `terminal` / `db` 是宿主内建能力面（无 provider 依赖），
  以 nsh 的 `nativeModule("navop")` 暴露（不是 `host.call` 名字）：
  - `app` → `App.open`；
  - `ui` → `Ui.notify`；
  - `terminal` → `Terminal.{list,read,write,wait}`（写入为高权限动作）；
  - `ai` → 默认由宿主在页面右下角画一枚 AI 浮钮，点开在右侧停靠一条可拖宽的聊天面板；
    `host.post("navop.ai.context")` 上报页面上下文（转给本页面的 AI 面板；只做提示，不授予任何数据权限）。
    页面若想自己决定位置，可用 `nativeComponent("navop.ai")` 嵌一份（见 plugin-ai-tools.md §9.4）；
  - `db` → `Db.*`（权限沿用清单 `db:*` 声明）。
  每个模块方法入口都按这份清单校验：没声明就是 `PERMISSION_DENIED`。
- 删除的模块：`context`（改 `params`）、`theme` / `i18n`（nsh 原生）。
- `log` 模块只管 `navop.log` post；不再有 `navop.log.info` 之类的 call。
- 鉴权派生：`granted = ["<module>", "<module>.*" for modules] ∪ fs/network/process/clipboard`；
  `declared = granted`（宿主权威，忽略页面声明），`harden = true`。
  为什么要带裸名字：`log` 模块唯一的宿主名是 `navop.log`（post），通配
  `navop.log.*` 匹配不到它。`spike.mounted` 这类测试专用名字只在 `cfg(test)` 里追加。
- **未声明的模块也要可区分**：宿主对 v2 的全部名字都注册 handler；模块没声明时
  该 handler 直接以 `PERMISSION_DENIED` reject（`details.module` 指出缺的模块），
  不是「宿主没有这个 handler」。这样「模块忘声明」与「名字写错」（后者是 nsh 的
  `ERR_NSH_HOST_ERROR`，因为宿主确实没有这个 handler）在页面侧可区分。

## 8. 超时与取消

- 页面超时由 `host.call` 第三参决定（`0` = 不超时）；宿主不再叠加页面级超时。
- `nativeModule` 方法没有 timeout 参数：宿主自己决定语义（终端 `wait` 用
  `timeoutMs` 长轮询，数据库沿用连接自身的超时）。
- 页面关闭：宿主取消该 view 的所有进行中调用，Promise 以 `REQUEST_CANCELLED` reject。
- provider 重启：连接型页面显示「provider 已重启，请重新打开这条连接」；
  普通页面 remount（重新读 `params`），不做「静默恢复」的假象。

## 9. 待办（不在本次范围）

- `.nshpkg` 签名包发布链路（`nsh publish` + 信任库轮换 + R2 分发）。
  宿主侧已同时支持「签名包」与「开发目录」两种形态，权限与 `harden` 语义一致。
