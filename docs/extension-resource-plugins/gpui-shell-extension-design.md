# 基于 gpui-shell 的通用扩展机制设计

状态：设计稿  
日期：2026-08-31  
目标分支：`feat/universal-resource-plugins`

## 1. 结论

Navop 的新扩展机制采用两个严格分离的执行平面：

- **UI 平面**：gpui-shell 执行扩展脚本，生成可嵌入 Navop 的 `ScriptView`。
- **能力平面**：现有 headless provider runtime 通过 `resource/job/event/blob` 连接任意中间件。

两者只通过 Navop 注册的 `HostModule` 交互。provider 不返回 UI tree，gpui-shell
也不直接持有 provider session、GPUI Entity、保存凭据明文或任意 runtime id。

该方案不恢复已经删除的 declarative panel、`ViewSpec/UiNode`、WIT view 或 provider
UI RPC。已有 provider 协议保持不变，Elasticsearch、Kafka、Kubernetes、消息队列、
对象存储、SaaS API 和本地工具都继续以 namespaced domain method 运行在通用机制之上。

对 gpui-component fork 的 MVP 改动仍限制在 `crates/shell`，但不能只把现有私有函数
改成 `pub`。嵌入 Navop 前必须同时补齐：

1. 公开带显式 `Policy` 和 load options 的嵌入式 view load API。
2. 提供接收 GPUI `App` 的对称、幂等 loaded-view unload 生命周期。
3. unload 时撤销 per-policy HostModules，并让 retired application 的旧 frame 不再进入 VM。
4. 让异步 HostModule 可传播底层 cancellation。
5. 提供不重复初始化 `gpui-base` 的 embedded init 入口。
6. 直接复用上游 `gpui-component-shell` frozen registry，不维护 Navop 专用组件绑定。

不修改 `gpui-component` 控件，不让 gpui-shell 依赖 `gpui-component::Root`，不把 Navop
业务类型放入 fork。

## 2. 当前基线

### 2.1 已有 headless 底层

当前分支已经具备以下可复用能力：

- `ExtensionRuntimeCatalog` 解析并验证 `runtime.ipc`。
- `ActivationManager::activate_runtime` 为每个调用者分配独立 activation lease。
- 同一 runtime 的并发启动共享一个 process session。
- `StartClaimGuard` 清理被取消的初始启动。
- 最后一个 lease 释放后清理 process、job、event 和 blob registry。
- `RuntimeMonitor` 监督进程、generation 和有限重启。
- `replace_catalog` 与全局 revision 允许安装、卸载、更新后读取新 catalog。
- provider typed client 已覆盖 resource、job、event stream、provider blob 和 host blob。
- reverse Host API 已定义 secrets、storage、log、notify 和 host blob upload。
- `extension_view` 继续负责扩展安装、卸载、更新和权限展示。

当前所有权入口见：

- `main/src/universal_plugins.rs`
- `main/src/shell_plugin_host.rs`
- `crates/extension-plugin-adapter/src/activation.rs`
- `crates/extension-host/src/universal_plugin.rs`
- `crates/extension-runtime/src/global.rs`

### 2.2 gpui-shell 已有能力

固定 fork `e6459613910143b817eecbacc8767976e86cac84` 已合并最新上游并包含
`gpui_ce_components_shell`，crate 名为 `gpui_shell`。它已经提供：

- QuickJS ES module 执行和 `ScriptView`。
- 每个插件独立的 `Policy`。
- deny-by-default 的 `Capabilities`。
- 每个 `Policy` 独立的 `HostModule` registry。
- plain-data `HostValue` 边界。
- script task、application generation 和 hot reload 生命周期。

Navop 只保留上游尚未公开的嵌入生命周期薄层：显式 policy load、幂等 unload、异步取消和
embedded init。UI 组件、类型声明和 materializer 全部来自上游 `gpui-component-shell`。

### 2.3 当前不能直接使用 `PluginManager`

Navop 不应直接采用 gpui-shell 的 `PluginManager` 作为扩展目录和权限来源：

- Navop 已经以 `extension.json` 作为唯一安装、签名、权限和 catalog 来源。
- 再引入 `gpui-shell.json` 会产生 identity、version、entry 和 capability 双重真相。
- provider runtime 与 shell view 必须在同一 catalog revision 中完成交叉校验。
- 扩展管理页必须在不执行脚本的前提下展示完整贡献和权限。

因此 Navop 直接调用 policy-aware view loader，gpui-shell `PluginManager` 仅保留给
独立 `gpui-shell` 应用和开发工具。

## 3. 目标与非目标

### 3.1 目标

1. 一个扩展包可以只提供 UI、只提供 provider，或同时提供两者。
2. 一个 shell view 可以绑定零个、一个或多个 provider runtime。
3. provider 可以连接任意中间件，不需要修改公共协议枚举。
4. 多个 view mount 可以共享一个 provider process，但持有独立 lease。
5. 安装后的新 catalog 立即可用；更新/卸载通过短暂 drain transaction 后原子生效。
6. 每个 view 的脚本权限、backend、资源句柄和异步任务相互隔离。
7. view 关闭、加载失败、启动取消和 runtime restart 都有确定的清理语义。
8. Navop 可继续使用现有 TabContainer、Root、通知、凭据和连接管理能力。
9. gpui-component adapter 提供全部脚本 UI 组件；fork 只暴露 engine-neutral 的嵌入 API。

### 3.2 非目标

- 不支持 Rust dylib 作为第三方 UI 插件。
- 不允许 provider 向宿主发送任意 UI 描述。
- 不把 manifest permission 宣称为 native process 的 OS sandbox。
- MVP 不支持脚本直接使用 `ShellRoot` 的 dialog、sheet、toast 和 window overlay；脚本 UI
  统一使用 `gpui-component-shell` 提供的 `gpui-component` registry。
- MVP 不保证恶意脚本的进程级隔离；gpui-shell policy 是 API authority 隔离。
- MVP 不实现跨进程脚本 renderer，也不把 QuickJS 放入 provider process。
- 不让 shell view 直接访问任意 Navop 内部 crate 或数据库对象。

## 4. 架构

```text
extension.json
  |-- runtime.ipc[] -------------------------------+
  |-- contributes.shellViews[]                     |
  |                                                v
  +--> ExtensionRuntimeCatalog(revision N) --> ActivationManager
                 |                                  |
                 |                                  v
                 |                          ProcessRpcSession
                 |                                  |
                 |                      resource/job/event/blob
                 v                                  |
          ShellPluginHost <-------------------------+
                 |
                 | builds Policy + navop.* HostModules
                 v
            gpui-shell
                 |
                 v
             ScriptView
                 |
                 v
        ShellPluginTab / Navop Root
```

依赖方向必须保持：

```text
gpui-shell       --X--> extension-runtime / provider protocol / Navop UI
provider         --X--> gpui-shell / GPUI / TabContent
gpui-component   --X--> Navop domain types

main::ShellPluginHost --> gpui-shell
main::ShellPluginHost --> UniversalPluginService
main::ShellPluginHost --> Navop host services
```

`--X-->` 表示禁止依赖。

## 5. 扩展包模型

### 5.1 包布局

```text
com.example.elasticsearch/
  extension.json
  icon.svg
  ui/
    explorer.js
    sdk.js
  bin/
    elasticsearch-provider
```

