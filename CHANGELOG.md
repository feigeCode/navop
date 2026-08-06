# Changelog

Navop user-facing release notes. Generate and review each bilingual version entry before creating the release tag.

<!-- NAVOP_RELEASES -->

## [v0.10.5] - 2026-08-06

### 中文

#### 更新内容

- SSH 终端新增可配置字符集，支持 UTF-8、GBK、GB18030、Big5、Shift_JIS、EUC-JP、EUC-KR 和 Windows-1252，改善旧系统及非 UTF-8 环境的显示与输入。
- 终端右键菜单新增“粘贴选中内容”，可直接将当前选中的文本发送到终端。
- SSH 主机指纹发生变化时新增安全确认，展示新旧指纹并提示中间人攻击风险，需明确确认后才能更新或临时接受。

#### 修复与优化

- 修复 Agent 上下文压缩模型调用失败时任务会中断的问题，现在会使用本地摘要继续执行，同时保留取消操作语义。

---

### English

#### What's New

- Added configurable SSH terminal encodings, including UTF-8, GBK, GB18030, Big5, Shift_JIS, EUC-JP, EUC-KR, and Windows-1252, improving display and input for legacy and non-UTF-8 environments.
- Added “Paste Selected Text” to the terminal context menu, allowing the current selection to be sent directly to the terminal.
- Added explicit security confirmation when an SSH host key changes, showing the new and previously trusted fingerprints and warning about possible man-in-the-middle attacks before allowing an update or one-time acceptance.

#### Fixes and Improvements

- Fixed Agent tasks stopping when context-compaction model calls fail; a local fallback summary is now used while preserving cancellation behavior.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.10.4...v0.10.5

## [v0.10.4] - 2026-08-05

### 中文

#### 更新内容

- 终端新增 SSH 下的 ZMODEM 文件传输支持，可在检测到上传或下载请求时选择本地文件或下载目录。
- SFTP 文件传输工具栏新增目录上传能力，可从上传菜单直接选择文件或目录。
- 数据库对象树中的表菜单新增“复制表名”和“复制表注释”操作。

#### 修复与优化

- 修复 SQL 查询结果导出不完整的问题，现在可导出完整结果集。
- 修复 Agent 输入框 mention 补全在快速输入、中文或数字查询时可能崩溃或显示过期结果的问题。
- 修复 Windows 本地终端环境变量未及时刷新以及 Git Bash 路径解析问题。
- 修复 RDP 显示及桌面交互相关问题，并改善连接侧栏中的连接分组拖放目标区域。
- 为 Linux Wayland 窗口设置稳定的应用 ID 和 `Navop` 窗口标题，改善桌面环境中的窗口识别。
- 修复 Markdown 编辑器删除包含 Unicode 字符的脚注引用时可能发生的崩溃。

---

### English

#### What's New

- Added ZMODEM file transfers over SSH, including file selection for uploads and destination-directory selection for downloads.
- Added directory uploads to the SFTP file-transfer toolbar, allowing users to choose files or folders directly from the upload menu.
- Added table actions for copying a table name or table comment from the database object tree.

#### Fixes and Improvements

- Fixed incomplete SQL query-result exports so complete result sets can now be exported.
- Fixed crashes and stale-result updates in Agent mention completion, especially during rapid typing and CJK or numeric queries.
- Fixed stale Windows local-terminal environments and improved Git Bash path resolution.
- Fixed display and desktop interaction issues in RDP sessions, and expanded connection-group drop targets in the sidebar.
- Set a stable Linux Wayland application ID and `Navop` window title for better desktop integration.
- Fixed a crash when deleting Markdown footnote references containing Unicode characters.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.10.3...v0.10.4

## [v0.10.3] - 2026-08-04

### 中文

#### 更新内容

- 支持编辑 MySQL 存储过程、MySQL 函数以及 PostgreSQL 函数和过程；例程列表显示参数和身份参数信息，按 schema 区分对象，并支持准确打开重载例程。
- 新增 RDP 保存前连接测试，提供超时和更清晰的失败诊断；修复远程键盘输入状态处理，并优化 RDP/VNC 连接图标显示。
- 重设计开始中心并统一桌面 UI 视觉系统，改善连接侧栏和数据库对象导航布局，以及连接协议、数据库导航和 AI 图标的一致性与可读性。
- 新增全局同步开关（默认关闭），并完善同步与加密提示；便携模式可在设置中选择将加密主密钥副本保存到 `data/state/key_storage` 以自动解锁。该副本使用程序内置密钥而非设备绑定保护，任何同时获得应用程序和完整 `data` 目录的人都可能恢复主密钥；仅在理解并接受此风险时启用。
- 新增 Windows 32 位发布包，并让更新器按 Windows x86 选择对应下载包。

