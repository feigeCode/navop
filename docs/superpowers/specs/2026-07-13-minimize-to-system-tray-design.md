# 最小化到系统托盘设计

## 目标

为 Navop 桌面应用增加跨平台系统托盘能力。用户关闭主窗口时，应用隐藏到系统托盘并继续保留当前标签页、连接和后台状态；用户可从托盘恢复原主窗口，或通过托盘菜单进入现有的安全退出流程。

本功能覆盖仓库当前发布的 macOS、Windows 和 Linux 平台。实现必须保留现有退出确认、标签页关闭检查和显式退出快捷键语义。

## 用户交互

### 关闭与最小化

- 点击主窗口关闭按钮时：
  - 托盘初始化成功，则隐藏主窗口，不关闭窗口实体、不销毁标签页、不终止进程；
  - 托盘初始化失败，则回退到现有退出确认，禁止产生无法恢复的隐藏窗口。
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

macOS 和 Windows 使用 target-specific `tray-icon` 依赖：

- 在 GPUI 平台事件循环已经启动后创建托盘图标；
- 使用 `tray_icon::menu` 创建“显示 Navop”和“退出 Navop”；
- 关闭 `tray-icon` 默认的左键菜单行为，使左键单击只发送 `ShowMainWindow`，右键继续打开菜单；
- 使用 `TrayIconEvent` 和 `MenuEvent` 转发命令；
- 托盘对象保存在创建它的平台线程上。

Linux 使用 target-specific `ksni` blocking API：

- 通过 freedesktop/KDE StatusNotifierItem D-Bus 协议提供托盘；
- 避免为 GPUI 引入 GTK main loop、`libappindicator` 和 `libxdo` 系统依赖；
- 左键激活和菜单 item 回调只发送统一命令；
- 桌面环境没有 StatusNotifierItem watcher 时，初始化视为失败，主窗口关闭行为回退到退出确认。

不在本次实现中增加托盘开关、启动时隐藏、通知气泡、动态菜单或后台启动选项。

### `window_visibility` 模块

新增 `main/src/window_visibility.rs`，负责操作已有主窗口：

```rust
pub(crate) fn hide_main_window(window: &Window) -> anyhow::Result<()>;
pub(crate) fn show_main_window(window: &Window, cx: &mut App) -> anyhow::Result<()>;
```

各平台行为：

- macOS：通过 AppKit `NSWindow` 执行 `orderOut:` 隐藏；恢复时执行 `makeKeyAndOrderFront:` 并激活应用；
- Windows：使用 `ShowWindow(SW_HIDE)` 隐藏；恢复时使用 `SW_RESTORE`、`SetForegroundWindow` 和 GPUI 激活入口；
- Linux X11：从 raw XCB handle 获取连接和 window id，使用 `xcb_unmap_window` 隐藏、`xcb_map_window` 恢复并 flush；恢复后调用 GPUI `activate_window`；
- Linux Wayland：Wayland 没有允许客户端任意隐藏并重新映射现有 xdg-toplevel 的通用协议，使用 `minimize_window` 和 `activate_window` 作为明确的平台回退。

隐藏或恢复失败时返回错误并记录平台和操作上下文。关闭事件中的隐藏失败必须回退现有退出确认，不能吞掉关闭请求。

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
    HideToTray,
    RequestQuit,
}
```

- 托盘可用时返回 `HideToTray`；
- 托盘不可用时返回 `RequestQuit`。

执行 `HideToTray` 时调用窗口可见性适配器并返回 `false`，阻止 GPUI 销毁窗口。若隐藏失败，立即调用现有 `request_quit`，仍返回 `false`，由现有退出流程决定是否退出。

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

- macOS/Windows 将 PNG 解码为 RGBA，并构造 `tray_icon::Icon`；
- Linux 将同一 RGBA 数据转换为 StatusNotifierItem 要求的 ARGB32 pixmap；
- 解码和颜色通道转换放在独立纯函数中，便于单元测试。

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

- 托盘可用时关闭策略返回 `HideToTray`；
- 托盘不可用时关闭策略返回 `RequestQuit`；
- 托盘图标点击映射为 `ShowMainWindow`；
- “显示 Navop”映射为 `ShowMainWindow`；
- “退出 Navop”映射为 `QuitApplication`；
- PNG 图标可解码为正确尺寸的 RGBA；
- Linux RGBA 到 ARGB32 转换保持 alpha、red、green、blue 通道顺序。

### 结构与集成测试

- 主应用使用 `QuitMode::Explicit`；
- 主窗口关闭 handler 不再无条件调用 `request_quit`；
- 托盘退出路径调用现有 `request_window_quit`，不直接调用 `cx.quit()`；
- target-specific dependency 不让 Linux 编译 `tray-icon`，也不让 macOS/Windows 编译 `ksni`。

### 验证命令

- 运行 `main` 的托盘和窗口生命周期定向测试；
- 运行 `cargo test -p main`；
- 运行 `cargo check -p main`；
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

## 风险与取舍

- Linux 桌面环境不一定提供 StatusNotifierItem watcher。此时功能明确降级为原有关闭确认，而不是隐藏到不可恢复状态。
- Wayland 不提供通用的客户端隐藏/重新映射顶层窗口协议，只能使用最小化回退；X11、macOS 和 Windows 提供真正隐藏。
- 平台 native API 和 raw handle 操作存在差异，必须封装在小型模块内，避免扩散到 `OnetCliApp`。
- 两套托盘后端增加少量条件编译复杂度，但避免 Linux GTK 事件循环和系统依赖风险。
- 保留原窗口而不是关闭后重建，可以保持标签页、连接、焦点状态和后台任务，不需要实现工作区序列化恢复。

## 验收标准

- macOS、Windows 和支持 StatusNotifierItem 的 Linux 桌面显示 Navop 托盘图标；
- 点击主窗口关闭按钮时，托盘可用的平台保留进程和主窗口状态；
- 系统最小化按钮保持原行为；
- 托盘单击和“显示 Navop”恢复同一个主窗口，不创建重复窗口；
- 托盘“退出 Navop”进入现有退出确认和标签页关闭检查；
- `Cmd+Q`、`Alt+F4` 和应用菜单退出继续表示显式退出；
- 托盘初始化或窗口隐藏失败时回退现有退出确认；
- macOS Dock reopen 可恢复隐藏窗口；
- Linux X11 使用真正隐藏，Wayland 使用有记录的最小化回退；
- 托盘图标使用内嵌的现有 Navop 品牌资源；
- 新增逻辑有红绿 TDD 证据，相关测试、check、Clippy 和格式检查通过；
- 不覆盖或提交工作区中与本任务无关的用户改动。
