# navop × nsh：全项目公共抽取目录与迁移蓝图

> 状态: 提案（基于 dev@5ab4bfced 的全仓扫描）
> 姊妹文档: [nsh-migration-plan.md](extension-resource-plugins/nsh-migration-plan.md)（已完成的扩展页层迁移）、[host-abi-v2.md](extension-resource-plugins/host-abi-v2.md)
> 原则: **先抽公共、再迁页面**——「逐文件直译」是反模式；组件写一次，页面薄化。

---

## 0. 扫描口径与全仓画像

扫描范围 `crates/ apps/`，工具 rg + wc，基线 dev@5ab4bfced。

| 维度 | 数值 |
|---|---|
| Rust 文件 / 总行数 | 2,205 / 934,151 |
| 触 gpui_component 的文件 | 455（含 one_ui） |
| `impl Render` | 227 / 166 文件 |
| 内嵌 nsh（宿主侧） | 16 文件（universal-plugins/nsh_host 12 + shell_domain 2 + extension-runtime 2） |
| i18n | 32 crate 挂 `rust_i18n!`；30 个 yml ≈ 5,600 顶层键 |
| one-assets 图标 | 256 个 24×24 lucide svg（stroke=currentColor，可零改动进 nsh） |

方言词汇频率（迁移面直觉）:

| 模式 | 次数 / 文件数 | 对应 nsh 手段 |
|---|---|---|
| `div(` | 2,633 / 303 | `view()` |
| `h_flex()/v_flex()` | 1,941 / 264 | `view().flex_row()/flex_col()` |
| `.child/.children` | 7,852 | `.child()` / `each()` |
| `.when` | 875 | `when(cond, || …)`（分支真建/毁） |
| `.id(` | 725 | nsh 无 div id（无全局元素 id） |
| `.on_click` | 1,297 / 215 | `.on_press` / `cx.emit` |
| `on_click(cx.listener` | 581 | `cx.emit` + `on_event` |
| `.listener(` 全部 | 1,018 | signal/computed |
| `window.listener` | 198 | 同上 |
| `.tooltip` | 385 / 101 | nsh tooltip 组件 |
| `.hover` | 163 / 72 | `.hover(\|s\| …)` 同形 |
| `t!()` / `t!().to_string()` | 6,258 / 4,181 | `t!()` 响应式化（去快照） |
| `cx.theme()` | 1,843 / 250 | `theme()` 读取即订阅 |
| `cx.notify()` | 2,200 | signal.set（隐式） |
| `Entity<` / `cx.new` / `.update(cx` | 1,689 / 1,414 / 3,455 | signal/computed/effect |
| `cx.open_window` | 47 | `open_window(Window)` per-id 复用 |
| `FocusHandle` | 350 | focus().focus_handle() 体系 |
| `push_notification` | 504 | nsh Notification + toast 栈 |
| `cx.spawn` | 445 | task executor（tokio enterable） |

render helper 长尾（每处 ≤29 次但真实重复）: `render_toolbar`×29、`render_header`×20、`render_item`×18、`render_td/th`×61、`render_empty`×10、`form_row`×176、`render_search_bar`、`render_tab_bar`、`render_sidebar`、`render_footer`、`render_find_bar` …

---

## 1. 公共组件/函数抽取目录

分层模型: **L0 资产 → L1 语义基元 → L2 复合组件 → L3 页面**。纯数据（item 列表、tone、过滤函数）沉到方言无关 model crate，gpui/nsh 双方言共用；「枚举→颜色/资产」映射表每方言一份。

### L0 资产层 `one-assets`（现成，零改动）

- 256 个 lucide svg 已是 `stroke="currentColor"`，nsh `svg()` + `.text_color()` 直接染色。
- build.rs 扫描生成 `IconName` 枚举（实现 gpui_component::IconNamed）→ nsh 侧同法生成 `const IconAsset: ImageAsset` 表（`asset!()` 宏编译期嵌入）。
- **产出物**: `one_assets::icon_asset(IconName) -> ImageAsset`。

### L1 语义基元（新 crate `ui-model` 或并入 one_ui）

跨 crate 高频语义，nsh 侧对应「语义组件」：