#### 修复与优化

- 连接快速打开现在支持按 IP 地址、用户名、主机和端口搜索。
- 约束 Agent/MCP 不把计划标题、状态等内容直接提交为 shell 命令；远程无 stdin 命令启动后立即发送 EOF，避免因等待输入而无限挂起。

---

### English

#### What's New

- Added editors for MySQL procedures, MySQL functions, and PostgreSQL functions and procedures. Routine lists now show argument and identity-argument information, distinguish schema-scoped objects, and open overloaded routines accurately.
- Added a pre-save RDP connection test with timeout handling and clearer failure diagnostics, fixed remote keyboard input-state handling, and improved RDP/VNC connection icons.
- Redesigned the Start Center and established a unified desktop visual system, improving the connection sidebar and database-object navigation layouts, together with the consistency and readability of connection-protocol, database-navigation, and AI icons.
- Added a global sync switch that is disabled by default and clarified sync and encryption prompts. Portable mode can optionally store an encrypted master-key copy under `data/state/key_storage` for automatic unlock. This copy uses a key embedded in the application rather than device-bound protection, so anyone who obtains both the application and the complete `data` directory may be able to recover the master key. Enable it only if you understand and accept this risk.
- Added Windows 32-bit release packages and made the updater select the matching Windows x86 download.

#### Fixes and Improvements

- Connection Quick Open now searches by IP address, username, host, and port.
- Prevented Agent/MCP from submitting plan titles or status text directly as shell commands, and now send EOF immediately to remote commands without stdin so they do not wait indefinitely for input.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.10.2...v0.10.3

## [v0.10.2] - 2026-08-03

### 中文

#### 更新内容

- Windows 新增 EXE 安装包，与 MSI 共用同一套当前用户安装流程；使用默认安装位置时无需管理员权限，并提供开始菜单、桌面快捷方式和文件关联。
- Windows 普通免安装 ZIP 与便携 ZIP 现已明确拆分：普通 `navop-x86_64-pc-windows-msvc.zip` 使用标准 Windows 用户数据目录并支持记住主密钥；`navop-x86_64-pc-windows-msvc-portable.zip` 将数据保存在程序旁，默认每次启动时要求输入主密钥，也允许用户在明确接受风险后选择将可自动恢复的加密副本保存到 `data/state/key_storage`。该副本使用程序内置密钥而非设备绑定保护，同时获得应用程序和完整 `data` 目录的人可能恢复主密钥。
- SSH 连接新增“允许旧版 SSH 算法”兼容选项，默认关闭；需要连接旧服务器时可按连接启用，并覆盖 SSH、SFTP、跳板机和连接复用场景。

#### Windows ZIP 用户升级提示

- **如果你使用的是 v0.10.1 或更早版本的 Windows ZIP，请继续下载新的 `navop-x86_64-pc-windows-msvc-portable.zip`。**旧版普通 ZIP 实际包含 `navop.portable`，因此原有数据位于程序旁的 `data` 目录。
- 升级前请完整备份旧便携目录；将新版便携 ZIP 解压到新目录后，把旧目录中的整个 `data` 复制过去，并确认 `navop.portable` 仍与 `navop.exe` 同级。启动后需要输入原主密钥。
- 不要通过删除 `navop.portable` 来迁移数据。新的普通 ZIP、MSI 和 EXE 安装版使用标准 Windows 用户数据目录，不会自动迁移旧便携数据；切换后连接和设置看似消失时，旧数据仍保留在原便携目录中。

#### 修复与优化

- 改进 SiliconFlow 等模型的图片附件兼容性：根据实际图片格式处理 PNG、JPEG、WebP 和 GIF，并对不兼容或过大的图片进行转换或缩放；无法处理的附件会在发送请求前给出明确错误。
- 改进旧版 SSH 服务器的连接失败提示：当密钥交换协商失败且没有共同 KEX 算法时，引导用户在连接的高级设置中启用旧版算法兼容选项。
- 优化 SSH 主机密钥算法选择，在不弱化主机密钥校验的前提下优先使用已信任密钥对应的算法，旧版算法仅在连接明确启用兼容选项后加入。
- 修复 SSH 连接设置窗口内容过长时的滚动和底部按钮布局，避免表单撑开窗口或遮挡操作按钮。