Navop 只读取根目录的 `extension.json`。shell entry 是 contribution 的相对路径，
不要求另一个 `gpui-shell.json`。

### 5.2 支持的组合

| 扩展形态 | shell view | provider runtime | 示例 |
|---|---:|---:|---|
| UI-only | 是 | 否 | 本地计算器、格式化器、只用 localStorage 的工具 |
| provider-only | 否 | 是 | 被 Navop 内建 UI 或其他宿主功能消费的 driver |
| composite | 是 | 是 | Elasticsearch、Kafka、Kubernetes 管理器 |
| aggregator | 是 | 多个 | 同时聚合日志、指标和告警 provider |
| multi-view | 多个 | 一个或多个 | explorer、monitor、settings 各自独立页面 |

## 6. Manifest 设计

### 6.1 新 contribution

在 `ContributesManifest` 中增加强类型 `shellViews`，不复用当前 loose `views/tabs/forms`
字段，也不恢复 `declarativePanels`。

```json
{
  "schema_version": 1,
  "id": "com.example.elasticsearch",
  "name": "Elasticsearch",
  "version": "1.0.0",
  "engines": {
    "onetcli": ">=0.14.0",
    "gpui_shell": "0.2.0"
  },
  "api": {
    "extension": "1.0",
    "shell": "1.0"
  },
  "permissions": [
    "shell:exec",
    "spawn:./bin/elasticsearch-provider",
    "net:tcp:localhost:9200",
    "secrets:read:self.*",
    "notifications:show"
  ],
  "runtime": {
    "ipc": [
      {
        "id": "provider",
        "entry": { "command": "./bin/elasticsearch-provider" }
      }
    ]
  },
  "contributes": {
    "shellViews": [
      {
        "id": "explorer",
        "title": "Elasticsearch",
        "icon": "icon.svg",
        "entry": "ui/explorer.js",
        "surface": "tab",
        "singleton": false,
        "backends": {
          "search": "provider"
        },
        "modules": [
          "context",
          "connection",
          "resource",
          "job",
          "event",
          "blob",
          "log"
        ]
      }
    ]
  }
}
```

### 6.2 `ShellViewContrib`

建议字段：

```rust
pub struct ShellViewContrib {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub entry: String,
    pub surface: ShellSurface,
    pub singleton: bool,
    pub backends: BTreeMap<String, String>,
    pub modules: Vec<ShellHostModule>,
}
```

语义：

- `id`：扩展内唯一 contribution id。
- `entry`：相对 extension root 的 ES module 路径。
- `surface`：MVP 仅接受 `tab`；后续可增加 `sidebar` 和独立 window。
- `singleton`：相同 view key 是否只允许一个 mount。
- `backends`：脚本可见 alias 到同 manifest 中 local runtime id 的映射。
- `modules`：请求的 Navop HostModule 集合，不代表自动授权。

脚本永远看不到 namespaced runtime key。示例中的 `search` 在 catalog 注册时被解析为
`com.example.elasticsearch::provider`，HostModule 闭包只接受 alias `search`。

### 6.3 注册后的类型

`ExtensionRuntimeCatalog` 增加：

```rust
pub struct RegisteredShellViewContribution {
    extension_id: String,
    extension_version: String,
    view_key: String,
    title: String,
    description: Option<String>,
    icon_path: Option<PathBuf>,
    extension_root: PathBuf,
    entry_path: PathBuf,
    surface: ShellSurface,
    singleton: bool,
    backends: BTreeMap<String, String>,
    modules: BTreeSet<ShellHostModule>,
    permissions: Vec<String>,
    shell_api_version: String,
    required_gpui_shell_version: String,
}
```

`view_key` 固定为 `extension_id::view_id`。所有路径在注册阶段完成 canonicalize 和
extension-root containment 检查；运行时加载前再次检查最终文件仍在 root 内，避免
symlink 替换绕过安装时校验。

### 6.4 校验规则

1. `id` 和 backend alias 只允许小写 ASCII、数字、`.`、`-`、`_`。
2. `entry` 必须是相对路径，禁止 `..`、盘符、UNC 和绝对路径。
3. `entry`、icon 和导入 root 必须位于当前扩展目录。
4. backend alias 不得重复，也不得使用 `host`、`navop`、`default` 等保留名。
5. backend 目标必须指向同一 manifest 的 `runtime.ipc[].id`。
6. `modules` 必须来自宿主已知 allowlist。
7. 请求 resource/job/event/blob 时至少声明一个 backend。
8. `api.shell` 表示 `navop.*` HostModule contract 版本；resource、job、event、blob 和
   connection 的脚本 facade 在该 contract 下整体版本化，provider wire 继续使用
   `api.extension`。
9. `engines.gpui_shell` 使用 gpui-shell 的 pre-1.0 minor compatibility 规则校验。
10. 只要存在 `shellViews`，manifest 就必须显式声明高危权限 `shell:exec`。
11. 新增的 `ShellViewContrib`、backend/module 子结构使用 `deny_unknown_fields`；当前旧
    manifest 顶层 parser 仍会忽略未知字段，若要全局 fail closed 必须作为独立 schema
    migration 处理，不能在本改动中静默改变兼容性。

### 6.5 命令和菜单入口

现有 command contribution 增加 `shell_view` handler：

```json
{
  "id": "elasticsearch.openExplorer",
  "title": "Open Elasticsearch Explorer",
  "handler": {
    "kind": "shell_view",
    "viewId": "explorer"
  }
}
```

menu、toolbar 和 keybinding 继续引用 command id。它们只触发宿主打开 view，不直接
执行脚本函数，避免全局 command 在没有 mount/policy 的情况下拥有脚本 authority。

## 7. 权限模型

### 7.1 三层 authority

扩展权限必须分三层理解：

1. **安装权限**：`extension.json.permissions`，由 `extension_view` 展示和审批。
2. **provider 权限**：进程启动、`resource/open`、secret resolve 等 host preflight。
3. **script 权限**：gpui-shell `Policy` 和 per-policy HostModule registry。

三层不能互相替代。特别是 provider 有 `net:*` 不代表脚本可直接发 HTTP 请求。

UI 脚本和同一安装包中的 native provider 是同一个扩展安全主体，不是互不信任的两个
sandbox。Host 可以防止普通 API 意外返回已保存 secret，但无法阻止恶意 provider 将
解析到的 secret 改名、编码后通过 inline/blob/event 返回 UI。安装审批、签名和来源信任
必须覆盖整个扩展包。

### 7.2 默认 script policy

Navop 为每个 shell view 创建独立 `Policy`：

```text
application  = extension_id/view_id/mount_id
storage      = true，仅对应宿主分配的隔离 store.json
filesystem   = denied
network      = denied
execute      = denied
clipboard    = denied
exit         = denied
ui catalog   = gpui-component-shell
host modules = 经过 contribution + permission 交集后注册
```

扩展脚本连接中间件必须通过 provider，不把 `net:*` 映射为 gpui-shell direct network。
这保证连接审计、secret 注入、timeout、取消、blob 和 generation 语义都只有一套。

