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
| M6 试点迁移 | shell-view-example、js-toolbox(3)、docker overview 以 nsh app 形态重写 | 试点页全功能；产出方言映射表（gpui-component → nsh 组件） |
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
| 31 页全量重写人力 | 中 | M6 产出方言映射表后机械化；nativeComponent 桥兜底复杂页面 |
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