| 抽取物 | 驱动数据 | 频率 | nsh 承接 |
|---|---|---|---|
| `ToolbarItem<A>` + `ToolbarTone` + `toolbar_items` 纯函数 | 4 个 toolbar 文件 975 行 | `render_toolbar`×29 | `nsh_base::toolbar`（WAI-ARIA roving focus）+ 自家 NavopToolbarButton |
| `tone_colors`（Primary/Success/Warning/Info） | 54 处 `.opacity(0.1x)` tint | toolbar 10 项 | `const fn tone(light,dark)->AdaptiveColor` 常量（gallery-rs 模式） |
| `IconButton`（role: Compact/Standard/Toolbar/Navigation） | one_ui/icon_button.rs 26 引用 | `Icon::new`×329 | `button()` 构造器 + role→尺寸预设 |
| `ContentState`（Empty/Loading/Error/Done 内容状态） | one_ui 10 引用 | render_empty×10、loading×96 | `nsh_base` 语义组件重铸 |
| `FormRow`（label + control + help + error slot） | `render_form_row`×176 | 表单类 269 Input | L2 `settings` 表单 |
| `EmptyState` / `Loading` / `ErrorBanner` | 84/96/10 处 | 空/错态页面 | ContentState 派生 |
| `SearchBar`（search_input 171 处/41 文件） | search_input | 269 Input::new | `input()` + 清除按钮 + 快捷提示 |
| `SidebarPalette`（154 处/12 文件局部调色板） | persistent_connection_sidebar | — | 升自定义 token `Theme::color(name)` 或保留 Prop 传色 |
| `NavopBadge/Pill`（badge 318 处/52 文件） | tone 语义 | Badge×318 | `text().bg().rounded_full()` 细粒度 |

### L2 复合组件层 `one_ui`（既有先例，重铸清单）

one_ui 是 navop 自抽复合组件层（104 文件引用），每个组件在 nsh 侧同 API 形状重铸——**页面调用点不变**：

| one_ui 组件 | 引用数 | nsh 对位 | 重铸难度 |
|---|---|---|---|
| `edit_table` 全家族（CellCoord/CellEditor/EditTable/EditTableDelegate/EditTableEvent/state.rs 4,436 行/find 面板） | 18 文件 | `nsh_base::data_table`（`#[component(methods=DataTableApi)]`，百万行级） | 高——编辑/查找/键位自定义 |
| `panel_header`（PanelHeader/Variant） | 111 | view+按钮+truncate 组合 | 低 |
| `status_bar`（StatusBar/Colors/StatusPresentation） | 104 | `nsh_base::status_line` | 低 |
| `resize_handle`×139/32 文件 | 13 | `nsh_base::resizable`（h/v_resizable、ResizeHandleProps、ResizableState serde） | 中 |
| `settings`（SettingField 111 处/23 文件、TableDisplaySettings） | 23 文件 | 自家 settings 重铸（label+control 行） | 中 |
| `icon_button` | 26 | button()+role | 低 |
| `marquee_text`/`short_label`/`signature_help` | 7/7/7 | text+overflow 动画 | 低 |
| `time`（DateTimePicker/TimePicker 各带 Event+State） | 双窗口 | `nsh_base::date_picker/time_field`/calendar | 中 |
| `picker_dialog`（PickerEntry/labels/open_picker_dialog） | 3 | `nsh_base::select`/`dialog` 组合 | 低 |
| `theme_geometry`（InterfaceDensity/LayoutSizeTokens/ThemeGeometry） | 82 | 自定义 Tokens 进程级注入（`set_tokens`） | 中——密度重排 token 矩阵 |
| `large_text_editor`/`ContentState`/fonts | — | editor/tokens | 低 |

### L3 外壳与深水组件（定制，不指望 base 对位）

