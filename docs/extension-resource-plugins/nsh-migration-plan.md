# navop 扩展机制适配 nsh 的重构方案（v2）

- 状态：提案（v2，2026-10-30）。**取代 v1 的「引擎替换」思路**：不是把 nsh 塞进 navop 现有扩展抽象当"另一个引擎"，而是**以 nsh 的包模型为一等扩展形态，navop 的扩展机制围绕它重构**。
- 前提（用户已确认，m00310）：扩展尚未正式使用、可任意重构；gpui-fast 与 nsh 均已 fork（feigeCode），长期与官方同步；扩展机制是 navop 后续发展的核心。
- 一句话方向：**navop 不再自定义扩展格式，改为做一个高质量的 nsh 宿主（host）**——扩展 = nsh 签名包（.nshpkg），分发/信任/能力声明用 nsh 的协议，navop 的价值收敛为「宿主能力 + 工作台集成 + 分发治理」。

---

## 1. 结论先行

| 问题 | 答案 |
|---|---|
| 为什么反转方向？ | nsh 已经有完整的**分发/信任/工具链闭环**（keygen→certify→publish→verify→accept→revoke、build 单调防回滚、min_runtime 版本门、critical 前向兼容、capabilities 声明+授予双层），且其 TSX 运行时加载模型就是为 plugins/AI 生成 UI 设计的。navop 自造的 extension.json + engines 门 + grant.rs 是这套东西的子集——映射进 nsh 模型，而不是让 nsh 适配 navop。 |
| 会有两份 gpui 吗？ | **不会**（结论与 v1 相同）。nsh 的 GPUI 后端依赖 gpui-fast（crates.io 0.1.9 需求），被 navop 的 `[patch.crates-io]` 统一解析到 feigeCode/gpui-fast `release/v0.1.12`：全进程一份 gpui-fast、一个 App/事件循环/渲染管线。 |
| navop 现有机制怎么办？ | 能力平面（provider runtime / ActivationManager / lease / RuntimeMonitor / ProcessRpcSession）**原样保留**，通过 host ABI 暴露给 nsh 页面；UI 平面与扩展格式（extension.json + engines.gpui_shell + contributes.shellViews + 31 个 JS 页面）**整体换血**为 .nshpkg + TSX 页面。 |
| 重构代价 | navop 侧：NshHost 新写（TrustStore 集成 + host ABI + 主题桥 + 装载）、catalog/激活接线改造；扩展侧：31 个页面 view 层重写为 nsh 方言 TSX（逻辑层可搬）；工具链**零成本**（nsh CLI 现成：init/build/check/test/dev/publish）。 |

---

## 2. nsh 的扩展模型（「一等形态」的事实基础）

以下全部已在 nsh-fork 源码核实。这套模型就是 navop 扩展机制的新地基：

### 2.1 扩展单元 = .nshpkg 签名包（nsh-package）

- **内容**：`MAGIC "NSHP" + FORMAT 1 + 签名头 + files + 64 字节签名`。App 元数据来自 nsh.json：`id`（反域名）、`name`、`version`、`build: u64`（单调递增）。文件四类：`bundle.js`（源码）/ `bundle.nshc`（本 runtime 字节码）/ `bundle.js.map` / `asset:/<path>` 资产，每文件带 sha256/offset/size，未知类型仍参与哈希。
- **头字段**（签名前不信任任何字段）：`app`、`capabilities`、`bytecode_version`、`min_runtime`、`created_at`、`certificate`、`files`、`critical: Vec<String>`（新 runtime 必须理解否则整包拒绝的字段名）、未知字段忽略。**前向兼容内建**：老 runtime 遇到新 signer 的未知内容会记录进 `ignored` 并跳过，不会崩。
- **构建**：`nsh_package::build(&Contents, &SecretKey, &Certificate)`；`Contents{app, capabilities, source, bytecode, source_map, assets, min_runtime}`。

### 2.2 信任链（Ed25519 + 证书 + 撤销 + 防回滚）

- `nsh keygen` 生成 SecretKey/PublicKey（Ed25519，base64）；`nsh certify` 由 root key 签发证书，**限定可签的 app id 模式**（`com.example.*` / `*`）与时间窗（days）。
- `TrustStore`：`add_root(PublicKey)` / `update_revocations(Revocations)` / `verify(bytes, BYTECODE_VERSION) -> VerifiedPackage`（唯一能构造 VerifiedPackage 的路径，持有即证明验签通过）/ `accept(&VerifiedPackage)` / `accepted_build(id) -> Option<u64>`——**build 单调**：接受过新 build 后旧 build 被拒（防回滚）。
- `nsh revoke` 按 key id / 整包 sha256 撤销，sequence 递增。
- **两阶段装载**（nsh-engine lib.rs:464-533）：`verify_package`（验签+资产解包到 `<assets_root>/<app id>/<sha256>/`，不记录 build）→ 宿主确认健康（首帧画出、无 fatal）→ `accept_package`（记录 build）。失败的包可回退到上一个仍能通过验证的包。
- `check_min_runtime(version)`（nsh-package lib.rs:174）：min_runtime 版本门。

### 2.3 能力模型（nsh-core access.rs:343）

