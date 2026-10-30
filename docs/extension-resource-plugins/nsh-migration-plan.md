# nsh 扩展 UI 平面迁移方案（gpui-shell → nsh）

- 状态：提案（v1，2026-10-30）
- 范围：navop 扩展机制的 **UI 平面** 从 gpui-shell 迁移到 nsh（nsh-platform-gpui）；**能力平面**（provider runtime / ActivationManager / lease / RuntimeMonitor / ProcessRpcSession）原样保留，不在本方案范围内。
- 前提（用户已确认）：扩展尚未正式使用，页面可任意重写；gpui-fast 与 nsh 均已 fork（feigeCode），长期与官方保持同步。

---

## 1. 结论先行

| 问题 | 答案 |
|---|---|
| 迁移可行吗？ | **可行**。nsh 的 GPUI 后端就是同一个 gpui-fast，AppView 是普通 GPUI Entity，可与 navop 的 TabContainer/Root 同窗共存；宿主 overlay（dialog/popover/notification）画在其上。官方 examples/embed-gpui 已验证该嵌入模式（Longbridge Pro / Allsum Desktop 同款场景）。 |
| 会有两份 gpui 吗？ | **不会**。nsh-fork/crates/gpui 只是 GPUI 后端子工作区（成员 nsh-platform-gpui、nsh-gpui-host），其 `Cargo.toml:53-54` 依赖 `gpui-fast 0.1.9`（crates.io）；而 navop 的 `[patch.crates-io]`（`Cargo.toml:393-425`）把 gpui-fast* 全部钉到 feigeCode/gpui-fast `release/v0.1.12`。cargo 的 patch 机制会把 0.1.9 的需求统一解析到 0.1.12：**全进程一份 gpui-fast、一个 App/事件循环/渲染管线**。会出现两份的唯一做法是绕开 patch 同时拉 0.1.9 与 0.1.12（同名不同版本可共存但类型不互通）——本方案不这么做。 |
| 代价在哪？ | 不是 gpui 重复，而是引入**第二套 UI 引擎/组件模型**：nsh 自带 35k 行自绘层 + 组件集 + engine 线程/QuickJS VM。成本集中在：31 个扩展页的方言重写（view 层重写、逻辑层可搬）、navop.* host ABI 重接（HostModule → HostChannel handlers + native components）、错误/取消/授权映射、manifest 版本门更换。 |

---

## 2. 依赖与版本策略

### 2.1 现状

- navop（workspace root）：gpui-kit 系 5 crate 钉 `feigeCode/gpui-kit` rev `62035f8d…`；`[patch.crates-io]` 把 gpui-fast* / gpui-pre-* 全部钉到 `feigeCode/gpui-fast` branch `release/v0.1.12`。
- nsh-fork（branch `feat/panel-window`，HEAD c78c678）：crates/gpui 子工作区依赖 crates.io 的 `gpui-fast 0.1.9`。nsh 主工作区（nsh-engine/nsh-js/nsh-base/nsh-bundler 等）全是 path 依赖。

### 2.2 目标态：navop 如何引 nsh

```toml
# navop/Cargo.toml（workspace root）
[workspace.dependencies]
nsh-platform-gpui = { git = "https://github.com/feigeCode/nsh", rev = "<钉死>" }
# default-features = false, features = ["js"]（按需再加 webview）
```

要点：

1. **git 依赖引 nsh-platform-gpui**（feigeCode fork，钉 rev），`default-features = false` 只开 `js`。nsh-platform-gpui 会把 nsh-engine/nsh-base/nsh-core/nsh-js 一并拉进来（主工作区 path 依赖）。
2. **不需要动 [patch.crates-io]**：navop 现有 patch 天然把 nsh 侧 0.1.9 的需求统一解析到 release/v0.1.12。全进程仍是一份 gpui-fast。
3. **nsh fork 上需要一个编译补丁**（对 0.1.12 编译，详见 §2.3）：改 `nsh-platform-gpui/src/chrome.rs:1315`。改法以 0.1.12 API 为准，作为 fork 补丁维护，同时向 longbridge/nsh 提 PR（兼容新旧 gpui-fast 的写法：旧版无 `cx` 参数需 cfg 或直接以新 API 为准）。
4. navop 升级 gpui-fast 时，需同步验证 nsh fork 仍编译（两个消费方共用同一 patch 面）。这是一个持续义务，写进维护清单。

### 2.3 nsh fork 补丁（唯一必改点）

`nsh-platform-gpui/src/chrome.rs:1315` 是 nsh 唯一的生产 `KeyBinding::new` 调用点：