| navop 模块 | 行数 | 说明与 nsh 路线 |
|---|---|---|
| `core/src/tab_container.rs` | 7,618 | tab 拖拽 DragTab 1201-1251/drag_over 4520,4681/on_drop 4527；→ `nsh_base::dock`（drag 原生、render_panel/tab/drop_indicator/drag_preview 全可定制、DockAreaState 持久化格式与 gpui-base 相同） |
| `navop_app/src/popup_window.rs` | 1,143 | 多窗口 → `open_window(Window)` per-id 复用+聚焦 |
| `markdown-editor` crate | 49,545 | 自有渲染层 → 走 nsh editor/rich_editor/syntax 或 WebView；**建议 WebView 承接**（prose 富文本生态） |
| `ai_chat_view` 消息流 | 73,047 行 crate（agent_view.rs 20,157） | virtual_list（sticky header、keys、empty、separator）+ markdown 渲染（同上） |
| `terminal` vte 栈 | terminal_view 61,849 | `nsh_base::term`（vte emulator + TerminalApi write/reset/text/focus） |
| `db_view/table_data` 网格 | data_grid 6,006+results_delegate 5,464 | DataTable 可编辑模式 + 窗口分页（百万行 60 列 demo 同型） |
| `remote_desktop` RDP/WebView | — | GPUI 后端先过渡；原生后端 Phase 5 PoC |
| `sftp_view` 双栏文件列表 | lib.rs 10,153 | file_list_panel 8,773 → Tree/virtual_list 组合 |

### 公共函数（非组件）

- `t!()` 消费侧: 4,181 处 `.to_string()` 快照 → nsh `t!()` 读取即订阅，去掉快照（**先在 gpui 侧不改**——迁移时自然消失）。
- `prompt*` 家族（134 处）: 纯函数+窗口流转，方言无关——沉 model crate 原样保留。
- `empty/loading/error` 分支: 统一进 ContentState 三态渲染。
- 表格渲染微 helper（render_th/td×61）: 收进 edit_table 重铸的 header/cell 渲染槽。
- 通知（504 处 push_notification）: 统一进 `nsh_core::Notification` + toast 栈，注意桌面系统通知 vs 应用内 toast 两条通道。

---

## 2. 迁移路线（基于公共组件）

前置事实（已核实）: 同一份 nsh Rust 组件代码可跑 GPUI 后端（AppView 嵌现有 gpui 窗口）或原生后端（AppKit/Win32/GTK4）；纯 Rust UI 不链 QuickJS（js 是 platform feature 可关）。渲染层已统一（同 feigeCode/gpui-fast release/v0.1.14）。

### Phase A — gpui 侧抽组件（不改运行时）

1. 建 `ui-model` crate: ToolbarItem/ToolbarTone/tone 色值/ContentState 数据/过滤函数——纯数据零依赖。
2. gpui 侧 one_ui 扩容: 把 render_toolbar×29、form_row×176、search_bar、empty/loading/error 收敛为 gpui 组件（IconButton 风格）。**抽完立刻被现有产品回归**——每个抽好的组件=nsh 待重铸孪生。
3. 不预抽的: Entity 搜索框、placeholder 缓存比对——状态管理类在 nsh 被 signal/computed 取代，gpui 侧预抽白干。

### Phase B — nsh 孪生层（并行开发，不切用户）

4. L0/L1 nsh 版: `one_assets::icon_asset` + NavopToolbarButton/IconButton/ContentState/FormRow/SearchBar（`#[component]`+Prop+Event derive）。
5. L2 重铸: edit_table→DataTable 桥、panel_header/status_bar/resize_handle/settings/time。
6. 验证场: apps/navop-data 的 db_tree/工具栏/表格视图走 AppView+AppSource::Rust 嵌入（同插件 tab 机制），**真实连接下跑换算率与 DataTable 负载**。

### Phase C — 逐域翻页（每域一 PR，可回退）

7. 顺序建议（依赖少→多）: notes/json_view/known_hosts → workspace_explorer/sftp 树 → redis_view → db_view 大视图 → terminal_view → ai_chat_view → navop_app 外壳。
8. 外壳翻转点: `navop_app/src/lib.rs` run_with: app.run→cx.open_window 段换 `run_app(AppSource::Rust(Root))`；托盘/URL scheme/单实例/更新走 nsh desktop.rs（Tray/SetTray、Root url_schemes）或保留 app 侧。
9. tab_container → nsh dock 持久化迁移（DockAreaState 格式兼容 gpui-base，可平滑换壳）。

### Phase D — 原生后端

10. 先 Linux 翻 GTK4（风险小），macOS AppKit，最后 Windows（RDP ActiveX+DirectComposition cloak 舞步需 PoC，`NAVOP_RDP_DISABLE_COMPOSITION` 环境变量语义保留）。
11. WebView: 原生后端各走 WKWebView/WebView2 composition/WebKitGTK6；Linux Wayland 黑屏坑（Xwayland+WEBKIT_DISABLE_DMABUF_RENDERER）预案。
12. 全链 gpui/gpui-kit/gpui-fast 出依赖图；32 位 Windows 顾虑消失（无 QuickJS）。