UI 组件不再通过 Navop 专用 UI HostModule 或第二套组件协议表达。shell view 统一导入
`gpui-component-shell` 注册的 `gpui-component` 模块；provider 数据访问仍通过显式声明的
Navop HostModule。`shell:exec` 安装审查覆盖脚本 UI 在 Navop 进程内运行的整体风险。

### 7.3 HostModule 授权

HostModule 注册集合是以下三者的交集：

```text
contribution 请求的 modules
∩ 当前宿主支持的 api.shell 版本
∩ manifest permissions 允许的操作
```

例如：

- `connection` 保存 secret 需要精确 extension namespace。
- `database` 读取 Navop 连接需要 `db:connections:list` 和对应 `db:read:*`。
- backend resource 的 endpoint descriptor 由 `StoredConnection` 或标准 metadata 提供，
  provider permission authorizer 进行 defense-in-depth preflight。

不新增“每个函数一个 permission string”。模块是粗粒度 capability，具体调用继续由
参数和已有 permission authorizer 做细粒度判定。

`self` 是 secret permission 和 secret reference 中的虚拟 namespace。Host 将 extension id
按 UTF-8 bytes 编码成 `ext_<lowercase-hex>` 内部 namespace，例如不同 publisher 的同名
扩展也不会冲突；该编码可逆且只使用当前 identifier 允许的字符。provider 使用
`secret://self/<key>`，`UniversalProviderHost` 必须同时持有 extension id 并在访问凭据库前
替换成内部 namespace。没有额外显式共享权限时，其他 namespace 一律拒绝。

### 7.4 风险披露

`shellViews` 必须同时满足两个门禁：manifest 显式声明现有高危权限 `shell:exec`，并在
安装确认页显示固定的“在 Navop 进程内执行脚本 UI”披露。parser 在 contribution 存在
但权限缺失时直接拒绝，因此 downloader、自动安装和其他入口都不能绕开风险确认。

gpui-shell policy 可以限制脚本能调用的 API，但不能替代 VM 漏洞、CPU 占用和 native
provider 的 OS sandbox。来源不可信的扩展仍需签名、仓库信誉和后续进程隔离策略。

## 8. gpui-shell fork 的最小 API

### 8.1 MVP unblocker

新增 builder 风格的嵌入 options，再将现有 policy-aware load 逻辑提升为
`ShellRuntime` 公共方法。options 需要关闭向签名/只读扩展目录写入 `gpui.d.ts`：

```rust
impl ShellRuntime {
    pub fn load_view(
        self: &Rc<Self>,
        options: ViewLoadOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<LoadedScriptView>;
}

let options = ViewLoadOptions::new(root, entry, policy)
    .write_type_declarations(false);
```

`LoadedScriptView` 是 opaque handle：

```rust
pub struct LoadedScriptView { /* private */ }

impl LoadedScriptView {
    pub fn view(&self) -> &Entity<ScriptView>;
    pub fn unload(&mut self, cx: &mut App);
    pub fn is_unloaded(&self) -> bool;
}
```

内部持有当前 `Entity<ScriptView>`、`Policy`、application generation 和 runtime。
`unload` 必须：

1. 通过 `cx` 将 root `ScriptView` 标记 retired；
2. retire application generation；
3. cancel view/application task 和 policy-owned owner-less task；
4. 撤销该 Policy 的 HostModule registry，释放闭包捕获的 mount/window/entity；
5. 使旧 GPUI frame 暂时保留的 entity 不再进入 VM；
6. 幂等。

`ScriptView::render/rebuild` 还应检查 application generation liveness，作为旧 frame 仍持有
entity 时的第二道保护。`Drop` 只能做无上下文的 generation/task/module 撤销兜底，不能
宣称等价于显式 `unload(cx)`。

### 8.2 不公开的内部

以下内容继续保持 crate-private：

- QuickJS runtime/context、`Persistent` 和 `ViewObject`。
- `load_app`、`instantiate_view_with_policy` 和 scope frame。
- `ApplicationGeneration`、scheduler registry 和 raw cancel 函数。
- `SpecArena`、materializer、callback arena 和 module resolver。
- GPUI Entity 的脚本桥接表示。

Navop 只依赖 `ShellRuntime`、`Policy`、`ViewLoadOptions`、`HostModule`、`HostValue`、
`ScriptView`、`LoadedScriptView` 和 `gpui-component-shell` 的 frozen registry。

### 8.3 不修改 `gpui_component::Root`

Navop 主窗口的第一层 view 继续是 `gpui_component::Root`。MVP 中嵌入的 `ScriptView`
不使用 `ShellRoot` overlay API；toast/notify、confirm/dialog、表单和其他 UI 统一使用
`gpui-component-shell` registry，由现有 Root 承载。

### 8.4 可取消 HostModule task

当前 `HostModule::async_function` 只会在 owner 消失时停止 JS continuation，后台 future
仍会继续运行。Navop 的 resource open/invoke、job start 和写操作不能使用这一语义。

在 shell crate 增加通用、engine-neutral 的可取消任务类型：

```rust
pub struct HostAsyncTask {
    future: HostFuture,
    cancel: Arc<dyn Fn() + Send + Sync>,
}

HostModule::cancellable_async_function(name, |arguments| -> Result<HostAsyncTask, HostError>);
```

QuickJS scheduler 将 `cancel` 安装到现有 task cancellation slot。view unload、application
retire 或 policy cancel 时，先触发底层 cancel，再丢弃 script continuation。

这不是 Navop 专用 API；任何 embedder 的异步 HostModule 都可以用它停止网络、数据库或
子进程工作。普通 `async_function` 保持现有兼容语义。

## 9. Navop 宿主对象

### 9.1 `ShellPluginHost`

扩展现有 GPUI Global：

```text
ShellPluginHost
  shell_runtime: Rc<ShellRuntime>
  universal_plugins: UniversalPluginService
  host_services: Arc<ExtensionHostServices>
  mounts: BTreeMap<ShellMountId, ShellMountRecord>
  catalog_revision: u64
```

职责：

- 从当前 catalog snapshot 查找 `RegisteredShellViewContribution`。
- 激活 contribution 声明的 backend。
- 构造 mount-scoped session、Policy 和 HostModules。
- 调用 gpui-shell public loader。
- 将 view 包装成 Navop TabContent 或其他 surface。
- 订阅 runtime health 和 catalog revision。
- 在 close、cancel、load failure、update 和 quit 时清理 mount。

### 9.2 `ShellPluginMount`

每次打开生成唯一 `ShellMountId`，状态机如下：

```text
Discovered
   -> ActivatingBackends
   -> LoadingScript
   -> Ready
   -> Closing
   -> Closed

任一步骤 -> Failed -> Closing -> Closed
```

mount record 固定持有打开时的 contribution snapshot，避免 catalog 替换后 title、entry、
backend alias 和 permission 在同一 view 生命周期中发生漂移。

### 9.3 `ShellPluginTab`

MVP surface 使用一个 Navop wrapper，而不是要求 `ScriptView` 实现业务 trait：

```text
ShellPluginTab
  metadata/title/icon/focus_handle
  state: Loading | Ready(LoadedScriptView) | Failed
  session: Arc<ShellMountSession>
```

wrapper 实现 `Render + Focusable + EventEmitter<TabContentEvent> + TabContent`。Ready 状态
的直接布局边界必须包含：