`Capabilities`（`#[non_exhaustive]`，Default 全拒，**新能力默认 denied**）：`process` / `fs` / `network`（fetch/远程图/视频）/ `webview` / `status_bar` / `clipboard` / `picker` / **`host: Vec<String>`**（nsh/host 可调的宿主函数名白名单，支持 `*` 前缀，空 = 无）/ `open_url`。

双层执行：`Policy { harden, declared, granted }`——声明（nsh.json capabilities，包内签名）∩ 授予（宿主上限，`Bundle::from_package` 后宿主可再收敛）。`harden` 默认 true（关 eval/函数构造器/可写共享原型），dev bundle 关。**未知能力不授予**（from_json_lenient 把 dropped 项记 warn）。

### 2.4 宿主桥（nsh-js host.rs + nsh-engine host_channel.rs）

- **Rust→JS**：`HostChannel::send(name, JsonValue)` → JS `host.on(name, listener)`（无监听器时事件保留）；`host_event` 同。
- **JS→Rust**：`host.call(name, args)` → `HostChannel::handle(name, Fn(HostCall))`；HostCall `resolve/reject`，drop 即 reject（天然异步取消语义）；`host.post` 单向。受 **capabilities.host 白名单** + pending-call 配额/超时约束。
- **启动参数**：`HostChannel::set_params(Map)`，JS `host.params` 首次 import 时读。
- **富宿主模块**：`nativeModule<T>("host/trading")` —— 宿主把 Rust 领域对象（含方法、事件 `on("change")`）以类型化模块暴露给页面（embed-gpui 的 TradingApi 实例）。
- **原生控件**：`nativeComponent<Props,Events>("host.switch")` —— 宿主用 gpui-kit 真控件参与 nsh 页面渲染（NativeComponent::render -> AnyElement）。
- **其它注入点**：`set_filesystem`（能力检查留在 Rust）、`set_websocket_transport`、`set_exit_handler`（NSH 永不自杀进程）、`set_system_wake`。

### 2.5 工具链（nsh CLI，全 Rust/oxc，无 Node）

`init --template ts/rust/mixed` / `build --release --split` / `check`（TS7 原生编译器，npm registry 一次性下载）/ `test --target gpui --junit`（每测试独立 engine，headless）/ `dev --port --limits --target`（热更新）/ `run` / `keygen` / `certify --apps --days` / `publish --key --cert --min_runtime` / `symbolicate` / `verify --root --revocations` / `revoke`。

### 2.6 运行时形态（nsh-platform-gpui）

`AppView`：标准 GPUI Entity（Render/Focusable/EntityInputHandler），per-view 独立 engine 线程 + QuickJS VM + data_dir + HostChannel；`AppView::new(AppOptions, window, cx)` → `load(App::Js(bundle))` 或 `run_package(&LoadedPackage)`；`connect_dev_server`（dev 热更新）；进程级共享面：`set_ui_sets(Vec<Registry>)`（组件/模块注册表）、keybindings、rt 主线程队列、macOS 菜单 last-wins。

### 2.7 UE 表现力（方言对比，2026-11-02 核实）

- **TSX 结构化 vs JS builder 链**：gpui-shell 方言（`v_flex().child(h_flex().child(...))`）嵌套 3~4 层后结构不可读，是现有 31 页全是「平铺列表+徽章+按钮」的结构性原因；nsh 方言（JSX 树 + style 对象/className） UE 上限高一个档次——实证：nwork（自研 nsh 应用，split-grid 分栏、quicklook、批量操作栏等复杂 UE，docs/shots/ 27 张截图）与官方 gallery（60+ demo）。
- **组件覆盖**：`nsh/base`（TSX 侧模块）71+ 组件（gallery import 统计）：DataTable（百万行虚拟化、onVisibleRowsChange 窗口服务、33ms 实时 tick）、Editor（LSP diagnostics/completion/decoration）、TextView(markdown/html)、DiffView、DockArea、ResizablePanel、Tree、Combobox、Command、ColorPicker 全家、plot 全家（Arc/Area/Bar/Line/Pie/Sankey）、Carousel、Tabs、Toolbar、HoverCard、Canvas 2D 等——gpui-component 深水区均有对应物。nativeComponent 桥只是兜底，不是主路径。
- **Tailwind className**：原生支持任意值/渐变/内阴影/dark 变体（gallery/Tailwind.tsx），AI 生成 UI 语料直接可用；主题 token 统一 `token("background")` / className，无 gpui-shell 的暗规则坑（如 Badge.color 拒主题 token 名）。

---

## 3. 目标架构：navop 作为一个 nsh host