### 里程碑与验收

| 里程碑 | 验收 |
|---|---|
| M1 组件抽取 | gpui 侧 render_toolbar/form_row/empty 收敛，产品回归绿 |
| M2 nsh 孪生 | navop-data 一个完整域在 AppView 跑 7 天无退化 |
| M3 域翻页 | db_view 树+表格在 nsh 后端渲染，交互回归清单过 |
| M4 外壳翻转 | run_app 独立窗口 + 托盘/快捷键/deeplink 全通 |
| M5 原生后端 | Linux GTK4 全功能 + mac/win 灰度 |

---

## 3. 风险与开放问题

1. **i18n 翻转**: yml（key→locale→text）→ Translations JSON（locale→key）需 xtask 翻转 + Root::translations 挂载；5,600 键 × 30 yml。t!() 无命中返回 "{locale}.{key}"——上线前合同测试兜底（现有 i18n_locale_keys_contract.rs 扩展）。
2. **SidebarPalette 局部调色板**（154 处）: 升自定义 token（Theme::color(name)）or Prop 传色——per-case 决策，不强制一刀切。
3. **edit_table 键位/find/编辑态**: DataTableApi 方法面差（scroll_to_row/selection/…）需桥接层；navop TableKeybindings 语义保留。
4. **focus 体系**: 350 处 FocusHandle——nsh focus() 语义对齐工作量集中在外壳（tab 环/命令面板）。
5. **异步**: 445 cx.spawn + tokio 391 文件——nsh-core executor tokio enterable，迁移时闭包包一层 enter，逻辑不动。
6. **4 个巨文件**: agent_view.rs 20,157/sftp lib.rs 10,153/file_manager_panel 8,773/tab_container 7,632——各自单列迁移 epic，先拆 helper 再翻页。
7. **方言陷阱**: nsh when() 分支真建/毁（非 gpui 每帧求值）；Prop 闭包签名 `move||`；Points vs Pixels；无 div .id()——以首版 cargo check + 真实交互回归为准。
8. **React 惯性反转**: 组件只跑一次（OnceInstance）——任何「每帧重建」假设（placeholder 比对、t!().to_string() 缓存）都要反转为 signal 驱动。

---

## 4. 实施状态（2026-11-27）

已建立持续迁移目标，并落地第一个可编译的 nsh 并行应用：

- `crates/navop_nsh`：纯 Rust nsh UI，依赖 `nsh-core` / `nsh-base`，不引用 `one_ui` 或 `gpui-component` 视图。
- `apps/navop-nsh`：迁移期可执行程序。先启动现有 `app_kit` / 领域服务，再通过
  `nsh_platform_gpui::open_app_window(AppSource::Rust(...))` 打开 nsh 主窗口；共享服务经
  `nsh_core::element::EngineContext` 注入。
- 已完成 nsh 外壳与路由（21 个）：Workspace、Workspaces、Connections、Notes、JSON、
  Known Hosts、Database、Redis、MongoDB、SFTP、Terminal、Port Forwarding、Remote
  Desktop、AI、Host Monitor、Team、Toolbox、Session Logs、Credentials、Extensions、
  Settings。
