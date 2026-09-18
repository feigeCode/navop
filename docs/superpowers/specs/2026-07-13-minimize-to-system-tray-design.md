# 最小化到系统托盘设计

## 目标

为 Navop 桌面应用增加跨平台系统托盘能力。用户关闭主窗口时，应用隐藏到系统托盘并继续保留当前标签页、连接和后台状态；用户可从托盘恢复原主窗口，或通过托盘菜单进入现有的安全退出流程。

本功能覆盖仓库当前发布的 macOS、Windows 和 Linux 平台。实现必须保留现有退出确认、标签页关闭检查和显式退出快捷键语义。

## 用户交互

### 关闭与最小化

- 点击主窗口关闭按钮时，托盘可用则按用户偏好分流：
  - 偏好为「每次询问」（默认）：弹窗让用户当场选择「最小化到托盘」还是「退出应用」，
    弹窗内提供「记住我的选择」，勾选后写入设置，之后不再询问；
  - 偏好为「最小化到托盘」：直接隐藏主窗口，不关闭窗口实体、不销毁标签页、不终止进程；
  - 偏好为「退出应用」：走现有退出确认与标签页关闭检查。
- 托盘初始化失败时不弹窗：任何偏好都不能制造无法恢复的隐藏窗口，一律回退到现有退出确认。
- 隐藏到托盘这一步失败时同样回退退出确认（宁可直接退出，也不留下找不到的隐藏窗口）。
- 托盘可用性的判断先于偏好：偏好只在托盘真的可用时才起作用。
- 点击系统最小化按钮时，继续使用 GPUI 当前的系统最小化行为。
- Windows 当前绑定为 `QuitApp` 的 `Alt+F4`、macOS 的 `Cmd+Q`、应用菜单退出操作继续表示显式退出，不改为隐藏。

### 托盘交互

托盘提供以下操作：

- 单击托盘图标：显示并激活已有主窗口；
- “显示 Navop”：与单击托盘图标行为一致；
- “退出 Navop”：先显示主窗口，再调用现有退出请求入口，展示退出确认并执行标签页关闭检查。

托盘恢复不得创建第二个主窗口。重复的显示请求应为幂等操作。

### macOS reopen

macOS 在应用已运行时从 Dock 再次打开应用，会触发 GPUI `Application::on_reopen`。该事件应显示并激活已有主窗口，与托盘恢复使用同一入口。

## 当前实现约束

当前主窗口在 `main/src/main.rs` 中创建，应用使用 `QuitMode::LastWindowClosed`。`OnetCliApp::new` 通过 `window.on_window_should_close` 拦截关闭，随后调用 `request_quit` 显示退出确认；确认后 `close_all_tabs` 成功才调用 `cx.quit()`。

`main/src/app_init.rs` 已保存主窗口的 `AnyWindowHandle`，并通过系统级快捷键在最小化和激活之间切换。托盘恢复应复用“操作已有主窗口”的思路，但不能把系统最小化等同于真正隐藏。

GPUI 当前版本的平台能力不一致：

- macOS 的 `App::hide()` 有实际平台实现；
- Windows 的 `App::hide()` 是空实现；
- Linux 的 `App::hide()` 仅记录日志，不隐藏应用；
- `Window::minimize_window()` 和 `Window::activate_window()` 在三个桌面平台可用，但最小化仍可能在 Dock 或任务栏保留窗口入口。

因此本功能需要独立的托盘后端和窗口可见性平台适配，不能只调用 `cx.hide()`。

## 架构

### `system_tray` 模块

新增 `main/src/system_tray.rs`，负责：

- 初始化平台托盘后端；
- 保存托盘专用的主窗口 handle，不改变 `app_init` 现有系统快捷键状态；
- 持有托盘资源，保证图标在应用生命周期内不被提前释放；
- 将托盘点击和菜单操作转换为统一的 `TrayCommand`；
- 把来自平台线程或回调的命令安全转发到 GPUI 主线程；
- 记录托盘是否可用，供窗口关闭策略查询；
- 在初始化失败时记录明确日志并保持应用可退出。

统一命令保持最小集合：

```rust
enum TrayCommand {
    ShowMainWindow,
    QuitApplication,
}
```