---

### English

#### What's New

- Added a Windows EXE installer that uses the same per-user installation flow as the MSI. The default installation location does not require administrator privileges and provides Start menu shortcuts, a desktop shortcut, and file associations.
- Clearly separated the standard Windows no-install ZIP from the portable ZIP. The standard `navop-x86_64-pc-windows-msvc.zip` uses the normal Windows user data directories and supports remembered master-key unlock. The portable `navop-x86_64-pc-windows-msvc-portable.zip` keeps data beside the executable and asks for the master key on every start by default, but users who explicitly accept the risk may store an encrypted, automatically recoverable copy under `data/state/key_storage`. This copy uses a key embedded in the application instead of device-bound protection, so anyone who obtains both the application and the complete `data` directory may be able to recover the master key.
- Added an opt-in “Allow Legacy SSH Algorithms” compatibility setting for individual SSH connections. It is disabled by default and applies to SSH, SFTP, jump hosts, and connection reuse when explicitly enabled for legacy servers.

#### Upgrade Notice for Windows ZIP Users

- **If you use the Windows ZIP from v0.10.1 or earlier, continue with the new `navop-x86_64-pc-windows-msvc-portable.zip`.** The earlier standard ZIP contained `navop.portable`, so its existing data is stored in the `data` directory beside the executable.
- Back up the complete old portable directory before upgrading. Extract the new portable ZIP to a new directory, copy the entire old `data` directory into it, keep `navop.portable` beside `navop.exe`, and enter the original master key when starting the new version.
- Do not migrate by deleting `navop.portable`. The new standard ZIP and the MSI/EXE installers use the normal Windows user data directories and do not automatically migrate old portable data. If connections and settings appear missing after switching editions, the original data remains in the old portable directory.

#### Fixes and Improvements

- Improved image attachment compatibility for SiliconFlow and other models by handling PNG, JPEG, WebP, and GIF according to their actual encoding, converting or resizing incompatible and oversized images, and reporting unsupported attachments before sending the request.
- Added an actionable hint when an older SSH server fails key-exchange negotiation because there is no common KEX algorithm, directing users to enable legacy algorithm compatibility in the connection's advanced settings.
- Improved SSH host-key algorithm selection by prioritizing algorithms associated with trusted keys without weakening host-key verification. Legacy algorithms are added only when explicitly enabled for the connection.
- Fixed scrolling and footer-button layout in the SSH connection form when the content is taller than the window.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.10.1...v0.10.2

## [v0.10.1] - 2026-08-02

### 中文

#### 更新内容

- 新增终端会话录制与只读时间线回放，支持持久化保存、录制文件关联和安全浏览；回放期间会阻止输入、在线操作及 Public MCP 暴露，避免误执行。
- 数据库表数据视图新增打开表查询入口，单元格预览面板支持调整大小，并改进行选择与 Shift 连续范围选择。
- 改进 Markdown 编辑和文件交互：增强 Typora 兼容编辑体验、支持在表格单元格中渲染图片、补充笔记导航快捷键，并可通过拖放打开已关联文件。
- 更新对话框提供更完整的版本信息与跳过版本选项，同时增加本地更新模拟能力，便于验证完整更新流程。

#### 修复与优化

- 修复 Windows 上 WSL、PowerShell 等终端在持续输出或高负载下白屏、窗口无响应的问题；同时为渲染、搜索、选择、剪贴板、滚动和控制操作增加非阻塞调度与有界排队，持续输出时界面仍可响应。
- 全面优化终端高负载路径：限制输入队列和命令执行输出捕获，改进 SSH/串口解析入口、性能指标、命令栏历史导航，并在 SSH 重连后保留现有终端输出。
- 改进 SSH 主机密钥校验、信任提示、会话复用、闲置回收和重连行为，使多窗口及连接恢复更加稳定。
- 提升 SFTP 传输可靠性：断开和重连时及时淘汰过期客户端与传输池，远程写入采用暂存后替换，并正确反馈远程读取失败，降低卡住和文件半写风险。
- 保留并展示 IPC 数据库驱动和 PostgreSQL 返回的详细错误信息，帮助定位 SQL、连接和服务端问题；同时提前阻止与当前 Navop 宿主不兼容的 IPC 驱动。
- 修复 CSV 导入导出以及 ClickHouse、DuckDB IPC 驱动中 `NULL` 与空字符串语义混淆的问题，并改进数据库表格编辑、搜索快捷键和大文本预览体验。
- 修复 RDP Caps Lock 状态不同步的问题，并使服务器监控中的进程颜色更好地适配当前终端主题。
- 改进 Public MCP 工具目标恢复和超大终端命令输出的截断反馈，降低异常会话或大输出对应用稳定性的影响。