```
┌──────────────────────────────── navop 主进程（一个 GPUI App）────────────────────────────────┐
│                                                                                              │
│  分发治理（navop 新增）              能力平面（保留不动）           UI 平面（重做）              │
│  ┌────────────────────┐            ┌───────────────────────┐   ┌──────────────────────────┐ │
│  │ ExtensionMarket     │  安装      │ ExtensionRuntimeCatalog│   │ NshHost（替代ShellPlugin  │ │
│  │  · .nshpkg 仓库      │──────────▶│ ActivationManager/lease│──▶│   Host）                  │ │
│  │  · TrustStore roots │  verify    │ RuntimeMonitor         │   │  · TrustStore 集成         │ │
│  │  · Revocations 更新 │  →accept   │ ProcessRpcSession      │   │  · load/verify/accept 两阶段│ │
│  └────────────────────┘            │ （resource/job/event/  │   │  · navop.* → HostChannel   │ │
│                                    │   blob provider）      │   │  · 主题/i18n 桥（push）     │ │
│                                    └───────────┬───────────┘   └──────────┬───────────────┘ │
│                                                │ host.call 暴露            │ AppView + Bundle │
│  ┌───────────────────────── GPUI 窗口（TabContainer/Root + 宿主 overlay）──────────────────┐ │
│  │   Tab: NshTab ──── Entity<AppView>（nsh surface：独立 engine 线程 + nsh-base 组件集）     │ │
│  └──────────────────────────────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

三层分工：

- **nsh（上游拥有）**：扩展格式、信任协议、能力模型、脚本运行时、组件集、CLI 工具链。navop 不 fork 这些语义，跟随上游演进（min_runtime/critical/ignored 机制保证老包永远可跑）。
- **navop 宿主侧**：TrustStore 治理（roots/撤销/accept 策略）、navop.* host ABI（把能力平面暴露给页面）、工作台集成（tab 装载/主题/i18n/lease）、市场与安装。
- **扩展作者**：`nsh init` → 写 TSX（`host.call("navop.resource.invoke", …)` 调 provider）→ `nsh publish` 出 .nshpkg → 提交 navop 市场。全程不需要懂 navop 内部。

---

## 4. 概念映射：navop 现有 → nsh（谁替代谁）

| navop 现有概念 | nsh 对应 | 处置 |
|---|---|---|
| extension.json（manifest） | **nsh.json**（name/id/version/build/icon/capabilities） | 替换。nsh.json 是 `deny_unknown_fields`（CLI AppConfig），**navop 不往里加字段**；navop 特有元数据放包内 asset（见 §5） |
| `engines.gpui_shell` 版本门（宿主常量 "0.6.4"，minor 必须相同） | 包头 `min_runtime` + `check_min_runtime` + `critical` 前向兼容 | 替换，且更健壮：semver 门 + "新 runtime 必须理解"字段 + 未知内容忽略 |
| `permissions: Vec<String>` + grant.rs fail-closed 授权 | `capabilities`（declared，签名）∩ 宿主 `granted` 双层 | 替换。grant.rs 的语义（读写根/network_hosts/spawn 白名单/fail-closed）映射进 nsh Capabilities 的 fs/network/process 字段 |
| navop.* 11 HostModule 30 函数 | `HostChannel::handle` + `capabilities.host` 白名单 + `nativeModule` | 重接。host 白名单天然表达"页面可调哪些宿主函数" |
| contributes.shellViews（tab/toolbox/singleton/backends） | 包内 `navop.json` asset（见 §5） | 语义收窄保留：只声明入口/表面/单例；backends 对应 host 依赖声明 |
| ShellViewOpener / ShellPluginTab（Entity\<ScriptView\>） | NshShellViewOpener / NshTab（Entity\<AppView\>） | 重写，trait（crates/extension_view/src/shell.rs:6-10）本来就引擎无关 |
| ActivationManager / lease / StartClaimGuard / RuntimeMonitor | 保留，挂在 nsh 生命周期上：open tab = acquire（AppView::new + load），close = release（stop 后台化 + drop） | 保留不动，只换触发点 |
| ProcessRpcSession（resource/job/event/blob provider） | 不变，经 host ABI 暴露 | 保留不动 |
| 扩展分发（git 子模块 / 手动 clone） | .nshpkg + TrustStore + 市场 | 替换。28 个 wasm/ipc provider 扩展形态不受影响（它们不是 UI 扩展） |
| HostAsyncTask + CancellationToken | HostCall resolve/reject + drop-reject | 替换，语义更干净 |
| `__NAVOP_ERROR__` Base64URL 错误信封 | `call.reject(json)` 直接结构化 | 替换，无需信封 |
| 主题 pull（render 时读 ThemeSnapshot） | `set_theme` push + shadcn token（embed-gpui theme.rs ~90 行模式） | 升级为 push |
| i18n pull（locale/revision） | `set_locale` + `host.send("navop.locale", …)` | 升级为 push |
| with_current_app 17 处主线程栈不变量 | 消失（engine 独立线程）；需主线程的走 nsh rt 队列 | 删除 |

---

## 5. 扩展单元与元数据决策

**一个 .nshpkg = 一个 nsh app = navop 里一个可装载的扩展单元。**

> **实现状态（2026-11-04）**：已按本节落地「一个扩展一个 nsh app、一个 entry、一个
> build 号」，页面改为 app 内路由；宿主清单用 `contributes.shellApp`（而不是包内
> `navop.json` asset），连接用 `connections[].route`。细节见
> `docs/extension-resource-plugins/plugin-app.md`。仍未做的是真正的 `.nshpkg`
> 签名分发（§14.2），当前仍是目录形态的 `ui/dist/`。

- **一包一 app，app id 即扩展 id**（反域名，如 `com.navop.ext.docker`）。需要多页面的扩展：页面内用 NavStack/NavPage 自行导航（nsh 原生模式），或发布多个包（同 publisher 前缀，证书可限定 `com.navop.ext.docker.*`）。
- **navop 特有声明放包内 asset `navop.json`**（参与整包签名哈希，不能被篡改）：
  ```json
  {
    "views": [
      { "id": "overview", "title": "Docker Overview", "icon": "asset:/icon.png",
        "entry": "default", "surface": "tab", "singleton": true,
        "requires": ["backends:docker", "host:navop.resource.*"] }
    ],
    "toolbox": [ … ]
  }
  ```
  宿主在 `verify_package` 后读该 asset 注册进 ExtensionRuntimeCatalog。这样 **nsh.json 与 nsh CLI 完全不用改**，工具链原生可用，navop 声明仍受签名保护。
- **复合扩展**（既有 provider 又有 UI，如 docker/mqtt/rocketmq/es）：provider 部分维持 wasm/ipc 形态（能力平面不动），UI 部分发布 .nshpkg，页面经 `host.call("navop.resource.invoke", {backend:"docker", …})` 打到同一 ProcessRpcSession——通过 `views[].requires` 声明 backend 依赖，宿主装载前核对。

---

## 6. host ABI：navop.* → HostChannel

11 个模块 30 函数按 nsh 语义重新归位（不是逐个照搬，而是按 nsh 的四类桥接分流）：

```rust
// NshHost 装配（per-view）
let host = HostChannel::new();
host.set_params({ext_id, view_id, locale, theme_revision, appearance});   // 替代 navop.context