- gpui-fast 0.1.12 的 `KeyBinding::new(keystrokes: &'static str, …)` 要求 'static 且 panic on parse error，`&gpui_keystroke(&s)`（&String→&str，运行时拼接串）编译不过。
- 改法：`gpui::KeyBinding::load(&s, Box::new(action), predicate, …, cx)`，解析失败跳过该 binding（勿 unwrap/panic）。`gpui_keystroke()` 返回类型不变。
- 另一处 `tests.rs:486` 的 `Keystroke::parse` 仅 `shot` feature 测试用，可同步修或忽略。
- **官方状态（2026-10-30 核实）**：upstream longbridge/nsh main（HEAD 201d780）未修——但对官方这不是 bug：upstream 自己钉 `gpui-fast 0.1.9`（其 crates/gpui/Cargo.toml:53），0.1.9 的 `KeyBinding::new` 接受 `&str`，官方组合下编译通过。断裂仅出现在 navop 的 `[patch.crates-io]` 强制统一到 0.1.12 的组合下，属于「我们领先官方一个 gpui-fast 版本」的适配成本。上游要到自行 bump 0.1.12 时才会撞上；PR 应定性为「适配 gpui-fast 0.1.12」而非 bug fix，也可以纯 fork 补丁钉 rev 维护、等官方 bump 后对齐作废。

---

## 3. 目标架构

### 3.1 总览

```
┌────────────────────────── navop 主进程（一个 GPUI App）──────────────────────────┐
│                                                                                   │
│  能力平面（不动）                        UI 平面（本方案重做）                      │
│  ┌───────────────────────┐             ┌──────────────────────────────────────┐ │
│  │ ExtensionRuntimeCatalog│            │ NshPluginHost（新，替代 ShellPluginHost）│ │
│  │ ActivationManager/lease │            │  · NshShellViewOpener: ShellViewOpener │ │
│  │ RuntimeMonitor          │   注册     │  · host ABI: navop.* → HostChannel     │ │
│  │ ProcessRpcSession       │───────────▶│  · Policy/grant → nsh Policy/Caps     │ │
│  └───────────────────────┘             │  · 主题/i18n 桥（push 模型）            │ │
│                                        └──────────────┬───────────────────────┘
│                                                         │ AppView::new + load(Bundle)
│  ┌────────────────────────── GPUI 窗口（TabContainer/Root + 宿主 overlay）────────┐│
│  │  Tab: NshTab ──── Entity<AppView>（nsh surface：自 layout/paint + 自 engine 线程）││
│  └──────────────────────────────────────────────────────────────────────────────┘│
└───────────────────────────────────────────────────────────────────────────────────┘
```

核心变化只有一处：Tab 内容物从 `Entity<ScriptView>`（gpui-shell 引擎在同一 GPUI 线程跑脚本）换成 `Entity<AppView>`（nsh 引擎在独立线程跑，surface 只借 GPUI 的 WindowTextSystem 塑形/IME/FocusHandle/输入事件）。装载路径 `ShellViewOpener` trait（crates/extension_view/src/shell.rs:6-10）本来就是引擎无关设计，新增 `NshShellViewOpener` 实现，push `TabItem` 到 `GlobalTabContainer::primary_pane()` 的流程不变。

### 3.2 NshTab 生命周期

```
open:  NshPluginHost::open_view(ext, view)
         → 组装 Bundle（source 或 bytecode）+ Policy + HostChannel handlers
         → AppView::new(AppOptions{data_dir, host, components…}, window, cx)
         → view.load(App::Js(bundle))
         → push TabItem（content_key "NshExtension"）
idle:  主题/外观/locale 变化 → set_theme / set_appearance / set_locale + host.send("theme"/"locale", …)
close: session.cancel()（取消所有在途回调）→ view.stop()【后台化，勿在 UI 线程 join】→ drop Entity → release_after_session
```

- **stop() 阻塞**：engine 线程 join 可能卡 UI，close 路径用 `cx.background_executor().spawn` 后台执行，UI 侧先摘 Tab。
- **窗口参数**：宿主窗口需 `inactive_frame_interval: None`，否则非聚焦窗口 30fps 会让 nsh 动画减半（AppView 文档 app.rs:200-233 明示）。
- **多视图**：一个窗口可挂 N 个 AppView（各自独立 engine 线程/VM/data_dir/HostChannel），官方 embed-gpui 示例已验证。进程级共享面见 §6 风险。

### 3.3 host ABI：navop.* → HostChannel

现有 11 个 navop.* HostModule（context/i18n/theme/log/runtime/dev/resource/job/event/blob/workbench，30 函数）映射为 HostChannel handler：

```rust
// Rust 侧（NshPluginHost 内）
let host = HostChannel::new();
host.handle("navop.resource.invoke", |call| {
    // call.args: {backend, action, payload} —— 与现有 resource HostModule 同语义
    // 异步：在 tokio 上跑 spawn_provider_task，完成后 call.resolve(json)
    // 取消：HostCall 语义是 drop = reject JS promise —— token 挂到 call 的守卫上
});
host.set_params({ext_id, view_key, locale, theme_revision, …}); // 启动参数，替代 navop.context
// JS 侧不变的概念：await host.call("navop.resource.invoke", {...})
```