托盘后端回调不得直接操作 GPUI entity 或 `Window`。回调只向线程安全 channel 发送 `TrayCommand`；GPUI foreground task 定期排空 channel，并在应用上下文中执行命令。

### 托盘平台后端

macOS、Windows、Linux 统一使用 `tray-icon` 的 target-specific 依赖，但 Linux 走它的
**`ksni` 后端**：

- macOS / Windows 用原生后端（`NSStatusItem` / `Shell_NotifyIcon`），
  依赖声明为 `default-features = false`；
- Linux 依赖声明为 `default-features = false, features = ["ksni"]`：`tray-icon`
  的默认特性会启用 `libappindicator`（连带 GTK 3 事件循环与 `libappindicator`）
  和 `libxdo`，两者都不需要；
- 关掉默认特性后，`tray-icon` 在 Linux 上选择的正是 ksni 后端
  （`platform_impl/mod.rs` 里 `feature = "ksni"` 才 `mod platform`），
  运行时仍是 freedesktop/KDE StatusNotifierItem D-Bus 协议，
  但不需要为 GPUI 引入 GTK，也不需要维护一套 Linux-only 的 `Tray` 实现；
- 三个平台共用同一套 `TrayIconBuilder` / `TrayIconEvent` / `menu::MenuEvent`
  API，事件语义差异由平台文档给定（KSNI 只发左/中键激活事件，右键归宿主，
  `rect` 为空；AppIndicator 后端完全不发事件——本实现不启用它）。

不做的事：不改变 `tray-icon` 的图标语义，不启用 `serde`，不使用
`TrayIconEvent::receiver()`。

### `window_visibility` 模块

新增 `main/src/window_visibility.rs`，负责操作已有主窗口：

```rust
pub(crate) fn hide_main_window(window: &Window) -> anyhow::Result<()>;
pub(crate) fn main_window_target(window: &Window) -> anyhow::Result<NativeMainWindow>;
pub(crate) fn show_main_window(target: NativeMainWindow) -> anyhow::Result<()>;
```

不需要 `cx: &mut App`：恢复窗口用 `Window::activate_window(&self)`，macOS 的整应用
激活由模块内部用 `NSApplication::activate` 完成。这样调用点（`AsyncApp` 的
`update_window` 闭包）不必额外借入应用上下文。

**恢复路径刻意拆成两步**（`main_window_target` 取句柄 → 借用释放后 `show_main_window`
改原生状态）。原生可见性/激活修改会**同步**回调进 GPUI：
`gpui/src/window.rs` 的 `on_active_status_change` / `on_visibility_change` 直接调
`handle.update(...)`。若在 `cx.update_window(...)` 的借用内改原生状态，回调会回头抢同一个
App 借用 ⇒ 二次借用失败，日志只剩 `ERROR gpui::window: RefCell already borrowed`
（2026-09-17 真机实测：仅在「窗口已隐藏再恢复」这一步出现）。隐藏路径不受影响——
`orderOut:` 的可见性回调由 AppKit 异步投递（实测隐藏无报错），所以仍可在借用内直接调用。

各平台行为：

- macOS：`NSWindow::orderOut:` 隐藏；恢复时 `makeKeyAndOrderFront:` 并
  `NSApplication::activate`。获取 `NSWindow` 的路径是
  `HasWindowHandle::window_handle(window)` → `RawWindowHandle::AppKit.ns_view`
  → `NSView::window()`；注意 GPUI 的 `Window::window_handle()` 返回的是
  `AnyWindowHandle`，会遮蔽同名 trait 方法，必须显式限定 trait；
- Windows：使用 `ShowWindow(SW_HIDE)` 隐藏；恢复时 `SW_RESTORE`、
  `SetForegroundWindow` 和 GPUI 激活入口；
- Linux X11：从 `RawWindowHandle::Xcb` / `Xlib` 取 window id，用
  `x11rb` 的 `unmap_window` / `map_window` 隐藏与恢复并 `flush`。
  `raw-window-handle` 0.6 的句柄不带 X 连接，所以每次调用新建一条连接
  （unmap/map 只按全局唯一的 window id 寻址）；