- 已可用页面：
  - Workspaces：复用 `WorkspaceRepository` 创建、重命名和删除工作区，并统计每个工作区
    下的连接数量。
  - Connections：复用现有 `ConnectionRepository`，支持新建和编辑数据库、SSH、Redis、
    MongoDB、FTP/FTPS、Telnet、RDP、VNC 连接及删除；数据库支持 PostgreSQL/MySQL/MSSQL/Oracle/
    SQLite/DuckDB，SSH 支持密码、私钥和 agent 认证，网络服务连接支持 TLS。Telnet 编辑
    会保留已有 login script，RDP/VNC 编辑会保留代理、音频、只读、后端偏好和高级 RDP
    设置。新建连接可以选择工作区；编辑会保留本地 ID、团队/所有者、同步状态等身份元数据。
    串口连接支持端口、波特率、数据位、停止位、校验与流控；MQTT 支持 client ID、TLS 和
    clean session；SSH 支持默认目录/初始化脚本、keyboard-interactive/OTP、独立 SFTP
    账户、跳板机、SOCKS5/HTTP 代理、agent/X11 转发；SSH 隧道支持 Local/Remote/Dynamic
    三种模式。
    - Open/Terminal 会把目标连接作为一次性导航请求带入 Database、Redis、MongoDB、SFTP
      或 Terminal，目标页面进入后自动选中并尝试连接。
  - Database：注册全部数据库连接配置，读取 database/schema/table 目录；点击表直接通过
    新 `GlobalDbState::query_table_data_direct` 分页浏览数据，Structure 页显示列、索引和
    外键。SQL 编辑与执行继续走 `db::GlobalDbState`，不复制数据库驱动逻辑。
  - Notes：复用 `NotesStorage`，支持浏览、选择、创建、编辑、保存。
  - JSON：nsh 多行 Input + 实时解析/格式化/复制。
  - Known Hosts：复用 `HostKeyVerifier`，支持刷新和导入 OpenSSH known_hosts。
  - Terminal：nsh-base 负责 Local/SSH VT emulation/渲染/键盘/滚动；独立 PTY worker 负责 shell
    进程、读写和窗口尺寸；已保存 SSH 主机复用 `terminal::SshBackend` 建立裸交互
    channel，由 Tokio worker 泵入 nsh Terminal。会话工具栏可在 Local shell 与 SSH
    主机之间切换。串口/Telnet 复用 `terminal_view::TerminalView` 作为
    `navop.terminal` NativeComponent，保留协议协商、登录脚本和完整终端能力。Unix
    本地 PTY 已接通，Windows 暂保留明确的不可用状态。
  - SFTP：nsh 双栏文件浏览器；复用 `connect_remote_file_client` 处理 SFTP/FTP 建连，
    支持本地/远程目录浏览、单文件上传下载，以及当前目录递归上传/下载；目录上传支持
    `Merge` / `Replace` 冲突策略，递归遍历、冲突处理、进度和取消能力继续复用 sftp crate。
    页面显示文件级进度/速度，可取消活动传输，并通过 `SftpFavoritePathRepository` 管理
    常用远程目录。
  - Redis：nsh key browser；SCAN pattern/分页、String/List/Set/ZSet/Hash/Stream 值预览，
    DB 切换、删除键和原始 CLI 执行，连接继续使用 `redis-runtime`。
  - MongoDB：nsh database/collection 导航、JSON filter、limit 查询、文档
    Extended JSON 展示/新增/替换/删除、索引查看和聚合 pipeline；连接由
    `GlobalMongoState` 选择 native/IPC driver。
  - AI：ACP agent 选择、连接/首种可用认证、prompt、停止当前 turn、流式 transcript 和
    nsh TextView 渲染；工具权限请求会进入 nsh 审批卡，可选择 Allow/Deny，未响应时
    fail-closed；支持 ACP elicitation 表单/URL 流程和图片附件。
  - Port Forwarding：Local/Remote/Dynamic SSH 隧道通过 `navop.port_forwarding`
    NativeComponent 复用现有 `PortForwardingTab` 与运行时。
  - Host Monitor：复用 `terminal_view::HostMonitorSession`，对已保存 SSH 主机按需建立
    一次性安全会话并采集 CPU、内存、磁盘、网络、进程、核心数和 uptime。
  - Remote Desktop：新增 RDP/VNC 工作区。页面用 `navop.remote_desktop`
    NativeComponent 嵌入现有 `RemoteDesktopView`，协议、输入、剪贴板、缩放和帧呈现继续
    复用原 remote-desktop 栈；Connections 的 Open 会预选对应主机。
  - Credentials：复用加密 `CredentialRepository`，支持搜索、新建、编辑、引用扫描、
    Ed25519/ECDSA/RSA SSH key 生成和受引用保护的删除；支持
    用户名、密码、私钥路径/内容、passphrase、同步和 agent forwarding，敏感字段由现有
    仓库加密落盘，密码/passphrase/私钥输入使用 nsh 原生 masked Input。
  - Session Logs：复用 `terminal::recording` 的 favorites/catalog 扫描，展示后端、身份、
    时长、完整性和文件路径。
  - Extensions：启动时从 `GlobalExtensionRuntimeCatalog` 抽取 nsh app 快照，展示版本、
    分类、模块、权限和入口；点击 Open 后通过 `NshHost::open_embedded` 创建独立
    `NshTab`，再由 `navop.extension` NativeComponent 嵌入 nsh 外壳。扩展 app 的 provider
    激活、能力通道、主题和关闭回收继续复用 universal-plugins 宿主。Manage /
    Marketplace 入口通过 `navop.extension_manager` 嵌入现有扩展管理器，安装、更新、
    重载和卸载继续复用 extension runtime。
  - Team：读取 `get_cached_team_options` 的共享团队缓存，展示角色、成员状态和团队密钥状态。
  - Toolbox：提供已迁移工具和页面的 nsh 原生导航入口。
  - Settings：使用 nsh Input/Switch 编辑 locale、主题、字体、UI 缩放、终端字号/回滚、
    自动复制、会话日志、自动更新和云同步开关，校验后原子写回现有 `settings.json`。