| 现有机制 | nsh 对应 | 说明 |
|---|---|---|
| HostModule 函数（同步/异步 closure） | `HostChannel::handle(name, Fn(HostCall))` | handler 跑在 engine 线程，勿阻塞、勿直接碰 App/窗口；需要 App 侧操作的走 rt 队列（nsh 自带）回主线程 |
| HostAsyncTask + CancellationToken | HostCall resolve/reject + drop-reject 语义 | 取消传播：close 时 session.cancel() 逐个 reject 在途 call；JS promise 收到 reject 即结束 |
| 错误结构化 envelope（`__NAVOP_ERROR__` + Base64URL in HostError::message） | reject(json) 直接塞结构化 JSON | 比现在更干净，无需 Base64 信封 |
| navop.theme pull 模型（脚本 render 时读快照） | `set_theme`（宿主→引擎 push）+ JS 读 token | 复用 embed-gpui theme.rs 模式：gpui-kit Theme → shadcn 同名 token + radius/font，~90 行 |
| navop.i18n pull 模型 | `set_locale` + `host.send("locale", {locale, revision})` 事件 | push 模型替代 pull；页面监听事件自行重渲染（nsh 响应式自动处理） |
| navop.context（app 信息） | `HostChannel::set_params`（启动参数） | view_key/ext_id 在 params 里，JS `host.params` 读取 |
| with_current_app 17 处 | 不再需要 | nsh 引擎不在 GPUI 线程，宿主栈不变量消失；需要主线程的走 nsh rt 队列 |

### 3.4 页面形态与构建

- **方言**：`import { View, Text, Scroll, List, signal, Each } from "nsh"` + JSX + style 对象（flexDirection/paddingHorizontal/gap）。view 层必须重写；逻辑层（navop.* 调用、状态管理思路）可搬。nsh-base 组件集（Button/Checkbox/Menu/Popover/Select/Switch/Table/Tooltip/Input/NavStack/Dialog/RichText/markdown…）覆盖桌面扩展页常用控件。
- **构建**：nsh-bundler 纯 Rust（oxc），npm-less；或者宿主运行时直接 `Bundle::source(src)` / `nsh_js::compile_bytecode` 进程内编译。建议：**开发期 source 热载（connect_dev_server），发布期 bytecode**。
- **缺组件时**：用 `AppOptions.components`（NativeComponent）把 gpui-kit 真控件桥进 nsh 页面（embed-gpui switch.rs 模式：`NativeComponent::render` 里画 gpui_kit 控件，JS 侧 `nativeComponent("host.xxx")`）。这是渐进迁移的救生索。

### 3.5 manifest 与激活

- `engines.gpui_shell`（schema.rs:49-53，宿主常量 GPUI_SHELL_VERSION="0.6.4"）→ `engines.nsh`，版本门常量改为 nsh runtime 版本（宿主引的 nsh-engine semver）。
- `contributes.shellViews` 结构保留：entry 指向 nsh 页面入口（app/ 目录或单文件），surface(tab|toolbox)/singleton/backends/modules 语义不变。ShellHostModule.requires_backend 语义映射到 HostChannel handler 名或 native component 名。
- 激活/lease/monitor 流程不变：activation 仍是打开 view 前 acquire lease → 构建 Bundle → AppView → 注册到 catalog；close 时 release。RuntimeMonitor 语义延续。

### 3.6 隔离与授权

- **per-view Policy** 由 navop 组装：`harden: true`（发布包）、`declared`（从 manifest capabilities 声明读）、`granted`（宿主上限，从现有 grant.rs 语义映射：fs:read/fs:write→读写根、net→network_hosts、spawn→ExecuteGrant 白名单 basenames；secrets/shell:exec/host/database/ui 不授予——ui 在 nsh 里天然不需要）。
- **storage**：nsh AppOptions.storage 接 `Arc<dyn StorageBackend>`，落 `<data_dir>/extensions/<ext>/<view>/store.json`（沿用现有路径布局，仅 singleton 视图）。
- **data_dir**：沿用 `<data_dir>/extensions/nsh/<ext>/<view>`（nsh 惯例是 `<dir>/<app_id>`，用 app_id=view_key 即可对齐）。
- **native 组件**：进程级 UI_SETS 只放公共集（nsh_base::registry() + navop 桥接件）；per-view 扩展用 AppOptions.components。注意 UI_SETS 进程级共享，见 §6。

---

## 4. 里程碑