```text
size_full + min_w_0 + min_h_0 + overflow_hidden
```

避免脚本内容的 intrinsic size 反向挤压 TabContainer 和窗口 chrome。

不能假设所有 tab 移除都先调用 `TabContent::try_close`。`ShellPluginHost` 订阅
`TabContainerEvent::TabClosed`，`ShellPluginTab::Drop` 还必须取消 mount root token 并调度
幂等 cleanup。正常路径仍显式 `unload(cx)`；强制清空、窗口销毁或恢复布局移除时，
`LoadedScriptView::Drop` 的 policy/generation 撤销和 `MountRuntimeLease::Drop` 的 Tokio
释放构成最后兜底。

### 9.4 mount key

- singleton view：`shell:<view_key>`。
- multi-instance view：`shell:<view_key>:<context-key>:<nonce>`。
- `context-key` 由宿主根据 `StoredConnection`、resource target 或调用入口生成，脚本不能
  用任意字符串覆盖另一个扩展的 tab identity。

## 10. Backend 激活与 lease

### 10.1 打开顺序

1. 读取原子的 `(catalog revision, catalog Arc)` snapshot。
2. 校验 view contribution 和 surface。
3. 分配 mount id/cancellation token，将 wrapper 放入 Loading 状态。
4. 在 Navop Tokio runtime 中并发激活全部 backend alias。
5. 任一激活失败时释放已取得的 lease，不加载脚本。
6. 构造 `ShellMountSession` 和 HostModules。
7. 在 GPUI foreground 中构造 `Policy` 并加载 `ScriptView`。
8. 若脚本加载失败，先 unload shell handle，再关闭 mount handles，最后释放 lease。
9. 只有 mount id 和 attempt generation 仍匹配时才安装 Ready view。

第 9 步防止“旧加载完成覆盖已关闭或重新打开的新 mount”。

每个 mount 有一个 root cancellation token，每次 HostModule call 派生 child token。
token 同时接入 gpui-shell `HostAsyncTask.cancel` 和 extension-host `RequestOptions.cancel`。

### 10.2 lease 语义

每个 mount 对每个 backend 获得一个独立 `ActivationHandle`：

- 两个 tab 使用同一 provider 时共享 process，但各自可独立关闭。
- runtime restart 不改变 activation id；已有 resource/client handle 按 generation 失效。
- 关闭旧 tab 的 stale lease 不能释放新 tab 的 activation。
- 最后一个 mount lease 释放后底层负责清理 job/event/blob registry。

在 main 层封装 `MountRuntimeLease`：

- `close().await` 调用 `deactivate_activation`。
- 状态原子地从 Open 变为 Released，保证只释放一次。
- `Drop` 使用应用 Tokio handle 调度幂等释放，作为没有走 `try_close` 的兜底。
- 应用正常退出仍由 `UniversalPluginService::shutdown` 统一等待。

`Drop` 调度只负责 provider lease 兜底。shell view 的强清理由 TabContent `try_close`、
extension retire 和应用 shutdown 显式调用 `LoadedScriptView::unload(cx)` 完成。

### 10.3 关闭顺序

```text
mark Closing
-> cancel mount-owned HostModule work
-> unload LoadedScriptView(cx) / revoke HostModules and policy tasks
-> close resource/event/job/blob handles (bounded best effort)
-> release activation leases
-> remove mount record
```

不能先释放 provider lease 再让脚本 task 停止，否则迟到 continuation 可能在关闭中的
session 上继续发请求。

对 provider RPC 的取消要求：

- extension-host 为所有 typed method 增加 `*_with_options` 或等价 cancellable facade，
  不只 event read 支持 `RequestOptions`。
- cancel 触发 `$/cancelRequest` 并移除 pending request。
- provider 必须在 response 发布前使取消后的 open/start 不留下不可达 side effect；若已
  创建资源，provider 自行回滚。
- reference providers 必须实现 cancellation notification，不能像当前 Elasticsearch
  示例一样忽略。
- 不合作的 provider 只能依赖最终 process shutdown 兜底；当同 runtime 仍被其他 lease
  使用时，宿主无法可靠回收一个连 id 都未返回的远端资源，这属于 provider contract
  violation，必须记录审计日志。

还必须处理“cancel 与成功 response 同时到达”的竞态：

- client select 改为 response-priority；两者同时 ready 时先交付成功 response，让 Host
  获得 resource/job/stream id 并执行正常 close。
- `RequestOptions` 对 side-effectful create call 可注册 bounded late-response compensator。
  cancel 后 pending entry 在短 grace 内变为 tombstone；reader 收到迟到成功 response 时，
  将 raw result 交给 compensator，而不是静默丢弃。
- resource open 的 compensator 解析 id 后 close；job start 执行 cancel + close；event open
  执行 close。补偿失败进入审计和最终 runtime cleanup。
- 无法补偿的写 invoke 在请求越过 transport 后取消，结果必须报告 `OUTCOME_UNKNOWN`，
  不能虚假返回“已取消且未执行”。插件若需安全重试，必须提供 operation id 或业务幂等键。

上述改动位于 extension-host client/adapter，不新增 provider wire method，也不恢复 UI
protocol。

tombstone registry 必须有显式上限，例如每 session 最多 128 个、grace 5 秒，并计入
runtime health/metrics。达到上限时不再接受“立即返回取消”的 side-effectful create call：
caller 在 bounded timeout 内等待 response 并补偿，超时返回 `OUTCOME_UNKNOWN`。禁止为
保持取消表面及时而无界增长 registry 或直接丢弃迟到成功。

### 10.4 长任务所有权

job 默认属于 mount。view 关闭时 cancel/close job；最终 lease 清理是兜底。

需要在 tab 关闭后继续运行的 job 必须显式转交给未来的 `BackgroundJobService`，该服务
取得自己的 runtime lease。禁止只把 job id 放入全局 map 而不转移 lease。

## 11. HostModule 设计

### 11.1 模块列表

推荐稳定模块：

| specifier | 主要职责 |
|---|---|
| `navop.context` | extension/view/mount、locale、theme、backend alias 和启动 context |
| `navop.runtime` | backend health、generation 和 capability introspection |
| `navop.resource` | open、ping、invoke、close |
| `navop.job` | start、status、cancel、result、close |
| `navop.event` | open、read、close；SDK 封装为 async iterator |
| `navop.blob` | open/read/close、text/base64 helper |
| `navop.log` | 带 extension/view/mount 字段的结构化日志 |
| `navop.database` | 可选的 Navop 内建数据库 facade，受 `db:*` 权限约束 |

每个 policy 只注册 contribution 请求且 permission 允许的模块。

### 11.2 禁止跨边界的对象

HostModule 不得返回或接收：

- `ProcessRpcSession`、`JsonRpcClientHandle` 或 trait object。
- GPUI `Entity`、`Window`、`App` 或 Rust closure。
- 原始 child process handle。
- 任意 namespaced runtime id。
- Navop connection/secret API 返回的已保存 secret 明文。
- 未分块的大型二进制或超出约束的大 JSON。

同包 native provider 仍能看到其被授权解析的 secret，并可能通过业务结果返回。该风险由
整包信任和签名模型约束，不能靠 HostValue 类型系统消除。