- 主域占位页已清空。剩余工作是各域深度能力补全、TSX/simple-page 选型与最终外壳翻转。

### 外壳翻转（2026-11-27 完成）

- `crates/navop_nsh_app` 抽出了可复用的 nsh 应用装配层：产品身份、领域初始化、
  共享服务快照、native component 注册和主窗口 root。
- `navop_app::run_with_nsh` 复用完整产品启动管线：路径初始化、单实例转发、托盘、
  更新检查、窗口状态恢复、URL/文件启动事件仍由原宿主处理，只是主窗口内容改为
  nsh `AppView`。
- `apps/navop`（默认 Full 产品）和 `apps/navop-nsh` 都通过同一装配层启动；外壳已
  从旧 `NavopApp` 翻转到 nsh，同时继续使用 `gpui_component::Root` 承载浮层/主题/
  通知设施，避免 nsh 子树破坏宿主级 UI 基础能力。
- native component 注册已覆盖 `navop.extension`、`navop.extension_manager`、
  `navop.remote_desktop`、`navop.port_forwarding`、`navop.terminal`。

### 下一阶段验收顺序

1. **Connection follow-up**：补扩展连接表单、测试连接、默认打开方式、凭据引用和团队归属。
2. **Extension follow-up**：工具箱按 `extension_id + route` 启动、嵌入页焦点/Tab 顺序、
   页面内 `App.open` 到 nsh 自己的 tab 路由，以及真实扩展包运行验收。
3. **Terminal follow-up**：补 SSH shell integration/重连/分屏、搜索、复制粘贴、录制与
   Windows ConPTY；本地 PTY、SSH、串口和 Telnet 已可用。
4. **SFTP follow-up**：补传输队列多任务视图、文件级冲突逐个确认和远程编辑；
   双栏浏览、单文件/目录递归传输、进度/取消和收藏路径已可用。
5. **Redis / MongoDB follow-up**：Redis 补 tree/namespace 与 Pub/Sub；MongoDB 补 schema
   校验编辑。CLI、删除、DB 切换、文档 CRUD、索引和聚合已可用。
6. **AI follow-up**：补会话历史/切换、模型模式和 transcript 搜索；elicitation、
   图片附件、停止与基础 permission 审批已可用，ACP/runtime 不动。
7. **Credentials / Team follow-up**：补团队创建、成员管理和密钥轮换；SSH key 生成、
   引用扫描与受引用保护删除已可用。
8. **外壳收尾**：托盘、单实例、更新和窗口状态已复用产品启动管线；剩余把文件/URL
   打开事件映射到 nsh 页面，并把旧 tab 容器/资源侧栏迁移为 nsh dock。

### 当前验证命令

```sh
cargo check -p navop-nsh
cargo test -p navop_nsh --lib
cargo clippy -p navop_nsh -p navop-nsh --no-deps --all-targets -- -D warnings
cargo build -p navop-nsh
cargo run -p navop-nsh
```

2026-11-27 实测：`cargo test -p navop_nsh --lib` 49 个测试通过；
`cargo check -p navop -p navop-nsh -p navop_nsh_app` 与
`cargo clippy -p navop_nsh -p navop_nsh_app -p navop-nsh -p navop --no-deps
--all-targets -- -D warnings` 通过。启动冒烟受本机 Xvfb 无 GPU adapter 限制，
需要在带 GPU/软件渲染的桌面会话做视觉与交互冒烟。