// ① 异步能力 → host.call（受 capabilities.host 白名单）
host.handle("navop.resource.invoke", |call| { /* spawn_provider_task → call.resolve/reject(json) */ });
host.handle("navop.job.*", …); host.handle("navop.event.*", …); host.handle("navop.blob.*", …);

// ② 领域状态 → nativeModule（类型化模块，含事件订阅）
//    navop.workbench（current/dispatch）→ nativeModule("navop/workbench")，selection 变化 host.send 推送
// ③ 宿主控件 → nativeComponent（gpui-kit 真控件桥进 nsh 页面，按需）
// ④ 纯信息 → host.params + host.on 事件（theme/locale push）
```

- **取消**：close 时 session.cancel() 逐个 reject 在途 call（drop-reject 语义）；JS promise 收到 reject 即结束。
- **错误**：reject 结构化 JSON（code/message/details），比 Base64 信封干净。
- **线程纪律**：handler 跑在 engine 线程——勿阻塞、勿碰 App/窗口；需要主线程的经 nsh rt 队列回 GPUI 线程（nsh 自带）。
- **capabilities.host 对齐**：包声明 `{"host": ["navop.resource.*", "navop.event.*"]}`；宿主 granted 同名收敛。现有 grant.rs 的 fs/net/spawn 语义映射进 Capabilities 对应字段，secrets/shell:exec/database 依旧不授予。

## 7. 能力平面（保留）与 nsh 生命周期的咬合

- **lease**：open view 前 acquire（StartClaimGuard）→ `AppView::new` + `run_package(&LoadedPackage)`；close 时 `session.cancel()` → `view.stop()`（**后台化**，engine 线程 join 勿在 UI 线程）→ drop Entity → release_after_session。
- **首帧健康 → accept**：正好用 nsh 两阶段装载表达"激活成功"——首帧画出 + 无 fatal 一段时间后 `accept_package`（记 build）；失败可回退上一包。
- **RuntimeMonitor**：nsh 侧 `on_error` + host.call 超时/配额自带一层；ProcessRpcSession 监控沿用。
- **窗口参数**：宿主窗口 `inactive_frame_interval: None`（否则非聚焦 30fps 动画减半，AppView 文档 app.rs:200-233）。

---

## 8. 信任与分发（navop 市场协议）

- **root 治理**：navop 内置官方 root key；设置页允许用户/企业管理员 `add_root`（自建生态）。市场分发的 .nshpkg 附证书链，`nsh verify` 可独立核验。
- **开发者入驻**：`nsh keygen` → 向 navop 市场注册公钥（个人）或 `nsh certify`（企业 root 限定 app id 模式 + 时间窗）。
- **撤销分发**：市场发布 Revocations（sequence 递增），navop 启动/定期 `update_revocations`。
- **防回滚**：TrustStore accepted_build 自动生效（接受过 docker 扩展 build 5 后，build 3 的旧包拒绝加载）。
- **dev 模式例外**（不破坏信任链）：
  - 本地开发：navop dev 模式加载**未签名** Bundle::source（`Policy::development()`，capabilities 仍强制，缺能力在开发期就暴露）；
  - 热更新：`connect_dev_server`（nsh dev server）；
  - CI：`nsh test --target gpui` headless 跑扩展自身测试，与 navop 的渲染基线测试互补。

---

## 9. 依赖与版本策略（同 v1，摘要）

1. navop workspace git 依赖 `nsh-platform-gpui`（feigeCode/nsh fork，钉 rev），`default-features = false, features = ["js"]`（按需 webview）。
2. `[patch.crates-io]`（现 Cargo.toml:393-425）天然把 nsh 侧 gpui-fast 0.1.9 需求统一到 release/v0.1.12，全进程一份 gpui-fast。
3. **nsh fork 补丁（唯一必改点）**：chrome.rs:1315 `KeyBinding::new` 对 0.1.12 编译不过（'static + panic），改为 `KeyBinding::load(&s, …, cx)` + 失败跳过。官方 upstream（HEAD 201d780）钉 0.1.9 不受影响，PR 定性「适配 gpui-fast 0.1.12」。
4. 每次 gpui-fast bump 同步验证 nsh fork 编译（两消费方一个 patch 面），进 CI。

---

## 10. 里程碑

| 阶段 | 内容 | 验收 |
|---|---|---|
| M1 fork 补丁 | nsh fork chrome.rs:1315 改 KeyBinding::load；上游 PR | nsh-platform-gpui 在 gpui-fast 0.1.12 下 cargo check 通过 |
| M2 spike | navop 挂一个 AppView tab（Bundle::source 加载 nsh 官方示例）；主题桥（~90 行） | tab 内渲染、Tab 进出 surface、宿主 overlay 盖其上 |
| M3 NshHost 骨架 | TrustStore 持久化（目录）、verify/load/accept 两阶段、NshTab 生命周期（stop 后台化）、`navop.json` asset 解析 | 从磁盘装一个 .nshpkg 开 tab、close、再装回退包 |
| M4 host ABI | navop.* 按 §6 四类桥接分流（resource/job/event/blob → host.call；workbench → nativeModule；主题/i18n → push）；错误/取消语义 | TSX 页面 await host.call 打通 provider 全链路 |
| M5 catalog/市场接线 | ExtensionRuntimeCatalog 收 .nshpkg、lease 咬合（§7）、安装/更新/回滚/撤销流、dev 模式例外 | 市场装包→升级→撤销→回退全流程；本地 dev bundle 热载 |
| M6 试点迁移 | shell-view-example、js-toolbox(3)、docker overview 以 nsh app 形态重写 | 试点页全功能；产出方言映射表（gpui-component → nsh/base，注意组件名与 props 语义差异） |
| M7 全量迁移 + 收尾 | 剩余页面、测试基线重建（extension_pages_render_tests → nsh test/shot 形态）、移除 gpui-shell 依赖、docs 更新 | cargo tree 无 gpui-shell；全页断言通过 |

M1–M2 去风险（嵌入可行性最便宜的验证点）；M3–M5 是本方案的核心重构量；M6–M7 机械迁移。

## 11. 体量估算

| 项 | 量级 |
|---|---|
| NshHost 新写（TrustStore 集成 ~200 行；host ABI 适配 ~1,500 行，自 HostModule 1,470 行改造；主题桥 ~90；NshTab/opener ~300；navop.json 解析/catalog ~400） | ~2,500 行 |
| 可移植直搬（session/cleanup/monitor/connection/grant 语义映射 ~930；main 胶水小改 ~270） | ~1,200 行 |
| 删除（gpui-shell 引擎绑定 1,452 + 测试 1,275 重写） | −2,700 行 |
| 扩展页面 31 个 / 7,462 行 | view 层重写为 TSX，逻辑层（navop.* 调用）可搬 |
| 工具链 | 0（nsh CLI 现成） |

## 12. 风险清单

| 风险 | 等级 | 缓解 |
|---|---|---|
| gpui-fast bump 需同步 nsh fork（两消费方） | 高 | patch 面收敛在 [patch.crates-io]；每次 bump 跑 nsh cargo check 进 CI；上游 PR 合入后减压 |
| nsh.json 不可扩展（deny_unknown_fields） | 中 | navop 元数据走包内 navop.json asset（签名覆盖），nsh.json/CLI 零改动 |
| TrustStore/密钥治理（root 泄露、企业管理） | 中 | roots 可增可撤（Revocations）；企业用 certify 限定 app id 模式 + 时间窗；文档化 keygen→注册流 |
| UI_SETS/keybindings/rt 队列进程级共享；macOS 菜单 last-wins | 中 | UI_SETS 只放公共集（nsh_base::registry() + navop 桥接件）；per-view 走 AppOptions.components；扩展页不定义应用菜单 |
| stop() join 阻塞 UI | 中 | close 路径 background_executor 后台化 |
| webview overlay 盖 GPUI；Linux 外 video；变换组字形等渲染缺口 | 低 | 扩展页暂无此需求；需要时 nativeComponent 桥接兜底 |
| 31 页全量重写人力 | 中 | TSX 结构化写法 UE 上限高于 builder 链（nwork 27 张截图 + 官方 gallery 60+ demo 已验证）；nsh/base 71+ 组件（DataTable 百万行虚拟化/Editor+LSP/DiffView/DockArea/plot 全家，gallery import 证实）覆盖 gpui-component 深水区；nativeComponent 桥仅兜底 |
| 生态断代（老 extension.json UI 扩展全失效） | 低 | 用户已确认：扩展尚未正式使用（28 个现存扩展全是 provider，不受影响） |
| 未来 gpui-fast 动态 keybinding 方向易碎 | 中 | KeyBinding::load 补丁模式固定；跟踪上游 0.2 |

## 13. 与 v1 方案的差异（方向反转点）

| 维度 | v1（引擎替换） | v2（本方案，navop 适配 nsh） |
|---|---|---|
| 扩展格式 | navop extension.json 保留，engines 换成 engines.nsh | **.nshpkg 签名包**，nsh.json + 包内 navop.json asset，格式跟随 nsh 上游 |
| 版本门 | 宿主常量 + minor 相同规则 | 包头 min_runtime + critical + ignored 前向兼容机制 |
| 授权 | grant.rs 原样，映射进 nsh Policy | nsh Capabilities declared∩granted 双层 + capabilities.host 函数白名单 |
| 分发 | 未涉及（git 子模块/manual） | 完整信任链：keygen/certify/publish/verify/accept/revoke + 防回滚 + 市场 |
| 开发者工作流 | navop 自建脚本装载 | nsh CLI 原生（init/build/check/test/dev/publish），navop 只做宿主 |
| host ABI | 30 函数逐个映射 HostChannel | 按 nsh 四类桥接分流（host.call/nativeModule/nativeComponent/params+on） |
| 战略含义 | navop 继续拥有扩展格式 | **扩展生态直接对接 nsh 生态**（含 AI 生成 UI 方向）；navop 差异化 = 宿主能力 + 工作台 + 分发治理 |

---

## 14. 实施记录（2026-11-02）

方案已按 M1–M8 全部落地，分支 `refactor/nsh-host`（开发期在 `m2-nsh-spike`），
扩展仓分支 `feat/nsh-pages`。沉淀下来的事实与**与方案的偏差**都在这一节。

### 14.1 里程碑与实际提交

| 里程碑 | 内容 | 落点 |
|---|---|---|
| M1 | nsh fork 兼容 gpui-fast 0.1.12 | `feigeCode/nsh` 分支 `navop/gpui-0.1.12` 提交 `81c93f59`：`chrome.rs` 改用 `KeyBinding::load`（`KeyBinding::new` 现在要 `&'static str`），并补本地 patch 块把 `gpui-fast*`/`gpui-pre-*` 全钉到 `release/v0.1.12` |
| M2 | navop 里挂 `AppView` tab | `nsh_host/` 雏形 + 编译期 `nsh build` 产物内嵌 |
| M3 | `NshHost` 骨架（目录装载） | `nsh:{view_key}` tab、装载失败也开 tab 并显示原因 |
| M4 | host ABI 全量桥接 | `abi.rs`（context/log/i18n/theme/resource/job/event/blob/runtime/dev/workbench）+ `session.rs`（四张句柄表、generation、取消）+ `host_error.rs`（12 个稳定码） |
| M5 | 租约装载 + 嵌入页 | `NshTab::start_prepare` 走 `activate_runtime` 租约；`page_host.rs` 实现 `CustomPageHost`（借来的资源 `owned:false`，`navop.workbench.dispatch`） |
| M6 | 引擎门 + 产物布局 + 试点页 | `engines.nsh`、`entry → dist/bundle.js`、docker overview 试点 |
| M7 | 删除 gpui-shell | `8cba5c7a5`：删 `shell_plugin_host/`、`shell_plugin_tab*`、`shell_page_host.rs`、`gpui-shell` 依赖与 `engines.gpui_shell` 门；`cargo tree -i gpui-shell` 不再命中，`Cargo.lock` 里 0 处 |
| M8 | 24 个页面改用 TSX | docker 10 / mqtt 5 / rocketmq 3 / elasticsearch 1 / dev-tools 1 / 示例 4；共享设计系统 `docker/ui/lib/{theme,format,ui}` |