| 阶段 | 内容 | 验收 |
|---|---|---|
| M1 fork 补丁 | nsh fork 改 chrome.rs:1315（KeyBinding::load + 失败跳过），向 longbridge/nsh 提 PR | nsh-platform-gpui 在 gpui-fast 0.1.12 下 cargo check 通过 |
| M2 spike | navop 里挂一个 AppView tab，跑通 nsh 官方示例页 | tab 内渲染、主题桥接（~90 行）、Tab 键进出 surface、宿主 overlay 盖在其上 |
| M3 host ABI | navop.* 11 模块 30 函数接 HostChannel；取消/错误信封；set_params | JS 页面 await host.call 调通 resource.invoke 全链路（provider plane 不动） |
| M4 接线 | manifest engines.nsh + NshShellViewOpener + 激活/lease/monitor + NshTab 生命周期（close→stop 后台化） | 从扩展目录装一个 nsh 扩展、开 tab、close、lease 释放 |
| M5 试点迁移 | shell-view-example + js-toolbox(3) + docker overview 页 | 试点页全功能可用，记录方言映射表（gpui-component→nsh 组件对照） |
| M6 全量迁移 | 剩余 31 页 + 测试基线重建（extension_pages_render_tests 改 nsh shot/test-support 形态） | 全部页面渲染断言通过 |
| M7 收尾 | 移除 gpui-shell 依赖与 rev 62035f8d 的 kit 依赖降级；更新 docs/extension-resource-plugins/* | cargo tree 无 gpui-shell；文档同步 |

M1–M2 是去风险步骤：如果 AppView 与 navop 的 TabContainer/overlay 有未知冲突，在最便宜的阶段暴露。

---

## 5. 体量估算

| 项 | 现状 | 迁移后 |
|---|---|---|
| universal-plugins 脚本 UI 平面 | 5,140 行 | 引擎绑定 1,452 行重写为 NshPluginHost（估 ~1,200）；HostModule ~1,470 行改 HostChannel ABI（逻辑可留）；可移植 session/cleanup/monitor/connection/grant 930 行基本直搬；manifest 元数据 ~400 行只改版本门；main 胶水 ~270 行小改 |
| 扩展页面 | 31 文件 / 7,462 行（view 层重写，逻辑层可搬） | 逐页重写（M5 起按试点节奏） |
| 测试 | extension_pages_render_tests.rs 501 行 | 改 nsh shot/test-support 形态重建 |
| nsh fork 补丁 | — | chrome.rs 一处（+tests 一处可选），上游 PR 跟踪 |
| 新增桥接 | — | 主题桥 ~90 行；native component 桥按需（每个 ~50 行） |

---

## 6. 风险清单

| 风险 | 等级 | 缓解 |
|---|---|---|
| gpui-fast bump 需同步 nsh fork（两消费方） | 高 | patch 面收敛在 [patch.crates-io]；每次 bump 跑 nsh cargo check 进 CI；上游 PR 合入后减压 |
| UI_SETS / keybindings / rt 队列进程级共享 | 中 | UI_SETS 只放公共集；per-view 走 AppOptions.components；接受语义（扩展本就同信任级） |
| macOS 菜单 last-wins（多 AppView 时 chrome.rs:1339 clear_key_bindings+set_menus） | 中 | navop 扩展页不定义应用菜单，冲突面小；embed-gpui 同场景已验证 |
| stop() join 阻塞 UI | 中 | close 路径后台化（background_executor） |
| webview overlay 盖 GPUI、离屏抓不到；Linux 外 video | 低 | 扩展页暂无此需求；需要时再评估 |
| 窗口 inactive_frame_interval 未设 → 动画减半 | 低 | 宿主窗口统一设置 None；写进 NshPluginHost 初始化检查 |
| nsh package --target gpui 不支持（需自行打包验签） | 低 | 发布链路用 Bundle::from_package + Ed25519 验签（nsh-package），dev 用 source/dev server |
| 未来 gpui-fast 动态 keybinding 方向易碎 | 中 | KeyBinding::load 补丁模式固定；跟踪上游 0.2 动向 |
| 31 页全量重写的人力 | 中 | M5 产出方言映射表后机械化；native component 桥兜底复杂页面 |

---

## 7. 与现状的关键差异（一句话版）

- 引擎从「GPUI 线程内 QuickJS + frozen gpui-component catalog」变成「独立 engine 线程 QuickJS + nsh-base 组件集 + 可桥 gpui-kit 真控件」。
- 隔离从「共享 ShellRuntime + per-view Policy」变成「per-view 独立 VM/engine 线程 + nsh Policy(harden/declared/granted)」——**隔离强度提升**。
- 主题/i18n 从 pull 变 push，页面自动响应，无需 refresh_shell_views 全量重跑。
- 全进程 gpui 数量不变：**一份**。