---

### English

#### What's New

- Added persistent terminal session recording with read-only timeline playback, recording file associations, and safe browsing. Playback blocks input, online operations, and Public MCP exposure to prevent accidental execution.
- Added an entry point for opening table queries from database table views, made the cell preview panel resizable, and improved row selection with Shift-based range extension.
- Improved Markdown editing and file interactions with better Typora compatibility, image rendering inside table cells, note-navigation shortcuts, and drag-and-drop opening for associated files.
- Expanded the update dialog with richer version information and a skip-version option, and added local update simulation for validating the complete update flow.

#### Fixes and Improvements

- Fixed Windows terminals such as WSL and PowerShell blanking or becoming unresponsive during continuous output or heavy load. Rendering, search, selection, clipboard, scrolling, and control operations now use non-blocking scheduling with bounded queues so the UI remains responsive.
- Optimized high-load terminal paths by bounding ingress queues and command-output capture, improving SSH and serial parser ingestion and performance metrics, adding command-bar history navigation, and preserving terminal output across SSH reconnects.
- Improved SSH host-key verification, trust prompts, session reuse, idle cleanup, and reconnect behavior for more reliable multi-window and connection recovery workflows.
- Improved SFTP reliability by retiring stale clients and transfer pools during disconnects and reconnects, staging remote writes before replacement, and surfacing remote read failures to reduce hangs and partial-write risks.
- Preserved and surfaced detailed IPC database-driver and PostgreSQL server errors for easier SQL and connection diagnostics, and now reject IPC drivers that are incompatible with the current Navop host before startup.
- Fixed `NULL` versus empty-string semantics across CSV import/export and the ClickHouse and DuckDB IPC drivers, and improved database table editing, search shortcuts, and large-text previews.
- Fixed RDP Caps Lock synchronization and adjusted server-monitor process colors to better match the active terminal theme.
- Improved Public MCP tool-target recovery and truncation feedback for very large terminal command output, reducing the stability impact of stale sessions and oversized results.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.10.0...v0.10.1

## [v0.10.0] - 2026-07-30

### 中文

#### 更新内容

- 新增 SSH 远程/反向端口转发（`ssh -R`），支持固定远程端口和端口 `0` 自动分配，并贯通连接管理、启动与停止、命令复制、分享、个人同步、状态展示及中英文文档。
- AI Chat 统一执行模式现在会持久化保存，重新打开应用后仍会保留上次选择。

#### 修复与优化

- 完善远程端口转发的生命周期处理，避免自动分配端口时的启动竞态，并确保停止失败后不会残留错误的运行状态。
- 优化 RDP 远程光标移动，使鼠标反馈更加平滑稳定。
- 修复数据库表重命名失败时错误未正确显示的问题。
- 修复 PostgreSQL 主键修改未正确应用的问题。
- 扩大 Tab 重命名输入区域，长名称编辑时可以看到更多内容。

---

### English

#### What's New

- Added SSH remote/reverse port forwarding (`ssh -R`) with both fixed remote ports and automatic port allocation via port `0`, integrated across connection management, start/stop handling, command copying, sharing, personal sync, status display, and bilingual documentation.
- Unified execution mode in AI Chat is now persisted, preserving the selected mode after restarting the application.

#### Fixes and Improvements

- Improved remote port-forwarding lifecycle handling by preventing the startup race during automatic port allocation and ensuring failed cleanup does not leave a stale running state.
- Smoothed remote cursor movement in RDP sessions for more stable pointer feedback.
- Fixed database table rename failures not being surfaced correctly.
- Fixed PostgreSQL primary-key edits not being applied correctly.
- Widened the Tab rename input so longer names remain visible while editing.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.9.8...v0.10.0