### 14.2 与方案不一致的地方（都是有意收敛）

- **打包分发仍未走 `.nshpkg` 签名包**。宿主两条装载路径都在（`bundle.rs`：同目录
  有 `.nshpkg` 就 `verify_package` + 首帧健康后 `accept_package`；否则 `Bundle::source`
  直装），但扩展仓目前发布的是**目录形态**——`dist/bundle.js` 随扩展包一起走，
  信任边界仍是"扩展包整体签名/安装"，没有用到 nsh 的包级签名与防回滚。要走签名包，
  扩展仓的发布流水线得改成 `nsh publish` 产出 `.nshpkg`，宿主把安装目录里的 `.nshpkg`
  交给 `verify_package`。这一步没有做，因为扩展仓的发布链路（R2/多 target）不在本
  次重构范围内。
- **`navop.json` 资产没有引入**。方案里设想把 navop 专有的工作台声明塞进包内
  asset 参与签名；实际仍用 `extension.json`（nsh CLI 不解析它，所以不冲突），
  也就没动 `deny_unknown_fields` 那条约束。
- **`nativeModule` / `nativeComponent` 没有用**。四类桥接里只落地了 `host.call`
  一类：navop 的工作台/资源能力全是数据型调用，不需要把 gpui-kit 控件注入页面；
  需要原生控件时再按 `examples/embed-gpui` 的 `NativeComponent` 做法补。