### 11.3 mount-scoped handle registry

脚本可见句柄不是 provider 返回的原始 id。`ShellMountSession` 为每种句柄维护映射：

```text
shell resource handle -> backend alias + runtime generation + provider resource id
shell job handle      -> backend alias + runtime generation + provider job id
shell event handle    -> backend alias + runtime generation + provider stream id
shell blob handle     -> backend alias + runtime generation + provider/host blob id
```

句柄包含不可预测 nonce，并在每次调用时检查：

- mount id 相同；
- handle type 相同；
- backend alias 属于 contribution；
- runtime generation 仍匹配；
- handle 尚未关闭。

脚本伪造、跨 tab 复制或使用旧 generation handle 时返回 `STALE_HANDLE` 或
`INVALID_HANDLE`，不能把字符串直接透传给 provider。

### 11.4 resource API

建议 TypeScript surface：

```ts
export function open(
  backend: string,
  resourceType: string,
  request: {
    config?: unknown;
    endpoints?: EndpointDescriptor[];
    metadata?: unknown;
  },
): Promise<{ handle: string; capabilities: string[]; metadata?: unknown }>;

export function invoke(
  handle: string,
  method: string,
  params?: unknown,
): Promise<ResultRef>;

export function ping(handle: string): Promise<void>;
export function close(handle: string): Promise<void>;
```

domain method 继续使用 `elasticsearch/search`、`kafka/topic/list` 等 namespaced string，
公共 Rust protocol 不新增领域 enum。

保存连接由 Host 从标准 `StoredConnection` 生成公开 config、secret reference 和 endpoint
descriptor，在加载 connection-scoped shell view 前完成 `resource/open`，再通过
`navop.context.current().connection.resource` 注入 mount-scoped opaque handle；script 不获得
保存过的 config 或 secret。未保存表单的测试连接使用瞬态 secret reference。Host 将标准 endpoint descriptors 放入现有
`ResourceOpenParams.metadata`，不改变 provider wire struct。

现有 `ResourceOpenAuthorizer` 对 `url/server/brokers/host+port` 的硬编码提取只作为 legacy
fallback。通用路径优先读取标准 descriptor，至少支持 TCP、UDP、HTTP(S)、Unix socket
和 host-defined tunnel endpoint。`net:tcp:*:<range>` 的 `*` 必须实现为显式高危 wildcard
匹配，而不是普通主机字符串。

endpoint preflight 是 defense in depth，不是 native provider 的网络 sandbox。provider
与脚本属于同一扩展安全主体，真正强隔离需要未来的 OS sandbox。

标准 metadata schema：

```json
{
  "navopEndpoints": {
    "version": 1,
    "items": [
      { "kind": "tcp", "host": "localhost", "port": 9200 },
      { "kind": "udp", "host": "127.0.0.1", "port": 8125 },
      { "kind": "http", "scheme": "https", "host": "example.com", "port": 443 },
      { "kind": "unix", "path": "/var/run/example.sock" },
      {
        "kind": "tunnel",
        "tunnelId": "host-owned-id",
        "target": { "kind": "tcp", "host": "db.internal", "port": 5432 }
      }
    ]
  }
}
```

规范化和 permission 映射：

- DNS host 转小写并执行 IDNA canonicalization；IPv4/IPv6 使用标准文本格式，IPv6 不把
  冒号当 permission 分隔符解析。
- `tcp` 和 `http` 映射 `net:tcp:<host>:<port-range>`；`udp` 映射
  `net:udp:<host>:<port-range>`。
- IPv6 permission 使用 bracketed host，例如 `net:tcp:[::1]:5432`；parser 从右侧解析
  port range，再对 bracketed host 做标准化。
- 新增 `net:unix:<path>`，沿用 fs path 的 absolute/`~/`/`${user_pick}` 和 escape 校验，
  但单独表示 connect authority，不用 fs read/write 冒充 socket 权限。
- `tunnel` 同时要求 `host:ssh_tunnel` 和 target 的 network permission；脚本只能引用
  Host 创建的 opaque tunnel id。
- wildcard `*` 仅可出现在 host permission，继续标记 High risk；descriptor 本身必须是
  具体 canonical endpoint。
- 存储 profile 时 Host 生成 descriptor；临时 config 的 descriptor 做结构校验，但 native
  provider 仍可能绕过，因此它不是 OS enforcement。

### 11.5 `ResultRef`

脚本侧保留和 provider 一致的显式结果类型：

```ts
type ResultRef =
  | { kind: "inline"; value: NavopValue }
  | { kind: "blob"; handle: string }
  | { kind: "event_stream"; handle: string };
```

Host 只在明确的 `ResultRef` 字段中创建 blob/event handle，不扫描任意 JSON 猜测 id。

### 11.6 JSON 精度

`HostValue::Number` 是 `f64`，不能无损表示所有 `i64/u64`。任意中间件常使用 64 位
offset、timestamp、Snowflake id，因此不能简单执行 `serde_json::Value -> HostValue`。

定义 `NavopValue` 编码：

```ts
type NavopValue =
  | null | boolean | number | string
  | NavopValue[]
  | { [key: string]: NavopValue }
  | { $navop: "i64" | "u64" | "decimal"; value: string };
```

- JS safe integer 范围内使用 number。
- 超出范围的整数使用 tagged decimal string。
- SDK 可选择转换为 `BigInt`，但原始 tagged value 始终可无损 round-trip。
- 输入同样接受 tagged value，再转换回 `serde_json::Number`。

### 11.7 二进制

二进制只通过 blob：

- `read` 默认 256 KiB，最大 4 MiB，复用现有协议限制。
- chunk 以 Base64 返回，SDK 可转换为 `Uint8Array`。
- 提供有界 `readText` 和 `readJson` helper；超过 limit 拒绝，不隐式读完整 blob。
- close 幂等，mount 关闭时回收尚未关闭的 handle。

### 11.8 event stream

JS SDK 用 pull API 构造 async iterator：

```ts
for await (const batch of events.read(handle, { maxEvents: 128, waitMs: 1000 })) {
  // batch.events, batch.droppedCount, batch.closed
}
```

底层保持有界 buffer。view unload 后 shell promise continuation 不再恢复，root
cancellation token 中断 event read 并关闭 provider stream，避免后台读取永久保留
runtime lease。

## 12. Tokio 与 GPUI executor 边界

`HostModule::async_function` 的 future 由 gpui-shell 放到 GPUI background executor。
provider client 和数据库 future 使用 Tokio mutex、socket、timer 和 channel，不能直接在
该 executor 上轮询。

正确桥接：

1. HostModule 同步 closure 在 script call scope 中解析参数、读取 mount session。
2. 从 App 取得 Navop Tokio handle，或创建 `Tokio::spawn_result` 对应任务。
3. 真正的 provider/db future 完全运行在 Tokio runtime。
4. Tokio task 使用 call child cancellation token 构造 `RequestOptions`。
5. HostModule future 只等待 Tokio join/result bridge，不自行创建 Tokio timer/socket。
6. `HostAsyncTask.cancel` 先取消 token，让 Tokio task 有机会发送 `$/cancelRequest` 和执行
   补偿清理；超过 bounded grace 后才 abort task。结果转换为 `HostValue` 后由 gpui-shell
   恢复原 policy 和 view owner。