- Linux Wayland：Wayland 没有允许客户端任意隐藏并重新映射现有 xdg-toplevel
  的通用协议，使用 `minimize_window` 和 `activate_window` 作为明确的平台回退。
  `NativeMainWindow` 在此情况下取 0（无 X11 句柄），`show` 变成空操作。

隐藏或恢复**必须校验结果**：macOS 用 `NSWindow::isVisible`、Windows 用
`IsWindowVisible` 复核目标状态，不一致即返回错误。关闭事件中的隐藏失败必须回退
现有退出确认，不能吞掉关闭请求。

### 主窗口生命周期

`main/src/main.rs` 将 quit mode 改为 `QuitMode::Explicit`。真正退出仍只能通过现有 `cx.quit()` 路径发生，避免未来平台窗口被关闭时自动终止仍持有托盘的应用。

主窗口创建完成后依次：

1. 保存主窗口 handle；
2. 初始化系统快捷键；
3. 初始化托盘；
4. 创建 `OnetCliApp` 和根视图。

托盘初始化结果应在 `OnetCliApp` 安装关闭 handler 前可查询。

### 关闭策略

关闭 handler 使用一个可单元测试的纯策略函数决定行为：

```rust
enum MainWindowCloseAction {
    AskUser,
    HideToTray,
    RequestQuit,
}

fn main_window_close_action(
    tray_ready: bool,
    behavior: CloseButtonBehavior,
) -> MainWindowCloseAction;
```

- 托盘不可用时一律返回 `RequestQuit`，无视偏好；
- 托盘可用时按 `AppSettings.close_button_behavior` 返回
  `Ask` → `AskUser`、`MinimizeToTray` → `HideToTray`、`Quit` → `RequestQuit`。

执行 `HideToTray` 时调用窗口可见性适配器并返回 `false`，阻止 GPUI 销毁窗口。若隐藏失败，立即调用现有 `request_quit`，仍返回 `false`，由现有退出流程决定是否退出。

`AskUser` 时同样返回 `false`，由弹窗决定后续：

- 弹窗用 `WindowExt::open_dialog` 打开，两条出路用自定义 `DialogFooter` 各自渲染一个
  按钮（`tray-close-minimize` / `tray-close-quit`），点击后先 `window.close_dialog(cx)`
  再走对应流程。**不要用默认的 ok/cancel footer**：默认 footer 只有两个固定按钮，
  把「取消」当成第二条出路会让 Esc 和点遮罩也走成退出应用；而 `button_props` 若在
  `confirm` 之后设置，还会把 `show_cancel` 重置、直接吞掉第二个按钮；
- 「记住我的选择」的勾选状态放在独立 entity（`CloseChoiceRememberState`）里：
  弹窗 body 由 `Root` 渲染，`NavopApp` 的 `notify()` 不会重绘它，勾选框必须自己刷新；
- `close_choice_prompt_open` 标记防止连点关闭按钮叠出第二个弹窗，弹窗被关掉时复位；
- 用户选择「最小化到托盘」沿用 `HideToTray` 路径，选择「退出应用」沿用现有
  `request_quit`，两条去向都复用既有流程，弹窗只负责选一次；
- 勾选「记住」时把结果写入 `AppSettings.close_button_behavior` 并落盘。

偏好同时在设置页「通用 → 关闭窗口行为」提供下拉项（默认值即 `Ask`），
文案键为 `Settings.General.CloseBehavior.*`，托盘弹窗文案键为 `Tray.*`，
三套语言（en / zh-CN / zh-HK）齐备。

### 显式退出复用

托盘“退出 Navop”不得直接调用 `cx.quit()`。统一流程为：

1. 获取已注册的主窗口 handle；
2. 恢复并激活主窗口；
3. 在该窗口上下文中调用现有 `request_window_quit`；
4. 用户确认后执行 `close_all_tabs`；
5. 只有 `close_all_tabs` 返回成功才调用 `cx.quit()`。

如果主窗口 handle 已失效，说明“窗口始终被关闭 handler 保留”的生命周期 invariant 已经被破坏。该异常路径记录 error 后允许直接调用 `cx.quit()`，避免用户选择退出后进程永久残留；正常托盘退出路径不得依赖这一回退，也不得绕过退出确认。

## 图标资源