- **`set_ui_sets` 只用默认集**（`nsh_base::registry()`），没有注册 navop 自有组件
  目录——`ui_sets` 是进程级的，而 navop 的组件需求已经在 TSX 里自绘解决。
- **热更新接入 dev server 未接**：`connect_dev_server` 已有 API，但 navop 的开发者
  工具页现在是"打开本地工程目录"的形态（`navop.dev.*`）。

### 14.3 迁移中确认的宿主侧缺口（provider 侧后续可补）

- docker：`containerStats` 缺 networks/blkio；`containerTop` 不能传 `psArgs`；
  网络列表返回 12 位短 id，`inspect/remove` 原样透传（daemon 不认短 id 会 404）；
  日志只能轮询，没有 follow 通道。
- mqtt：`client_addr`、主题级 `message_count` 恒为 `None`；`metrics` 无连接建立时间，
  速率只能页面采样差算。
- rocketmq：`metrics` 无消费组数/消息总量；`clusterOverview` 只回主节点（无 role/磁盘）；
  `MessagePage.total` 是本页条数，`has_more` 恒真；三个查询都强制 topic；
  `sendMessage` 的字段是单数 `tag`/`key`。

### 14.4 nsh/TSX 的语义坑（写页面前必读，已写进 [`nsh-page-guide.zh-CN.md`](nsh-page-guide.zh-CN.md)）