禁止：

```text
HostModule future on GPUI background executor
  -> direct await UniversalPluginClient
  -> tokio::time / socket / channel
```

这会重现“没有 Tokio reactor/runtime”的崩溃路径。

## 13. 连接、secret 与宿主服务

### 13.1 统一 `ExtensionHostServices`

在 main 层建立应用所有的服务集合，同时供 provider reverse Host API 和 shell
HostModules 使用：

```text
ExtensionHostServices
  secrets
  storage
  notifier
  logger
  blob_store
  optional database facade
```

这不是 gpui-shell API，也不进入 gpui-component fork。

### 13.2 secret 原则

- Navop connection/secret HostModule 从不返回保存过的 secret 明文。
- manifest 表单由 Host 渲染并保存到标准 `ConnectionRepository`。
- shell view 只收到连接 identity 和 Host 已打开的 mount-scoped resource handle。
- provider config 携带 secret reference；provider 通过现有 `host/secret/resolve` 获取。
- extension/provider 对外只使用虚拟 namespace `self`。Host 以
  `ext_<lowercase-hex(extension-id)>` 生成全局唯一内部 namespace，不接受脚本指定任意
  storage namespace。
- Host 自己生成的日志和错误做已知 secret redaction；不能宣称可可靠识别 provider 对
  secret 的任意改名、分片或编码。

UI 和 provider 是同一扩展安全主体。恶意 provider 可以有意把 secret 放入
inline/blob/event，通用宿主无法从任意业务数据中恢复 taint。因此安全承诺是“Navop API
不直接泄漏保存 secret”，不是“同包 provider 永远无法把 secret 送回脚本”。

### 13.3 统一连接持久化

扩展不建立独立 profile repository。所有连接统一保存为 `StoredConnection`，类型为
`ConnectionType::Extension`；`params` 保存 extension/contribution identity、公开 config 和
加密 secret map。工作区、同步、编辑、复制、删除、最近使用和活跃连接保护全部复用宿主
现有连接机制。manifest 的 `shellViewId` 可选，不影响连接持久化模型。

### 13.4 补齐已有 reverse Host API

当前 production 中 secret resolver 为空，notify/storage/log 是 stub。接入 shell 前应将
它们替换为上述真实服务，否则 provider E2E 与应用真实行为不一致。

host blob upload 保持当前实现，provider 可将大结果写入宿主 store，再通过统一
`ResultRef::Blob` 被 shell 读取。

## 14. catalog 热替换

### 14.1 原子 snapshot

`GlobalExtensionRuntimeCatalog` 应提供一次锁内读取的：

```rust
pub struct ExtensionCatalogSnapshot {
    pub revision: u64,
    pub catalog: Arc<ExtensionRuntimeCatalog>,
}
```

revision 与 catalog 不能分别读取。consumer 只接受比本地更新的 revision，避免并发
sync 将 manager 回退到旧 catalog。

### 14.2 runtime binding 固定

活动 runtime 必须持有激活时的完整 `RegisteredIpcRuntimeBinding` 或等价 immutable
snapshot。monitor、restart budget 和 process restart 不能依赖“当前 catalog 仍有该 key”。

`ActivatedRuntime` 固定持有 activation 时的 binding snapshot。`universal_plugin_client` 的
permission authorizer、monitor、restart budget 和 process restart 都读取该 snapshot，
不能在中途切到新 catalog 的同名 binding。

MVP 不支持同一个 runtime key 的新旧 process incarnation 并存。更新/卸载采用 drain
transaction：

1. 将 extension 标记 `Retiring`，拒绝新 mount 和新 runtime activation。
2. 原子 activation barrier 取消/拒绝已经通过外层检查但尚未返回 lease 的 start claim。
3. 关闭该 extension 的 shell mounts，并通知其他 runtime consumer 释放 lease。
4. 同时等待 returned lease 数和 in-flight start claim 数归零；超时后由用户选择取消更新
   或强制停止。
5. 停止旧 runtime，完成 job/event/blob 清理。
6. 原子发布新 catalog 或删除后的 catalog。
7. 解除 `Retiring`；之后的新 mount 才能使用新 binding。

activation barrier 必须位于 `ActivationManager` 内部，而不是只在 `ShellPluginHost` 外层
检查：`begin_activation` 在同一 state lock 下检查 extension 未 retiring、增加
`inflight_starts` 并返回 guard；guard 在成功、失败、取消和 panic unwind 时递减并 notify。
retire 会触发每个 start claim 的 token，session factory 通过 `select` 响应取消，现有
`StartClaimGuard` 继续负责删除没有 session 的 `Starting` runtime。

这样不需要在 `runtime_id` map 中引入复合 incarnation key，也不会让新 mount 错误共享旧
process。安装一个此前不存在的 extension 不需要 drain，可直接发布新 catalog。

### 14.3 shell contribution 固定

普通运行期间 mount 固定打开时的 contribution snapshot，不在 render 过程中偷换
entry、backend 或 permission。生产 update/uninstall 会先关闭旧 mount，再发布新 catalog；
不存在旧 mount 和同 key 新 mount 并行使用不同 binding 的状态。

事件处理：

- **安装**：新 view/command 立即可发现和打开。
- **更新**：进入 Retiring 后拒绝新 mount，关闭旧 mount，切换 catalog 后重新允许打开。
- **卸载**：进入 Retiring 后拒绝新 mount，关闭旧 mount和 runtime，再删除并发布 catalog。

### 14.4 文件生命周期

卸载/更新不能先删除活动 mount 仍需使用的目录。推荐安装布局采用版本目录和原子
active pointer：

```text
extensions/composite/<id>/versions/<version-or-digest>/...
extensions/composite/<id>/active.json
```

版本目录用于 stage、原子切换和失败回滚。MVP 不用它维持同 key 新旧 mount 并行；旧
版本在 drain 完成和新 catalog 发布后垃圾回收。

若第一阶段不实现版本目录，则卸载必须先请求关闭该 extension 的所有 mount/runtime，
等待 bounded shutdown 后再删除文件；不能保留“先删目录、后刷新 catalog”的顺序。

### 14.5 script reload

生产更新默认 close/reopen，不自动执行脚本 migration hook。gpui-shell 明确采用宿主任务
取消而非第三方 `deactivate()`，因此不要新增不可靠的脚本 teardown 回调。

MVP 的开发模式也使用 close/reopen，不额外公开只接受 `ShellRoot` 的 watcher 内部 API。
若后续确需原位刷新，再单独设计接受 `LoadedScriptView` 的 public watch seam。

## 15. runtime restart

provider restart 后 activation lease 仍有效，但 generation-bound handle 全部失效：

1. `RuntimeMonitorEvent` 通知 `ShellPluginHost`。
2. mount session 将旧 resource/event/blob handle 视为 stale。
3. Host 卸载当前 shell view，显式 close mount resources 并释放 activation。
4. 页面提示用户关闭并重新打开连接，Host 在新 mount 上重新执行 `resource/open`。
5. HostModule 对旧 handle 返回结构化 `STALE_HANDLE`，不自动重放写操作。

只允许显式声明为安全、幂等的 read operation 由 SDK 提供可选 retry helper。Host 不根据
method name 猜测幂等性。