托盘图标使用仓库现有 `resources/navop-icon.png`，通过 `include_bytes!` 编译进二进制，避免依赖运行目录或安装包中的相对路径。

该资源是 1024×1024 的应用图标，直接交给平台会被按菜单栏/通知区域高度做一次质量
不可控的缩放，所以先解码为 RGBA、按最长边缩到 32px 再构造 `tray_icon::Icon`
（`Icon::from_rgba` 内部完成各平台需要的通道/位深转换，无需手写 ARGB32 变换）。
解码与缩放放在独立纯函数 `decode_tray_icon` 中，便于单元测试。

解码失败即托盘初始化失败——宁可不显示托盘，也不做一个没有图标、用户找不到的
隐形入口。

本次不修改品牌图标，不生成新的视觉资产。

## 并发与资源生命周期

- 托盘平台回调可能不在 GPUI foreground executor 上运行，禁止直接持有或更新 GPUI context；
- channel sender 可跨线程克隆，receiver 只由一个 GPUI task 消费；
- 托盘初始化只能执行一次；重复初始化返回已有状态；
- macOS/Windows 的 `TrayIcon` 保持在线程本地存储中，避免 `Rc` 类型跨线程；
- Linux 的 `ksni` handle 保持到进程退出，避免托盘 service 提前 shutdown；
- GPUI task 退出或 channel 断开时停止轮询并记录日志，禁止无界错误循环。

## 错误处理

- 托盘创建失败：记录 warning，关闭按钮继续走现有退出确认；
- 图标解码失败：托盘初始化失败，不创建无图标的不可发现入口；
- 托盘事件发送失败：记录 warning，不 panic；
- 窗口隐藏失败：回退现有退出确认；
- 窗口恢复失败：记录 error，保留托盘和进程，允许用户再次尝试或选择退出；
- Linux StatusNotifierItem watcher 不可用：视为托盘不可用，不改变关闭行为。

## 测试策略

本功能属于跨平台窗口生命周期行为变更，使用 TDD。

### 纯逻辑测试

- 托盘可用且偏好为「每次询问」时关闭策略返回 `AskUser`；偏好为「最小化到托盘」返回
  `HideToTray`，偏好为「退出应用」返回 `RequestQuit`；
- 托盘不可用时关闭策略返回 `RequestQuit`；任何偏好都不能隐藏窗口；
- 关闭弹窗同时提供「最小化到托盘」与「退出应用」两条去向，并带「记住我的选择」勾选框；
- 勾选记住后偏好写入 `AppSettings.close_button_behavior` 并落盘；
- 用户偏好三值（`ask` / `minimize_to_tray` / `quit`）在设置 JSON 上 round-trip，
  旧 JSON 与未知值回退 `ask`；
- 托盘图标点击（左键抬起）映射为 `ShowMainWindow`；左键按下、右键、中键都不映射；
- “显示 Navop”映射为 `ShowMainWindow`；
- “退出 Navop”映射为 `QuitApplication`；
- 未知菜单 id 不产生命令；
- 内嵌 PNG 图标可解码为 32×32 RGBA（像素总数 = 宽×高×4，且不是全透明）；
- 非图像载荷解码失败。

### 结构与集成测试

- 主应用使用 `QuitMode::Explicit`；
- 托盘初始化发生在窗口系统初始化之后、`OnetCliApp` 安装关闭 handler 之前；
- 主窗口关闭 handler 不再无条件调用 `request_quit`，而是先过托盘策略，并区分
  `AskUser` / `HideToTray` / `RequestQuit` 三条分支；
- 关闭弹窗用自定义 footer 渲染两条去向按钮与记住勾选框，不回退到默认 ok/cancel footer；
  连点关闭按钮不会叠弹窗；Esc 与点遮罩只关询问、不触发退出；
- 设置页通用页存在「关闭窗口行为」下拉项，三套语言文案齐备；
- 托盘退出路径调用现有 `request_window_quit`，不直接调用 `cx.quit()`；
- 平台回调只入队命令，不触碰 `Window` / `update_window`；
- `TrayIcon` 保存在 `thread_local!` 中而不是任何 `Send` 容器里；
- Linux 依赖只启用 tray-icon 的 `ksni` 后端，`libappindicator` / `libxdo` 不出现在依赖里。