组件体只在挂载时执行一次（顶层 `if (...) return` 会永久停在加载态）；
`onChange={sig.set}` 抛 `not a live signal`；`numberOfLines` 是 prop 不是 style；
`Each` 不从 `"nsh"` 导出、不能传 `key=`；主题颜色写在模块级常量里会冻住（要 getter）；
百分比宽度只能 `` `${Math.round(p*100)}%` ``。

### 14.5 给后续接手的人

- 写页面看 [`nsh-page-guide.zh-CN.md`](nsh-page-guide.zh-CN.md)（页面工程结构、
  `navop.*` 命名空间表、两种页面形态、设计系统、10 条坑、测试与交付清单）。
- 宿主侧入口：`crates/universal-plugins/src/nsh_host/{mod,tab,session,abi,bundle,page_host,connection,monitor,dev,dispatch,grant,theme}`。
  加一个 `navop.*` 能力 = 在 `abi.rs` 注册 handler + 在 `nsh-page-guide` 的命名空间表
  里登记；加一种页面形态 = 看 `tab.rs`（tab）与 `page_host.rs`（嵌入页）。
- 别再加回 gpui-shell：`crates/resource_view` 的 `CustomPageHost` 是引擎无关契约，
  换/加第三套 UI 平面时才需要动它。

## 15. 再收敛：一个 tab 视图 + 最小 API（2026-11-03）

> **实现状态（2026-11-04）**：§15 的「一个插件一个 tab 视图」已落地为「一个插件
> 一个 nsh app + 一个 tab（连接型按连接复用）」，`host.params` 增加 `route`，
> 页面在 app 内自行导航；`shellViews` 已删除。docker/mqtt/rocketmq/elasticsearch/
> dev-tools 五个复合扩展均已迁移（22 个页面 → 5 个 app）。

用户决定（原话）："整个插件机制我想改一下，就直接给一个 tab 视图，让用户自由发挥
就行，不再保留目前的工作台，原生加脚本的混合机制了，只保留最简单的 api 就行，
给用户足够的定制化需求"。

### 15.1 删掉了什么

- **原生资源工作台**：`contributes.resourceWorkbenches` 整块声明式 UI（表格/树/
  表单/路由）不再存在。清单里这个键被 serde 忽略（旧扩展仍能注册，但那些页面
  不会出现）。
- **`navop.workbench.*` 通道**：`current` / `dispatch` 都没了，连同宿主侧
  `WorkbenchContext`、`dispatch_invoke_scoped` / `dispatch_job_scoped` 的调用、
  `BindingContext` 的 route/selection/paging/parent 注入。
- **headless 连接 tab**（`ExtensionConnectionTab`，609 行）：连接只有一种形态，
  就是打开自己的页面；`decide_runtime_change` 留在 `nsh_host/decision.rs` 供页面
  tab 复用。
- **嵌入页机制**（`CustomPageHost` / `page_host.rs`）：页面不再有"从工作台里
  嵌进来"的第二种形态。

### 15.2 留下了什么

`navop.*` 只剩：`context.current`、`resource.{open,invoke,ping,close}`、
`job.*`、`event.*`、`blob.*`、`runtime.info`、`log.*`、`dev.*`，外加宿主推送的
`theme` / `i18n`（页面 token 依赖它们，属于"宿主事实"而不是"插件机制"）。

`navop.resource.invoke` 现在替页面解码 provider 的 `ResultRef`：inline 直接给值，
blob 走分片读并按 8 MiB 上限拼回 JSON，事件流给句柄。页面拿到就是普通数据。

### 15.3 页面侧的影响

- 一页一个 `dist/bundle.js`，数据全靠 `navop.resource.invoke({handle, method,
  params})`；句柄来自 `navop.context.current().connection.resourceHandle`。
- 共享库 `ui/lib/host.ts`（`connection()` / `call()`）与 `ui/lib/test-host.ts`
  （`hostHandlers()`）承担这层仪式，页面里不重复写。
- **页面自己决定"当前对象"**：docker 的 6 个容器级页面（进程/变更/挂载/统计/日志/
  详情）过去由宿主注入容器 id，现在页面自己拉容器列表、自己放选择器。这是这次
  简化的真实代价，也是"给用户足够定制化"的直接体现。
- docker 新增「容器」页（列表 + 启停/重启/暂停/删除），补上被删掉的工作台表格页；
  连接默认打开它。

### 15.4 与 §14 的关系

§14 记的是"把 UI 平面从 gpui-shell 换成 nsh"；本节记的是"换完之后再把插件机制
本身砍到最小"。§14.2 里"nativeModule/nativeComponent 没有用""`set_ui_sets` 只用
默认集"这两条在这里变成定论：不再计划引入，页面就是纯 TSX + 数据通道。

## 16. 原生组件（nativeComponent）

页面是 TSX，凡是「必须是 Rust 实体」的东西 —— 终端、编辑器、图表面板 —— 只能以
原生组件出现。这一节记录接法，因为它是这套机制里唯一**跨宿主层**的缝。

### 16.1 方向