job recovery 不能只依赖 host registry 改 generation。Phase 0 必须停止当前无条件
`recover_generation` 行为：provider 若通过 negotiated feature 和 domain resume token 明确
支持恢复，才迁移 job；否则旧 job 进入 failed/stale，由 UI 提示重新执行。

## 16. 错误模型

gpui-shell `HostError` 当前只携带 message，而且 runtime 会在外层增加
`` `module.function`: ``。MVP 将结构化错误编码为可搜索 marker 和 Base64URL JSON：

```text
__NAVOP_ERROR__<base64url-json>
```

SDK 解析为：

```ts
class NavopError extends Error {
  code: string;
  retryable: boolean;
  details?: NavopValue;
}
```

SDK 在完整 exception message 中搜索最后一个 `__NAVOP_ERROR__` marker，不假设 marker
位于开头；解码失败则保留普通 JavaScript Error。message/details 进入 envelope 前做长度
限制，避免通过异常构造无界数据。

稳定 code 至少包括：

- `INVALID_ARGUMENT`
- `PERMISSION_DENIED`
- `BACKEND_NOT_FOUND`
- `BACKEND_START_FAILED`
- `RUNTIME_UNAVAILABLE`
- `INVALID_HANDLE`
- `STALE_HANDLE`
- `REQUEST_CANCELLED`
- `REQUEST_TIMEOUT`
- `RESULT_TOO_LARGE`
- `PROTOCOL_ERROR`
- `EXTENSION_UNLOADED`

如果未来 gpui-shell 将 `HostError` 扩展为 code/details，Navop 可移除字符串 envelope，
provider wire protocol不需要变化。

## 17. UI 集成

### 17.1 Loading 和失败状态

script 在 backend 激活完成后才加载。wrapper 使用 Navop 原生状态页展示：

- 启动 provider；
- 加载脚本；
- 权限拒绝；
- provider 启动失败；
- script link/construct/render 失败；
- runtime crash loop；
- extension 已卸载或需要重新打开。

不能让一个未初始化的脚本负责展示自己的启动失败。

### 17.2 焦点和快捷键

- wrapper 拥有 TabContent focus handle。
- Ready 后将焦点委托给 `ScriptView` 内部 GPUI focus tree。
- Navop 全局 shortcut 先于脚本局部事件处理。
- 扩展不得动态注册任意全局 keybinding；只能通过已验证 command contribution。

### 17.3 dialog 和 notify

dialog、sheet、notification 和其他 UI 统一使用 `gpui-component` module。Navop 主窗口已由
`gpui_component::Root` 承载。窗口关闭后的脚本 view 由
`LoadedScriptView::unload` 统一 retire，并取消其 HostModule task。

## 18. extension_view 集成

扩展管理页增加以下静态信息，不执行脚本：

- shell view 数量、title、surface 和 entry。
- 绑定的 backend alias/runtime。
- 请求的 HostModules。
- “进程内脚本 UI”固定风险披露。
- provider process、network、secret、db 和 host permissions。
- 当前活动 mount 数和 update/reopen 状态。

卸载流程：

```text
request uninstall
-> catalog/command 先标记 retiring，拒绝新 mount
-> ShellPluginHost close extension mounts
-> UniversalPluginService release/retire runtimes
-> delete or GC version directory
-> publish new catalog revision
-> refresh extension_view
```

更新流程先 stage 和完整校验新版本，成功后再切 active snapshot；失败不能破坏旧版本。

## 19. Elasticsearch 验证扩展

Reference implementation lives in
`../navop-extensions/extensions/composite/elasticsearch`. Its provider remains
headless and uses the official Rust `elasticsearch` SDK; the shell view only does:

1. 通过 Navop 标准新建连接窗口创建或编辑 `StoredConnection`。
2. Host 通过 backend alias `search` 打开 `elasticsearch` resource，shell view 从
   `navop.context` 读取 opaque handle。
3. 调用现有 `elasticsearch/cluster/info`、index 和 search domain method。
4. 大结果使用 blob，async search 使用 job，持续结果使用 event stream。
5. provider crash/restart 后提示重新连接并重建 resource handle。

该扩展同时验证“一个 UI + 一个任意中间件 provider”的完整路径，但不成为公共协议的
Elasticsearch 特例。

## 20. 测试策略

### 20.1 gpui-shell fork

1. explicit-policy view load 使用传入 policy，而不是 default policy。
2. 两个 view 的 HostModule registry 和 storage 不互相可见。
3. load/link/construct 失败会取消该 policy 创建的 task。
4. `LoadedScriptView::unload(cx)` 幂等，撤销 HostModules 并取消 owner-less task。
5. generation retired 后，即使旧 frame 保留 entity 也不再进入 VM。
6. `HostAsyncTask.cancel` 在 unload 时触发底层 cancellation。
7. `write_type_declarations(false)` 不写只读 extension root。
8. `gpui-component` module 可在 Navop shell view 中导入并物化组件。

### 20.2 manifest/catalog

1. shell view 正确 namespaced 注册。
2. entry/icon path escape、symlink escape 被拒绝。
3. backend 指向未知、WASM 或其他 extension runtime 被拒绝。
4. duplicate view id、alias 和 module 被拒绝。
5. 缺少 `shell:exec` 时在执行脚本前失败并进入高危权限审查。
6. `api.shell` 或 `engines.gpui_shell` 不兼容时在执行脚本前失败。
7. shell contribution 子结构的未知字段 fail closed。
8. catalog snapshot 的 revision 和 Arc 原子一致。
9. `secret://self/key` 为两个不同 extension id 生成不同内部 namespace，不能跨包读取。

### 20.3 ShellPluginHost

1. 两个 mount 绑定同一 runtime：两个 lease、一个 process。
2. 关闭一个 mount 不停止 process；关闭最后一个清理 job registry。
3. close during start 不安装迟到 view，`StartClaimGuard` 后可重试。
4. 多 backend 中途失败会回滚已经取得的 lease。
5. script load 失败会 unload policy 并释放全部 lease。
6. stale mount completion 不能覆盖新的 mount attempt。
7. runtime restart 后旧 handle 返回 `STALE_HANDLE`，新 handle 可用。
8. mount cancel 传入所有 typed provider RPC 的 `RequestOptions`。
9. event read 有界，关闭/cancel 后不保留 runtime。
10. blob chunk limit 和 Base64 round-trip 正确。
11. 超出 JS safe integer 的 JSON 值无损 round-trip。
12. 未声明 backend alias/module/permission 的调用 fail closed。
13. endpoint descriptor 支持 wildcard、Unix socket 和 profile path，legacy extractor 仅回退。
14. `gpui-component` overlay 使用现有 Root，不要求窗口 root 为 `ShellRoot`。
15. cancel 与 success response 同时 ready 时保留 result，并执行 create-call compensator。
16. forced tab removal/clear 不经过 `try_close` 时仍会取消 mount 并释放 policy/runtime。
17. late-response tombstone 达到容量上限时 fail closed，不发生无界增长或静默丢弃。

### 20.4 catalog install/update/uninstall