### 验证命令

- 运行 `main` 的托盘和窗口生命周期定向测试；
- 运行 `cargo test -p main`；
- 运行 `cargo check -p main --all-targets`；
- 运行 `cargo clippy -p main --all-targets -- -D warnings`；
- 运行 `cargo fmt --all -- --check`；
- 使用可用 target 执行平台编译检查；无法在本机运行的平台必须明确报告验证边界。

### macOS 手工冒烟

当前开发环境为 macOS，完成自动验证后执行真实应用冒烟：

1. 启动 Navop 并打开多个标签页；
2. 点击关闭按钮，确认主窗口消失且托盘图标仍存在；
3. 点击托盘图标，确认原窗口和标签页状态恢复；
4. 再次隐藏，通过“显示 Navop”恢复；
5. 选择“退出 Navop”，确认出现现有退出确认；
6. 取消退出，确认应用仍可隐藏和恢复；
7. 再次选择退出并确认，确认应用进程终止且托盘图标消失；
8. 从 Dock reopen 隐藏中的应用，确认主窗口恢复。

## 附：Windows 单实例重复启动修复

症状：Windows 上重复双击启动 Navop 会开出第二个完整进程，第二个进程不再把启动请求
转发给已有实例。

根因（`main/src/windows_single_instance.rs` + `interprocess` 2.4.4）：

- `interprocess` 的 Windows 命名管道监听器带 `FILE_FLAG_FIRST_PIPE_INSTANCE`，
  名字已存在时 `CreateNamedPipeW` 返回 `ERROR_ACCESS_DENIED(5)`；
- 该错误被原样透传，没有做 error kind 归一化，所以 `kind()` 永远不是 `AddrInUse`；
- 于是「名字已被占用」的分支在 Windows 上永不命中，第二个实例误判自己为主实例并继续
  完整启动。

修复：把判定抽成 `instance_name_taken(&io::Error)`，接受 `AddrInUse` 或
`raw_os_error()` ∈ {`ERROR_ACCESS_DENIED(5)`, `ERROR_PIPE_BUSY(231)`}；其余错误
（如 `PermissionDenied`）不能被误判为「已占用」，否则主实例会把启动请求转发给不存在
的管道。不更换机制，仍是命名管道 + 转发启动路径；转发失败仍按现状记录日志后继续启动。

## 风险与取舍

- Linux 桌面环境不一定提供 StatusNotifierItem watcher。此时功能明确降级为原有关闭确认，而不是隐藏到不可恢复状态。
- Wayland 不提供通用的客户端隐藏/重新映射顶层窗口协议，只能使用最小化回退；X11、macOS 和 Windows 提供真正隐藏。
- 平台 native API 和 raw handle 操作存在差异，必须封装在小型模块内，避免扩散到 `OnetCliApp`。
- 两套托盘后端增加少量条件编译复杂度，但避免 Linux GTK 事件循环和系统依赖风险。
- 保留原窗口而不是关闭后重建，可以保持标签页、连接、焦点状态和后台任务，不需要实现工作区序列化恢复。

## 验收标准

- macOS、Windows 和支持 StatusNotifierItem 的 Linux 桌面显示 Navop 托盘图标；
- 点击主窗口关闭按钮时，托盘可用的平台保留进程和主窗口状态；首次关闭按偏好询问并记住选择；
- 系统最小化按钮保持原行为；
- 托盘单击和“显示 Navop”恢复同一个主窗口，不创建重复窗口；
- 托盘“退出 Navop”进入现有退出确认和标签页关闭检查；
- `Cmd+Q`、`Alt+F4` 和应用菜单退出继续表示显式退出；
- 托盘初始化或窗口隐藏失败时回退现有退出确认；
- Windows 重复启动只保留一个实例，第二个进程把启动请求转发给已有实例；
- macOS Dock reopen 可恢复隐藏窗口；
- Linux X11 使用真正隐藏，Wayland 使用有记录的最小化回退；
- 托盘图标使用内嵌的现有 Navop 品牌资源；
- 新增逻辑有红绿 TDD 证据，相关测试、check、Clippy 和格式检查通过；
- 不覆盖或提交工作区中与本任务无关的用户改动。
