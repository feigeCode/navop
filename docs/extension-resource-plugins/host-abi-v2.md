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
  "connection": null           // 非连接型 app 为 null；连接型 app 见下
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

## 4. 调用（`host.call`）

所有调用都返回 Promise，失败以 `JsError` reject，页面按 `e.code` / `e.retryable` /
`e.details` 读取（`details` 是对象或 `undefined`）。

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

### 4.6 运行时

| 名字 | 参数 | 返回 |
|---|---|---|
| `navop.runtime.info` | `{ backend? }` | `{ backend, runtimeId, generation }` |

### 4.7 开发者工具（仅内置扩展）

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

- 可用模块：`resource` / `job` / `event` / `blob` / `runtime` / `log` / `dev`。
- `resource|job|event|blob` 仍要求 `backends` 非空。
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
- 页面关闭：宿主取消该 view 的所有进行中调用，Promise 以 `REQUEST_CANCELLED` reject。
- provider 重启：连接型页面显示「provider 已重启，请重新打开这条连接」；
  普通页面 remount（重新读 `params`），不做「静默恢复」的假象。

## 9. 待办（不在本次范围）

- `.nshpkg` 签名包发布链路（`nsh publish` + 信任库轮换 + R2 分发）。
  宿主侧已同时支持「签名包」与「开发目录」两种形态，权限与 `harden` 语义一致。