1. revision N 安装新 view 后无需重启应用即可打开。
2. update Retiring gate 期间拒绝新 mount/activation。
3. 旧 mount/runtime drain 完成前不发布同 key 新 binding。
4. revision N+1 发布后新 mount 使用新 entry/binding。
5. 卸载先关闭 mount/runtime，再删除文件和发布 catalog。
6. 活动 runtime 使用固定 binding snapshot，catalog 删除后 monitor 不 panic。
7. 并发 sync 不会将 manager 回退到旧 revision。
8. provider restart 默认将旧 job 标记 stale，而不是无条件迁移 generation。
9. Retiring 会等待/取消尚未返回 lease 的 start claim，不会提前删除旧文件。

### 20.5 E2E

使用 fake Elasticsearch HTTP server：

```text
install extension
-> publish catalog
-> open ShellPluginTab
-> provider activate/init
-> StoredConnection + secret reference
-> resource/open
-> cluster info/search
-> job/event/blob
-> close tab
-> verify process and registries cleaned
```

## 21. 实施阶段

### Phase 0：底层收口

- 将 global catalog 改为原子 snapshot。
- 活动 runtime 固定 binding snapshot，增加 extension Retiring gate 和 drain transaction。
- Retiring drain 统计并取消 in-flight start claim，不能只等待已返回 lease。
- 所有 typed provider method 支持 `RequestOptions` 和 mount cancellation。
- side-effectful create call 使用 response-priority 和 late-response compensator。
- 将 endpoint 授权改为标准 descriptor 优先、legacy config extractor 回退，并修复 wildcard。
- 停止无 negotiated recovery 的 job generation 自动迁移。
- 补齐 production secrets/storage/log/notify。
- 为 ShellPluginHost 暴露所需的 typed client 和 health subscription facade。

### Phase 1：最小 fork 改动

- 在 gpui-shell 增加 `ViewLoadOptions` 和 public view loader。
- 增加 opaque `LoadedScriptView`、`unload(cx)`、module revoke 和 generation liveness guard。
- 增加 cancellable HostModule task，并允许关闭 type declaration 写入。
- 增加 fork 单元测试。
- 更新 Navop 固定 revision 和 lockfile。

### Phase 2：manifest/catalog

- 增加 `ShellViewContrib` 和 `RegisteredShellViewContribution`。
- 增加 `engines.gpui_shell`、`api.shell`、path、backend、module 和 `shell:exec` 校验。
- command handler 增加 `shell_view`。
- extension_view 展示静态 shell contribution。

### Phase 3：宿主和 tab

- `main` 引入 `gpui_ce_components_shell`。
- `gpui_component::init(cx)` 后调用 fork 新增的 `gpui_shell::init_embedded(cx)`；该入口只
  初始化 shell reflection/runtime，不重复调用 `gpui_base::init(cx)`。
- 扩展 `ShellPluginHost`、mount state 和 lease rollback。
- 实现 `ShellPluginTab` 和 Loading/Failed/Ready 状态。

### Phase 4：HostModules 与 SDK

- 先实现 context、runtime、resource、blob、log。
- 再实现 job、event、connection 和 database。
- 提供 TypeScript declarations、`NavopValue` codec 和 async iterator helper。
- 所有 Tokio-bound 调用走 Navop Tokio bridge 和 root cancellation token。

### Phase 5：热更新和管理页

- extension install/update/uninstall 与 mount reconciliation 串联。
- 实现版本目录 staging、Retiring gate 和严格 drain 后切换。
- 增加 active mount、reopen 和错误状态展示。

### Phase 6：Elasticsearch reference extension（已迁移到 navop-extensions）

- provider 保持 headless。
- 增加 shell explorer UI 和 fake-server E2E。
- 验证 resource/job/event/blob/secret/restart/catalog 全链路。

## 22. 拒绝的方案

### 22.1 恢复 provider UI RPC

拒绝原因：再次把 GPUI 生命周期、UI schema 和 provider protocol 耦合；每种复杂控件都
会扩张协议，并且无法安全表达原生焦点、虚拟列表和现有 Root 行为。

### 22.2 动态 Rust dylib

拒绝原因：Rust 没有稳定 ABI，加载后拥有宿主全部权限，也无法形成有意义的 policy。

### 22.3 WebView 作为统一插件 UI

拒绝原因：与 Navop 原生焦点、主题、TabContent、快捷键、无障碍和资源占用不一致；
同时没有消除 backend 权限和生命周期问题。

### 22.4 修改 `gpui_component::Root` 适配 ShellRoot

MVP 拒绝原因：影响所有 Navop window，扩大 fork 面和回归范围。现有 Navop Root 已可承载
`gpui-component-shell` 注册的组件和 overlay。

### 22.5 脚本直接获得 provider client/runtime id

拒绝原因：无法限制跨扩展调用、generation、handle ownership 和 cleanup；也会把 Rust
内部类型泄漏到脚本 ABI。

### 22.6 脚本直接获得 network/process 权限

MVP 拒绝原因：绕过 provider 的 permission authorizer、secret、timeout、cancel、blob
和审计路径。需要连接任意中间件时应新增 provider，不新增 script direct network。

## 23. 验收标准

设计实现完成时必须满足：

1. 不存在新的 provider UI method 或 declarative UI DTO。
2. gpui-component 普通控件和 Root 无 Navop 专用改动。
3. 一个安装包可声明 shell view 和任意 IPC provider，并在无需重启 Navop 时打开。
4. `shell:exec` 必须经过高危权限门禁；脚本 UI 统一使用 `gpui-component` module，不存在
   第二套 Navop UI API。
5. shell view 只能使用声明的 backend alias 和已授权 HostModules。
6. 两个 mount 共享 runtime process，但关闭和取消互不释放对方 lease。
7. 最后一个 mount 关闭后 process、job、event、blob、HostModules 和 policy task 均清理。
8. mount cancel 能到达 provider RPC；不合作 provider 被记录为 contract violation。
9. provider restart 后旧 generation handle 明确失效，不发生隐式写重放或 job 假恢复。
10. catalog 更新通过 Retiring/drain transaction，不产生同 key 新旧 binding 混用或
    revision 回退。
11. Navop connection API 不返回保存 secret；同包 provider 的 secret exfiltration 由整包
    信任模型承担，不作虚假隔离承诺。
12. Elasticsearch reference extension 覆盖完整 UI 到 middleware 的 E2E。

## 24. 关键代码引用

- `main/src/shell_plugin_host.rs`：当前 runtime lease bridge。
- `main/src/universal_plugins.rs`：应用唯一 provider owner、catalog sync 和 monitor。
- `crates/extension-plugin-adapter/src/activation.rs`：activation、generation 和 supervision。
- `crates/extension-host/src/universal_plugin.rs`：typed resource/job/event/blob facade。
- `crates/extension-runtime/src/extension/manifest/contributes.rs`：新 contribution 落点。
- `crates/extension-runtime/src/catalog.rs`：registered shell view catalog 落点。
- `crates/extension-runtime/src/global.rs`：revision snapshot 落点。
- `crates/core/src/tab_container.rs`：`ShellPluginTab` 的宿主 contract。
- gpui-component fork `crates/shell/src/plugin.rs`：policy-aware load/unload 落点。
- gpui-component fork `crates/shell/src/policy.rs`：per-plugin authority。
- gpui-component fork `crates/shell/src/host_modules.rs`：plain-data host boundary。