注入方向是「注册表在扩展宿主、实现在应用层」：

- `crates/universal-plugins/src/nsh_host/native.rs` 只有注册表：
  `register_native_component(name, factory, cx)` + `NativeViewContext
  { extension_id, view_id, allow_process, component }`；
- `NshTab` 建 `AppView` 时调 `native::components_for(&contribution, cx)` 拼出
  **本页面**的 `Components` 交给 `AppOptions.components`（不是进程级
  `set_ui_sets`：那样一页注册的名字会漏给所有页面）；
- 真正的实现在 `crates/shell_domain/src/native_terminal.rs`（终端），
  由 `shell_domain::init` 调 `install(cx)` 注册 —— 扩展宿主不依赖
  `terminal_view`，应用层才认识它。

### 16.2 权限

`allow_process` = 该页面所属扩展的清单里有没有 `shell:exec`。没有时组件渲染
一行错误而不是静默空白。`navop.terminal` 起的是宿主本地进程，命令与参数必须由
页面显式给出（不是"用户随便敲命令"的入口）。

### 16.3 生命周期

- 组件每次建实例都能拿到 `NativeViewContext`（含页面身份），props 走 `set_prop`；
- 改 `command`/`args`/`env`/`cwd` 触发**重开** pty（页面上的「重新连接」就是改
  一个环境变量），只改 `title` 不重开；
- 组件 drop 时主线程拿不到 `App`，所以终端走的是一条回收队列：drop 里把
  `Entity<TerminalView>` 投给 `smol::channel`，主线程任务补一次
  `shutdown_embedded`（注销广播输入、释放连接占用、关 pty）。

### 16.4 同时删掉的旧机制

- `crates/resource_view` 整个 crate（3228 行原生工作台 + `TerminalHost` 契约）；
- `extension_plugin_adapter::workbench_dispatch`（`dispatch_invoke*/dispatch_job*`
  与 `BindingContext` 那一套）；
- `extension-runtime::registration` 里的 `validate_shell_renderer` /
  `validate_resource_workbench` / `validate_workbench_layout` /
  `validate_route_bindings` / `validate_tree_children` / `TREE_CHILDREN_MAX_DEPTH`
  与它们唯一的测试模块（`tree_children_tests`，10 条）；
- `shell_domain::workbench_terminal::{NativeTerminalHost, install_terminal_host}`
  （保留 `install_workbench_terminal_provider`，那是 AI 工作台的面板，与扩展无关）。

**当时还留着、后来也删干净了**：`extension-runtime` 里 `ResourceWorkbench*`
的清单类型（`contributes/resource_workbench.rs` 675 行、`types/resource_workbench.rs`、
`types/workbench_operation_catalog.rs`）与只测它们的 `tree_children_tests` 模块，
以及 `InvalidResourceWorkbench` 错误变体。现在清单里既没有字段也没有类型，
`catalog.rs` 的工具箱过滤不再有「工作台页体」这个例外。

## 17. 页面 host ABI v2（已完成，2026-11-03）

§6 里「`host.set_params` 替代 `navop.context`」当时只写了、没做：v1 的落地形态是把
gpui-shell 的 21 个 `HostModule` 函数原样摊平成 `host.call("navop.*")` 字符串调用，
nsh 的四类桥接只用了一类。本节记录补齐后的形态，实现细节与逐条契约见
[`host-abi-v2.md`](host-abi-v2.md)，页面作者看
[`nsh-page-guide.zh-CN.md`](nsh-page-guide.zh-CN.md)。

- **启动事实**走 `host.params`（`host.set_params` 在首帧前写入，页面同步可读），
  不再有一次 `navop.context.current` 的启动 RPC；
- **宿主 → 页面**只有 `host.on("navop.event")` 一组推送（事件泵在宿主侧，
  单批 ≤64、溢出 `EVENT_OVERFLOW` 带 `since` 续订），删掉轮询的 `navop.event.read`；
- **页面 → 宿主**：动作走 `host.call`（resource / job / blob / event / runtime /
  dev），日志走 `host.post("navop.log")`，不再一次日志一个 promise；
- **错误**改走 nsh 原生的 `JsError::Fields`：页面直接 `e.code` / `e.retryable` /
  `e.details`，`__NAVOP_ERROR__` base64 信封删除。为此在 nsh 侧加了
  `HostCall::reject_error` / `HostReply::reply_error`（旧 String 入口保留，
  `with_host_code` 兜底成 `ERR_NSH_HOST_ERROR`）；
- **删掉的模块**：`context` / `i18n` / `theme`（语言用 nsh 的 `locale()`，明暗用
  `colorScheme()` / `token()`）；清单里再写它们会得到「ABI v2 已删除」的安装期错误；
- **权限**单层化：`granted` 由 `modules` + `permissions` 机械派生，`declared = granted`
  由宿主权威写入，源码与 `.nshpkg` 两条加载路径都 `harden = true`；
  `dev` 模块只给内置扩展 `com.navop.devtools`。

仍未做（维持 §14.2 的结论，不在本次范围）：扩展包改走 `.nshpkg` 签名分发、
navop 的 nsh 依赖与仓库 `nsh` CLI 对齐（当前 CLI 0.3.0 比内嵌引擎新，
`numberOfLines` / `Input.onChange` / `nsh/base` 的 `Text` 等 UI 差异是改造前就有的）。