Database 页落地后的补充验证：`cargo check`、`cargo build`、4 个 lib 测试和
`clippy --no-deps -D warnings` 均通过。数据库连接配置在 host 初始化阶段注册进
`GlobalDbState`；目录和查询执行使用 nsh task + Tokio background，结果通过 nsh `DataTable` 渲染。

Terminal 页落地后的补充验证：6 个 lib 测试通过，其中包括真实 `/bin/sh` PTY 输出回归；
`cargo check`、`cargo build`、`clippy --no-deps -D warnings` 均通过。nsh Terminal 持有 emulator，
PTY worker 只跨线程传输输入、输出和尺寸，页面销毁时通过 cleanup 终止子进程。

SFTP 页落地后的补充验证：7 个 lib 测试通过，`cargo check`、`cargo build`、
`clippy --no-deps -D warnings` 均通过。连接仍由 `sftp_view::connect_remote_file_client`
统一解析，页面只消费 `RemoteFileClient` 的协议无关文件操作。

Redis 页落地后的补充验证：9 个 lib 测试通过，`cargo check`、`cargo build`、
`clippy --no-deps -D warnings` 均通过。host 在启动时一次性解析需要密码簿/隧道展开的连接快照，
nsh 页面直接使用 `RedisConnectionImpl` 的 SCAN 和 typed value API。

MongoDB 页落地后的补充验证：11 个 lib 测试通过，`cargo check`、`cargo build`、
`clippy --no-deps -D warnings` 均通过。连接与 driver 选择留在 `GlobalMongoState`，
页面只通过 `MongoConnection` 执行 database/collection 导航、find/count 和 JSON 渲染。

AI 页落地后的补充验证：14 个 lib 测试通过，`cargo check`、`cargo build`、
`clippy --no-deps -D warnings` 均通过。ACP agent 配置在 host 解析，nsh 页面通过
`connect_with_runtime_and_providers` 连接；Tokio 事件流和 permission provider 都经 channel
泵入 nsh signal，工具审批在页面内完成。

公共页面补齐后的补充验证：`navop_nsh` lib 测试扩展到 37 个并通过；`cargo check
-p navop_nsh -p navop-nsh` 与 `cargo clippy -p navop_nsh -p navop-nsh --no-deps
--all-targets -- -D warnings` 通过。新增 `GlobalDbState::query_table_data_direct` 的 Rust
contract + SQLite 真实分页测试通过；Connections 新建数据库连接、Settings 数值校验、
SSH/Redis/MongoDB 连接构建、更新身份保留、Workspaces 颜色解析、Credentials CRUD 草稿、
Extensions 搜索、Session Logs 时长格式化、Host Monitor 速率格式化和 Team 状态标签均有
定向单元回归。

Terminal SSH 桥接的补充验证：真实 `/bin/sh` PTY 回归、SSH 字节流 pump 的 fake channel
测试覆盖输出、输入、resize 和断开状态，`navop_nsh` lib 共 33 个测试通过。

扩展页嵌入链路已接通并通过 `navop_nsh`、`navop-nsh`、`universal-plugins --features
nsh-host` 的编译与 clippy 校验：`NshHost::open_embedded` 生成独立 `NshTab`，
`navop.extension` NativeComponent 将它画进 nsh 页面。当前环境没有 GPU adapter，且本地
未安装可做交互验收的 nsh extension fixture，因此真实扩展包打开/键盘焦点仍需在图形桌面
环境补冒烟。

2026-11-27 深度能力补齐后的验证：`navop_nsh` lib 测试扩展到 49 个并通过，覆盖高级
SSH/跳板机/代理/OTP、MQTT、串口、Port Forwarding、Redis CLI/DB、MongoDB 文档/聚合、
AI elicitation/图片附件、SFTP 进度、SSH key 参数和终端 serial/Telnet 路由。新增
`navop.terminal`、`navop.extension_manager`、`navop.port_forwarding` 三个
NativeComponent；复杂协议/桌面能力继续复用现有领域实现，简单页面保持 nsh 原生 Rust。
