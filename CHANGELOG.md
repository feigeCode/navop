# Changelog

Navop user-facing release notes. Generate and review each bilingual version entry before creating the release tag.

<!-- NAVOP_RELEASES -->

<!-- Unreleased 草稿（feat/tdengine-mqtt-middleware）：发布时将本节内容并入目标版本条目
     （按仓库惯例本批均为功能新增，预计为下一个 minor 版本），补齐 CNB 镜像下载行与
     Full Changelog 对比链接后删除本节与本注释。changelog.py 仅识别带日期的版本标题，
     本节不影响既有条目的 extract/validate/upsert。 -->

## [Unreleased]

#### 更新内容

- 新增 TDengine 时序数据库连接：基于官方 taos WebSocket 驱动（经 taosAdapter :6041 连接，纯 Rust、无需本地 C 依赖），支持数据库/超级表/子表浏览、DESCRIBE 与 TAG 识别、分页查询等完整数据库能力；连接表单、连接导入协议与 TDengine CLI 命令（`taos` / `jdbc:TAOS-RS://`）同步支持。
- 新增 MQTT 中间件连接：基于 rumqttc 运行时（MQTT 3.1.1，rustls 加密），提供订阅、消息、发布三个视图（消息环形缓冲、文本/Hex 切换），连接页签对齐 MongoDB 多开模式，支持 SSH 隧道与断线自动重订阅。
- 新增通用中间件声明式连接表单引擎：中间件连接复用数据库表单的页面模式，标签页由声明式配置驱动，工作区/团队/云同步/钥匙串/备注由引擎统一处理，为 RocketMQ、Kafka 等后续中间件接入铺路。
- 新建连接弹框新增「时序数据库」「中间件」分类，分别承载 TDengine 与 MQTT 入口卡片。

---

#### What's New

- Added TDengine time-series database connections on the official taos WebSocket driver (via taosAdapter :6041, pure Rust with no local C dependency), with full database capabilities including database/super-table/child-table browsing, DESCRIBE with TAG awareness, and paged queries; the connection form, connection import protocol, and TDengine CLI commands (`taos` / `jdbc:TAOS-RS://`) are supported as well.
- Added MQTT middleware connections on a rumqttc runtime (MQTT 3.1.1 over rustls) with dedicated subscribe, messages, and publish views (ring-buffered messages, text/Hex toggle); connection tabs follow the MongoDB multi-tab pattern, with SSH tunneling and automatic resubscription after reconnects.
- Added a declarative connection-form engine for middleware: middleware connections reuse the database form's page model with declaratively configured tabs, while workspace/team/cloud-sync/keychain/notes are handled uniformly by the engine, paving the way for future middleware such as RocketMQ and Kafka.
- The new-connection dialog adds "Time-series Database" and "Middleware" categories, hosting the TDengine and MQTT entry cards respectively.

## [v0.16.0] - 2026-09-03

#### 更新内容

- 后台任务不再自动弹出任务面板或发送 Toast，上传/下载等进度统一通过各视图底部进度条展示，界面更安静。
- 「通用资源插件」基础框架落地：扩展可基于标准连接声明新的资源类型，并复用连接表单、工作区、首页与侧边栏生命周期；宿主统一负责扩展进程的启动、权限、健康检查与重启管理，为第三方资源类型铺路。
- SQL 补全与编辑增强：新增 UPDATE 语句表别名诊断与补全；表数据页签支持右键复制表名，列头支持右键复制字段名与注释；文件管理器支持 Backspace 返回上一级目录（输入框聚焦时不触发）。
- Oracle 支持：连接表单新增连接角色（默认 / SYSDBA / SYSOPER）；普通权限账户可正常获取 schema 补全；元数据改为表名优先发布，大 schema 下列扫描期间表名即可完成补全。

#### 修复与优化

- 修复终端手动输入密码时大写与部分特殊字符丢失导致认证失败的问题（#147）。
- 修复粘贴确认弹窗按钮被挤出可视区、大体量粘贴缺少确认的问题；长内容预览改为限高内滚动，支持合并为单行粘贴，并对非 bracketed 粘贴过滤危险控制字符。
- 修复 SQL 运行按钮偶发阻塞 UI 与指针样式、INSERT 值提示覆盖或重叠 SQL 文本的问题（#141）。
- 修复独立 MSTSC 远程桌面未传递已保存凭据的问题（#136）。
- 修复 Windows 上 Personal Sync(Git) 自动同步每次弹出黑色控制台窗口的问题。
- 修复 SQLite 启用 WAL 失败（杀软 / 同步盘 / 受限文件系统）时应用启动卡死或崩溃的问题，现会回退到 DELETE 日志模式并正常启动。
- 修复钥匙串列表滚动容器塌陷导致白屏的问题。
- 笔记：因组件库升级迁移，移除左编辑右预览的分栏（Split）模式，该模式旧配置会自动回退到源码（Source）模式。

国内下载：如果 GitHub 下载较慢，可从 [CNB 镜像](https://cnb.cool/navop-dev/navop/-/releases/tag/v0.16.0) 下载桌面端安装包

---

#### What's New

- Background tasks no longer auto-open the task panel or fire toasts; upload/download progress is now shown in each view's bottom progress bar for a quieter experience.
- A "Universal Resource Plugin" foundation lands: extensions can declare new resource types backed by a standard stored connection and reuse the connection form, workspace, home, and sidebar lifecycles; the host owns process startup, permissions, health checks, and restarts, paving the way for third-party resource types.
- SQL completion and editing: added UPDATE table-alias diagnostics and completion; table-data tabs can copy the table name from the context menu and column headers can copy field names/comments; the file manager navigates up with Backspace (ignored while an input is focused).
- Oracle: the connection form adds a connection role (Default / SYSDBA / SYSOPER); schema completion now works for limited-privilege accounts, and metadata publishes table names first so completion is available early during large-schema column scans.

#### Fixes and Improvements

- Fixed terminal authentication failures when manually typing passwords containing uppercase or certain special characters (#147).
- Fixed paste-confirm dialog buttons being pushed out of view and missing large-paste confirmation; long previews now scroll within a height limit, with an option to paste as a single line, and non-bracketed pastes filter dangerous control characters.
- Fixed the SQL Run button occasionally blocking the UI, its cursor style, and INSERT value hints overlapping/replacing SQL text (#141).
- Fixed saved credentials not being passed to the standalone MSTSC remote desktop (#136).
- Fixed Personal Sync (Git) flashing a black console window on Windows during auto-sync.
- Fixed app startup hanging or crashing when SQLite fails to enable WAL (antivirus, sync folders, restricted filesystems); it now falls back to DELETE journal mode and starts normally.
- Fixed the keychain list scrolling container collapsing and causing a blank screen.
- Notes: the split (left-edit / right-preview) mode is removed as part of the component-library migration; existing split configurations automatically fall back to Source mode.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.15.2...v0.16.0

## [v0.15.2] - 2026-08-31

#### 更新内容

- 设置入口统一迁移到全局标签栏右上角，位于后台任务入口之后；现代主页与传统主页不再重复显示设置入口。
- SQL 结果表格编辑体验增强：日期、时间和数值列会根据结果集与表结构元数据使用对应的编辑控件，非 MySQL 数据库无需为识别字段类型额外查询表结构。

#### 修复与优化

- 修复 Windows 上数据库树筛选弹窗输入时闪烁、无法持续输入或丢失 IME/焦点的问题；筛选面板改为独立宿主，并完善外部点击、Escape、焦点恢复与连接切换行为。
- 修复 MySQL 在 `character_set_results=binary` 等环境下将数据库名、表名及其他文本结果误显示为十六进制的问题；连接后显式协商结果字符集，同时继续无损保留真实二进制数据。
- 修复 Redis 树视图的搜索、刷新和添加键等图标在暗黑模式下显示为黑色的问题，纯色图标现在正确跟随主题颜色。
- 修复非 macOS 平台的全局关闭窗口快捷键占用 `Ctrl+D`，导致终端无法发送 EOF 的问题；默认快捷键现改为 `Ctrl+Shift+W`，macOS 继续使用 `Cmd+W`。

国内下载：如果 GitHub 下载较慢，可从 [CNB 镜像](https://cnb.cool/navop-dev/navop/-/releases/tag/v0.15.2) 下载桌面端安装包

---

#### What's New

- The Settings entry is now consistently placed at the top-right of the global tab bar, after Background Tasks; it is no longer duplicated in the modern or classic home navigation.
- SQL result editing is improved: date, time, and numeric columns now use type-appropriate editors based on result-set and schema metadata, without extra schema queries just to identify types for non-MySQL databases.

#### Fixes and Improvements

- Fixed the database-tree filter popover flickering, rejecting continued input, or losing IME/focus on Windows; the filter panel now uses an independent host with consistent outside-click, Escape, focus restoration, and connection-switching behavior.
- Fixed MySQL database names, table names, and other text results being displayed as hexadecimal under environments such as `character_set_results=binary`; result character sets are now negotiated explicitly while genuine binary data remains lossless.
- Fixed Redis tree search, refresh, and add-key icons rendering black in dark mode; monochrome icons now correctly follow the active theme color.
- Fixed the global close-window shortcut reserving `Ctrl+D` on non-macOS platforms and preventing terminals from sending EOF; the default is now `Ctrl+Shift+W`, while macOS continues to use `Cmd+W`.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.15.1...v0.15.2

## [v0.15.1] - 2026-08-30

#### 更新内容

- 终端新增「选中文本后高亮相同内容」：选中一段文本后，可见区域内相同文本会以淡色背景高亮，SSH 与本地终端同时生效，可在终端侧边栏设置中开关（默认开启）。
- 连接列表宽度支持持久化：拖拽调整侧栏连接树宽度后自动保存，重启应用恢复上次宽度；停靠模式侧栏与主窗口背景统一、分隔线由拖拽手柄承担，浮动模式改为浮层卡片样式（圆角 + 阴影）。
- 「自动检查更新」开关与「检查更新」按钮从通用设置页迁移到关于页面，与版本信息同页展示。

#### 修复与优化

- 修复侧边栏与命令栏图标按钮在终端/Agent 自定义主题下颜色不跟随、误显示为黑色的问题。
- 修复 SFTP 覆盖远端文件时恢复旧修改时间（mtime），导致 rsync 部署、Web/应用缓存与增量构建等基于 mtime 的变更检测误判文件未更新、继续使用旧内容的问题；现在覆盖写入后 mtime 由服务器按实际写入时间记录。

国内下载：如果 GitHub 下载较慢，可从 [CNB 镜像](https://cnb.cool/navop-dev/navop/-/releases/tag/v0.15.1) 下载桌面端安装包

---

#### What's New

- Terminal gains "highlight identical text on selection": after selecting text, matching text in the visible area is highlighted with a subtle background, working in both SSH and local terminals; toggleable in the terminal sidebar settings (on by default).
- Connection list width is now persisted: resizing the sidebar connection tree is saved automatically and restored on next launch; the docked sidebar shares the main window background with a resize-handle divider, and the floating mode adopts a card-style look (rounded corners + shadow).
- The "check for updates automatically" toggle and "Check for Updates" button move from general settings to the About page, alongside the version information.

#### Fixes and Improvements

- Fixed sidebar and command bar icon buttons rendering black instead of following the terminal/Agent custom theme colors.
- Fixed SFTP restoring the old mtime when overwriting remote files, which made mtime-based change detection (rsync deploys, web/app caches, incremental builds) treat the overwritten file as unchanged and keep serving stale content; the server now records the actual write time.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.15.0...v0.15.1

## [v0.15.0] - 2026-08-30

#### 更新内容

- 终端 SSH 监控改为通过当前 SSH 会话推送执行采集脚本，不再向远端主机写入脚本，并在监控面板顶部新增监控开关（#126）。
- 终端内联凭据/MFA 输入体验优化：用户名与验证码在终端内明文回显，密码以 `*` 掩码显示。
- 终端文件管理器新增「自动跟随终端工作目录」开关，与设置页开关复用同一持久化链路；远端路径统一归一化，修复面包屑路径重复显示。
- 笔记目录布局优化：用户显式选择的目录直接作为笔记根，不再自动创建 `files/` 子目录；新增左编辑右预览分栏模式，预览实时镜像（#109）。

#### 修复与优化

- 修复终端内联 MFA/凭据输入时按键透传平台文本输入系统，导致验证码/密码被双写。
- 修复 MySQL 连接取消 SSL 后残留参数仍强制启用 TLS。
- 修复 OpenAI Compatible 连接名含非打印 ASCII（如中文）时，出站 User-Agent 被上游拒绝的问题。
- 完善终端内联连接反馈：重连失败以红色内联提示报告、提示文案固定英文。

---

#### What's New

- Terminal SSH monitoring now runs collection through the current SSH session instead of writing scripts to the remote host, with a new monitoring toggle in the panel header (#126).
- Terminal inline credential/MFA input now echoes usernames and verification codes in plain text while masking passwords with asterisks.
- The terminal file manager gains an "auto-follow terminal working directory" toggle sharing the settings persistence path, and remote paths are normalized to fix duplicated breadcrumb segments.
- Notes directory layout improvements: an explicitly chosen directory becomes the notes root directly (no automatic `files/` subdirectory), plus a split edit-with-live-preview mode (#109).

#### Fixes and Improvements

- Fixed terminal inline MFA/credential keys passing through to the platform text-input system, double-typing verification codes and passwords.
- Fixed MySQL still forcing TLS after SSL was disabled.
- Fixed OpenAI Compatible connections whose names contain non-printable ASCII (e.g. Chinese) being rejected for non-printable User-Agent header values.
- Completed inline terminal connection feedback: reconnect failures are reported as red inline notices with English-only copy.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.14.0...v0.15.0

## [v0.14.0] - 2026-08-29

#### 更新内容

- SQL 编辑器新增跨数据库/跨 Schema 限定名补全（惰性加载），并优化 FROM 子句的数据库提示、选中数据库限定符建议与限定符元数据作用域隔离。
- SQL 格式化支持保留关键字大小写，新增格式化设置（关键字大小写、缩进）与实时预览，并通过模板掩码避免示例代码/占位符被误格式化。
- 终端将连接状态与认证提示内联显示，不再以弹窗打断操作。
- 后台任务对话框重构为带计数过滤页签，文件操作分组展示更清晰。
- 新增操作系统/网络设备图标并刷新现有图标配色；SSH 连接刷新 Linux penguin 图标并支持 FreeBSD uname 检测。
- SSH 跳板机配置在禁用后仍保留，便于快速重新启用。
- SFTP 左侧远端面板遵循配置的 SFTP 初始目录。
- 无标签页时退出应用跳过确认，加快退出。
- 扩展市场页支持「有更新」过滤，更新通知跳转只显示可更新扩展，并移除 MCP 助手分类。
- macOS 标题栏内容内边距改为可选开启，避免干扰自绘标题栏。

#### 修复与优化

- 修复首页「开始中心」最近使用列被 items_center 撑爆的响应式布局，改用主轴居中。
- 修复 SQL 编辑器输入抖动与弹层交互不稳定问题；查询工具栏按钮统一为 28px 控件高度。
- 修复会话日志删除确认后未真正删除的问题。
- 修复同步记录未保留远端时间戳的问题。
- 修复 SFTP 覆盖远端文件时未保留 owner/group/权限的问题。
- 修复回到首页时连接侧边栏折叠状态丢失的问题。
- 内部改进：统一 rustfmt 格式、修复存量测试失败、CI 新增 fast 构建模式（跳过 fat LTO 加快构建）。

国内下载：如果 GitHub 下载较慢，可从 [CNB 镜像](https://cnb.cool/navop-dev/navop/-/releases/tag/v0.14.0) 下载桌面端安装包

---

#### What's New

- SQL editor now supports cross-database/schema qualified completion with lazy loading, plus database hints after FROM, selected-database qualifier suggestions, and isolated qualifier metadata scopes.
- SQL formatting preserves keyword case; new format settings (keyword case, indentation) with live preview and balanced template masking so sample code/placeholders are not mangled.
- Terminal shows connection status and auth prompts inline instead of interrupting with popups.
- Background task dialog reworked with counted filter tabs for clearer grouped file operations.
- New OS/network device icons with refreshed colors; SSH connections refresh the Linux penguin icon and add FreeBSD uname detection.
- SSH jump server config is retained when disabled for quick re-enable.
- The SFTP left remote panel honors the configured SFTP initial directory.
- Quitting the app skips confirmation when no tabs are open.
- Extension marketplace supports an "updates available" filter; update notifications jump only to updatable extensions; the MCP Assistant category is removed.
- macOS titlebar content inset is now opt-in to avoid disturbing custom titlebars.

#### Fixes and Improvements

- Fixed the home "Start Center" recent list being stretched by items_center; centered on the main axis instead.
- Stabilized SQL editor typing flicker and popover interactions; query toolbar buttons pinned to the shared 28px control height.
- Fixed session log delete confirmation not actually deleting the log.
- Fixed synced records not preserving the remote timestamp.
- Fixed SFTP overwrite not preserving owner/group/permissions on remote files.
- Fixed the collapsed connection sidebar state being lost when returning home.
- Internal: unified rustfmt formatting, fixed pre-existing test failures, and added a fast CI build mode (skips fat LTO for quicker builds).

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.13.0...v0.14.0

## [v0.13.0] - 2026-08-28

### 中文

#### 更新内容

- SQL 编辑器大幅增强：支持函数签名提示、行内内联小组件与编辑器生命周期管理；多语句执行、`@set` 变量、IN 列表与 INSERT 子句智能提示、通配符补全；执行错误映射到准确源码位置，配合来源映射便于定位问题。
- 终端新增运行时 SSH Shell 集成：无需写入远端即可实时感知提示符、命令开始/结束与当前目录，为自动化执行与脚本提示提供基础。
- 后台任务能力升级：文件操作支持按标签页分组显示；SFTP 传输、远程删除等转移为全局后台任务；终端与后台任务面板新增传输进度展示。
- 连接树自动隐藏模式优化：点击设置、扩展、AI 工作台等非连接区域时自动收起连接树；同时支持将连接树固定为常驻侧栏。
- 连接支持自定义 SSH 图标。
- MCP 审批等待时间可配置，超时后返回 `approval_timeout`。
- AI 工具调用默认改为「手动确认」模式：模型请求业务工具前需用户确认，降低误操作风险。
- 刷新内置模型目录，接入最新模型列表。
- 会话日志支持删除操作。

#### 修复与优化

- 修复 zmodem 上传/下载的传输竞态、任务生命周期与帧解析问题，进度更新更准确且不冲突。
- 修复终端大段粘贴导致 SSH 超时的问题。
- 修复 Redis 空用户名存储为 null 导致默认用户认证超时的问题。
- 修复数据库表/对象树未按字母排序的问题，子节点排序更稳定。
- 修复带注释的 DDL 导出：补充主键、表/列注释的导出，并在驱动仅返回注释时回退到 DDL 构建器。
- 修复真 schema 驱动在表导出时错误附加数据库限定名的问题。
- 修复后台任务进度完成时未固定到 100% 的问题；进度条默认使用主题主色。
- 修复连接树浮层滚动事件传播到标签内容的问题。
- 修复文件管理器面包屑标签过长时未截断的问题。
- 后台任务展示由浮层改为独立对话框，信息更完整。
- 转发窗口表单高度调整，避免内容挤压。

---

### English

#### What's New

- Major SQL editor upgrade: function signature hints, inline widgets and editor lifecycle management; multi-statement execution, `@set` variables, IN-list and INSERT-clause smart completion, and wildcard completion; execution errors now map to precise source locations for easier troubleshooting.
- The terminal gains runtime SSH shell integration: prompts, command start/end, and the current directory are sensed in real time without any writes to the remote host, laying the groundwork for automation and script hints.
- Background task capabilities improved: file operations can be grouped by tab; SFTP transfers and remote deletes move to global background tasks; transfer progress is shown in the terminal and the background task panel.
- Connection tree auto-hide mode refined: clicking non-connection areas such as Settings, Extensions, or the AI Workbench now collapses the tree automatically; the tree can also be pinned as a fixed sidebar.
- Connections support custom SSH icons.
- MCP approval wait time is configurable and returns `approval_timeout` when it expires.
- AI tool calls now default to manual confirmation: the user confirms before the model runs business tools, reducing the risk of accidental operations.
- Refreshed the built-in model catalog with the latest model list.
- Session logs support deletion.

#### Fixes and Improvements

- Fixed zmodem upload/download transfer races, task lifecycle, and frame parsing issues; progress updates are more accurate and no longer collide.
- Fixed SSH timeouts caused by large terminal pastes.
- Fixed Redis storing an empty username as null, which caused default-user auth timeouts.
- Fixed database table/object tree children not sorted alphabetically.
- Fixed DDL export for commented objects: PRIMARY KEY and table/column comments are now exported, with a fallback to the DDL builder when a driver returns only comments.
- Fixed true-schema drivers incorrectly appending a database qualifier during table export.
- Fixed background task progress not pinning to 100% when complete; the progress bar now uses the theme primary color by default.
- Fixed scroll events from the floating connection tree propagating into tab content.
- Fixed file manager breadcrumb labels not truncating when too long.
- Background tasks now show in a dedicated dialog instead of a popover for a fuller view.
- Adjusted the forwarding window form height to avoid content squeezing.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.12.1...v0.13.0

## [v0.12.1] - 2026-08-26

### 中文

#### 更新内容

- SSH 连接支持为 SFTP 单独配置一套账户：在连接表单新增「SFTP 账户」页签，可启用独立 SFTP 用户名/密码；启用后 SFTP 传输、远程文件浏览与远程文件编辑使用该账户连接远端，SSH 终端仍使用主账户，未配置时 SFTP 与 SSH 共用一套凭据。
- SFTP 文件浏览的面包屑导航优化了最小宽度处理：根目录标签不再强制保留额外宽度，窄窗口下路径显示更紧凑。
- 连接树新增「自动隐藏」开关（默认开启）：开启时连接树以浮层显示，双击打开会话后自动收起、不再遮挡终端；关闭后连接树改为与终端并排固定的分割面板，保持展开，方便持续浏览连接列表。

#### 修复与优化

- 修复终端粘贴误拦截：粘贴带行尾反斜杠续行的多行命令（如多行 wget/curl）不再被当作「不安全的多行粘贴」硬拦截，改为走普通多行粘贴确认；对 heredoc、未闭合引号等高风险粘贴，提示框新增「仍然粘贴」按钮，可在确认后继续粘贴。
- 修复从终端复制表格内容时列对齐丢失的问题，复制结果保留原始列间距。
- 修复 RDP 连接测试（IronRDP 路径）依赖完整 RDP/TLS/NLA 认证的问题：改为 TCP 可达性探测（支持直连与代理），无需账号凭据也能快速反馈目标主机是否可达。
- 修复 Windows 下 RDP 重连时选择重连方式的仲裁对话框被自动隐藏/中断的问题，保持对话框可见直至用户选择。
- 修复从对象页签（对象树）双击打开 MySQL 表数据时数据库名为空、报 "Incorrect database name"（ERROR 42000）的问题。
- 修复查询结果编辑时带引号表名（反引号、双引号、方括号等）被二次加引号、导致 INSERT/UPDATE/DELETE 语句异常的问题。
- 修复导入数据库连接后在新连接表单中编辑并保存时误按「更新已有连接」处理的问题：现在保存为全新连接。

---

### English

#### What's New

- SSH connections can now use a separate SFTP account: a new "SFTP Account" tab in the connection form lets you enable an independent SFTP username/password. When enabled, SFTP transfers, remote file browsing, and remote file editing connect with that account while the SSH terminal keeps using the main account; when unset, SFTP and SSH share the same credentials.
- The SFTP file browser breadcrumb now handles minimum widths more smartly: the root label no longer reserves extra width, keeping the path compact in narrow windows.
- The connection tree gains an auto-hide toggle (enabled by default): when on, the tree shows as a floating overlay and collapses automatically after you open a session so it never covers the terminal; when off, the tree renders as a fixed split panel docked beside the terminal and stays expanded for continuous browsing.

#### Fixes and Improvements

- Fixed terminal paste blocking: multi-line commands with trailing backslash line continuations (e.g., multi-line wget/curl) are no longer hard-blocked as "unsafe multi-line paste" and now use the normal multi-line paste confirmation; the unsafe-paste warning for heredoc and unterminated quotes now offers a "Paste Anyway" button so you can proceed after confirming.
- Fixed copying table output from the terminal losing column alignment; copied text now keeps the original column spacing.
- Fixed the RDP connection test (IronRDP path) depending on full RDP/TLS/NLA authentication: it now performs a TCP reachability probe (direct or proxy-aware) and reports whether the target host is reachable without needing account credentials.
- Fixed the reconnect arbitration dialog on Windows RDP being dismissed/interrupted on reconnect; it now stays visible until you choose.
- Fixed opening a MySQL table from the object tab failing with "Incorrect database name" (ERROR 42000) when the database metadata was empty.
- Fixed editable result sets mis-handling quoted table names (backticks, double quotes, brackets): generated INSERT/UPDATE/DELETE statements no longer double-quote the table name.
- Fixed saving an edited database draft imported into the new-connection form treating it as an update to an existing connection; it now saves as a brand-new connection.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.12.0...v0.12.1

## [v0.12.0] - 2026-08-25

### 中文

#### 更新内容

- 扩展市场新增更新提醒：应用启动后在后台检查扩展市场，与已安装的插件版本对比，发现新版本时弹出通知，并可直接跳转到扩展市场查看更新；同一批更新仅提醒一次，确认后不再重复打扰。
- 离线安装包下载窗口现在展示全部下载渠道：扩展市场、GitHub Releases 与国内扩展下载镜像，每个渠道都支持复制地址与一键打开。
- 连接树改为浮动面板：展开连接树时不再挤压或推动终端与标签栏，点击连接也不会误收起侧栏；展开/收起按钮在窗口控件下不再跳动。

#### 修复与优化

- 表设计器 SQL 预览改为通过数据库驱动异步生成，与保存共用同一路径：方言级 DDL（如 COMMENT ON）由数据库插件生成而非宿主内置，修复 DM、金仓（Kingbase）等表/列注释修改时预览为空白或「没有需要变更的语句」的问题。
- 表设计器打开时正确回显表注释：通过 IPC 传递 Schema 匹配已加载的表，驱动未返回 Schema 时按表名兜底匹配。
- 表设计器 SQL 预览与保存增加加载状态：预览生成期间显示进度条，保存期间禁用保存按钮。
- 修复 Oracle 在对象页签右键「设计表」无法打开的问题：对象树节点 ID 统一从父节点派生，与左侧树保持一致。
- 修复多显示器场景下弹窗位置错误：新建连接、导入、设置、更新等弹窗现在会出现在当前活动窗口所在的屏幕。
- 修复数据比较中 JSON 字段控制字符显示不一致的问题：比较面板不再将 `\r\n` 显示为自动换行的多行文本，与查询面板保持一致。
- 修复 Moonshot Kimi 系列模型（kimi-k2 等）调用报错的问题：强制使用模型要求的 temperature=1。
- 授权协议调整：允许免费渠道分发 Navop，禁止商业转售。

---

### English

#### What's New

- Extension marketplace update notifications: Navop now checks the extension marketplace in the background on startup, compares it with installed plugin versions, and shows a notification when updates are available, with a direct link to view them in the marketplace; each batch of updates is announced only once.
- The offline package download dialog now lists all download channels: the extension marketplace, GitHub Releases, and the domestic mirror, each with copy-address and open buttons.
- The connection tree is now a floating panel: expanding it no longer squeezes or pushes the terminal/tab bar, clicking a connection no longer accidentally collapses the sidebar, and the expand/collapse toggle no longer jumps under the window controls.

#### Fixes and Improvements

- Table designer SQL preview is now generated asynchronously by the database driver, sharing the same code path as saving: dialect-specific DDL such as `COMMENT ON` is produced by the IPC plugin instead of a host-local builder, fixing blank or "no changes detected" previews when editing table/column comments on DM, Kingbase, and others.
- The table designer now echoes the table comment on load by plumbing the schema through IPC, falling back to matching by table name when a driver does not report a schema.
- Table designer SQL preview and save now show loading states: a spinner while the preview is generated and a disabled save button while DDL is built and executed.
- Fixed "Design Table" from the object tab for Oracle by deriving object tree node IDs from their parent node so they match the left-side tree.
- Fixed popup placement on multi-monitor setups: dialogs such as New Connection, Import, Settings, and Update now appear on the screen of the active window.
- Fixed inconsistent JSON control-character rendering in data comparison, so `\r\n` no longer wraps into multi-line text and now matches the query panel.
- Fixed invocation errors with Moonshot Kimi models (kimi-k2 and newer) by forcing the model-required `temperature=1`.
- License update: free distribution channels are permitted; commercial resale is prohibited.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.11.0...v0.12.0

## [v0.11.0] - 2026-08-24

### 中文

#### 更新内容

- 连接列表新增排序设置：「设置 → 通用 → 连接显示」新增「连接排序」，默认按名称自然排序（IP 地址等数字段按数值比较、忽略大小写），也可切换为「最近使用优先（LRU）」；首页连接列表、Redis/MongoDB 工作区标签页与持久侧栏连接树统一应用该配置，切换后即时生效。
- SSH 新增对老旧服务器的可选兼容支持：在连接「高级设置」中开启「允许旧版 SSH 算法」后，可连接仅支持 DSA 主机密钥、SHA-1 密钥交换/MAC 或 1024 位 DH 组协商的旧设备，并针对「Key exchange init failed」问题调整协商参数与顺序，同时完善相关错误提示。
- 标签页改进：复制标签页自动追加序号（例如 192.168.1.1 → 192.168.1.1(1)），并复用已释放的编号；标签宽度按内容自适应，不再截断长标题。
- 更新依赖以提升安全性与功能：升级 clickhouse、sqlparser、russh、russh-sftp 等依赖，并引入 gpui-ce 剪贴板修复。

#### 修复与优化

- 修复原生 RDP 遮挡对话框与关闭流程问题，改进原生窗口叠加层、连接状态显示与剪贴板同步重试回退。
- 修复 MySQL 数据库导出时 LONGTEXT 字段未能作为文本正确导出的问题。

---

### English

#### What's New

- Added configurable connection sorting under **Settings → General → Connection Display**: a new "Connection Sorting" option defaults to natural name order (numeric segments such as IP addresses compared by value, case-insensitive) with "Most Recently Used" (LRU) also available; the Home connection list, Redis/MongoDB workspace tabs, and the persistent connection sidebar tree all honor the setting and refresh immediately on change.
- SSH now offers opt-in compatibility for legacy servers. With "Allow Legacy SSH Algorithms" enabled under **Advanced Settings**, Navop can connect to old devices that only support DSA host keys, SHA-1 key exchange/MAC, or 1024-bit DH group negotiation, with adjusted negotiation parameters and order that avoid "Key exchange init failed", plus clearer error messages.
- Improved tabs: duplicated tabs are automatically numbered (e.g. `192.168.1.1` → `192.168.1.1(1)`), reusing freed numbers, and tab widths now adapt to content so long titles are not truncated.
- Updated dependencies for security and functionality: clickhouse, sqlparser, russh, and russh-sftp were upgraded, and a gpui-ce clipboard fix was included.

#### Fixes and Improvements

- Fixed native RDP overlay dialogs and the close flow, and improved native window overlay handling, connecting status display, and clipboard retry/backoff.
- Fixed MySQL export so `LONGTEXT` fields are correctly exported as text.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.10.10...v0.11.0

## [v0.10.10] - 2026-08-24

### 中文

#### 修复与优化

- 修复 Windows RDP 独立全屏窗口的兼容性与稳定性问题：从连接右键菜单打开独立窗口时，改用系统远程桌面客户端 `mstsc.exe` 启动全屏会话，避免内嵌原生窗口可能出现的白屏、焦点和退出异常。
- 支持将主机名、IPv4、IPv6 与自定义端口正确传递给系统远程桌面客户端；参数无效或外部程序启动失败时会在 Navop 中显示明确提示。
- 修复 Windows 原生 RDP 会话关闭超时后标签页可能无法完成关闭的问题；超时隔离原生组件后，Navop 现在会正确收敛标签页关闭流程。
- 修复关闭当前标签页后剩余标签页未正确激活、聚焦，以及延迟激活事件可能让空标签容器覆盖新版首页的问题。
- 非 Windows 平台及 VNC 独立窗口继续使用 Navop 内置窗口，不受本次调整影响。

---

### English

#### Fixes and Improvements

- Fixed compatibility and stability issues with dedicated fullscreen Windows RDP windows. Opening a dedicated window from a connection's context menu now launches the system Remote Desktop client (`mstsc.exe`) in fullscreen, avoiding white-screen, focus, and exit issues that could occur with the embedded native window.
- Correctly passes hostnames, IPv4/IPv6 addresses, and custom ports to the system Remote Desktop client, with clear in-app messages when the connection parameters are invalid or the external program cannot be launched.
- Fixed an issue where a Windows native RDP tab could remain open after native shutdown timed out. Once the native component is quarantined, Navop now completes the tab close flow correctly.
- Fixed lifecycle and focus restoration for the remaining tab after closing the active tab, and prevented a delayed activation event from replacing the modern home page with an empty tab container.
- Dedicated VNC windows and remote desktop windows on non-Windows platforms continue to use Navop's built-in window and are unaffected by this change.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.10.9...v0.10.10

## [v0.10.9] - 2026-08-24

### 中文

#### 更新内容

- 标签页新增会话锁定功能：可通过密码锁定/解锁会话（密码仅保存在内存中），支持「锁定全部会话」与「隐藏输出」，锁定中的终端会拒绝键盘输入，且无法通过关闭按钮直接关闭。
- 标签页新增 SecureCRT 风格的连接状态徽章：已连接、断开、已连接并锁定等状态以不同图标显示，并带悬浮提示。
- 连接导入新增 SecureCRT 会话与快捷命令支持，支持手动扫描目录，并展示扫描到的可用工作区分组。
- SSH 支持可选的旧版 ssh-dss 主机密钥认证，并在 Windows 上新增 Pageant 认证。
- 终端快捷命令编辑器新增「点击执行」选项，点击命令即可自动回车执行；终端设置新增建议弹窗独立开关，并增强设置面板与命令栏功能。
- Telnet 连接支持自定义退格键编码。
- 数据库工作区改进：MySQL/PostgreSQL 表信息视图新增表大小、索引数等信息；SQL 导出保留 Schema 元数据并支持使用当前选中的数据库；二进制与文本值（含 MySQL BIT、文本 sidecar、空二进制）在显示、编辑、导入导出等数据工作流中得到更好保留；修复字符类型显示与编辑问题。
- 数据库比较功能优化：改进结果布局与差异浏览，差异详情列表采用虚拟化渲染，比较问题区域支持滚动查看。
- 连接表单统一 SSH 隧道配置，减少重复填写。
- Windows 原生 RDP 全面重构初始化与关闭生命周期，修复白屏与崩溃问题，默认在标签页中打开；独立全屏窗口改为从连接右键菜单打开且默认激活呈现，支持通过顶部悬停显示标题栏并按 ESC 退出全屏；仅保留 Windows 原生 MSTSC 与 IronRDP 后端。
- 其他改进：窗口跨显示器恢复位置、SFTP 支持延迟凭据提示、PostgreSQL 瞬时连接失败自动重试、RDP 标准化 Windows 剪贴板文件路径、首页快速打开连接改为双击触发、补充国际化文案。

#### 修复与优化

- 修复 Windows 原生 RDP 初始化与关闭期间的崩溃和白屏问题。
- 修复数据库字符类型显示与编辑，以及二进制/文本值在数据工作流中丢失的问题。
- 修复终端 ZMODEM 探测输出停滞、AI 聊天侧栏切换标签后滚动位置丢失等问题。
- 修复窗口在多个显示器之间切换后无法恢复位置的问题。

---

### English

#### What's New

- Added session locking to tabs: lock and unlock sessions with a password kept only in memory, with "Lock All Sessions" and "Hide Output" options; locked terminals reject keystrokes and cannot be closed via the close button.
- Added SecureCRT-style connection status badges to tabs, showing connected, disconnected, and connected-and-locked states with tooltips.
- Added SecureCRT session and quick-command import, with manual directory scanning and surfaced scanned workspace groups.
- Added opt-in legacy ssh-dss host-key support for SSH, and Pageant authentication on Windows.
- Quick-command editor now supports "execute on click" to run a command immediately, added an independent toggle for the suggestion popup in terminal settings, and enhanced the settings panel and command bar.
- Added configurable backspace code for Telnet connections.
- Improved the database workspace: MySQL/PostgreSQL table views now show table sizes and index counts; SQL exports preserve schema metadata and can use the currently selected database; binary and text values (including MySQL BIT, text sidecars, and empty binary) are better preserved across display, editing, import, and export workflows; fixed character-type display and editing.
- Improved database comparison with better result layout and diff browsing, virtualized diff-detail lists, and scrollable comparison issues.
- Unified the SSH tunnel form in connection forms to reduce repeated configuration.
- Rebuilt Windows native RDP initialization and shutdown lifecycle to fix white screens and crashes, opening in a tab by default; the dedicated fullscreen window is available from the connection context menu, starts as the active presentation, reveals its title bar on top-edge hover, and exits fullscreen with Escape; only the Windows-native MSTSC and IronRDP backends remain.
- Other improvements: window placement is restored across displays, SFTP prompts for delayed credentials, PostgreSQL retries transient connection failures, RDP normalizes Windows clipboard file paths, quick-open on the home page now triggers on double-click, and additional i18n text was added.

#### Fixes and Improvements

- Fixed crashes and white screens during Windows native RDP initialization and shutdown.
- Fixed database character-type display and editing, and the loss of binary/text values across data workflows.
- Fixed stalled ZMODEM probe output in terminals and AI Chat sidebar scroll position after switching tabs.
- Fixed window placement not being restored when switching between multiple displays.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.10.8...v0.10.9

## [v0.10.8] - 2026-08-20

### 中文

#### 更新内容

- 新增 Telnet 连接支持，并支持自动登录脚本和手动凭据覆盖。
- 新增会话日志和静态终端历史查看器，支持滚动查看、文本选择、搜索与 TXT 导出，可查看 SSH、串口和本地终端的活动日志。
- 凭据管理新增可复用的钥匙串引用与个人钥匙串同步，改善凭据跨连接复用和跨设备同步体验。
- 终端新增可复用的快捷命令及全局作用域，并改进快捷键捕获、历史建议、自定义串口波特率、重连和分屏交互。
- Markdown 编辑器支持点击 Mermaid 图和数学公式放大查看，并可在源码与预览之间切换；同时优化表格操作栏和渲染内容交互。
- 数据库工作区新增 SQL 执行历史侧栏，并持久化保存历史记录，方便快速回看和复用查询。
- 改进数据库 Schema/Data Compare，增强跨数据库列类型映射、目标表匹配、差异浏览和同步计划安全性，并修复新增表索引与外键遗漏问题。
- Oracle 连接配置新增 Native 与纯 Go 驱动选择，改善编辑连接时的驱动模式保留，并支持 Oracle 11g 查询分页限制。
- Linux 发布包新增 x64 与 ARM64 便携版，并改善旧版 ARM64 运行时、Wayland 依赖和 usrmerge 环境下的兼容性。

#### 修复与优化

- 修复终端 Escape 被清除选区快捷键拦截的问题，Vim 等终端程序现在可正常接收 Escape。
- 改进 MySQL BIT 和二进制值在 SQL 导入导出、表格编辑和数据网格中的保留与编辑，避免值在格式化或保存过程中丢失。
- 改善凭据存储、SSH/Telnet 登录、终端重连、窗口快捷键和 AI Chat 侧栏滚动等稳定性问题。
- Public MCP 终端工具支持发送原始按键输入，便于自动化处理交互式终端场景。

---

### English

#### What's New

- Added Telnet connections with automatic login scripts and manual credential overrides.
- Added session logs and a static terminal history viewer with scrollback, text selection, search, and TXT export for SSH, serial, and local terminal sessions.
- Added reusable keychain references and personal keychain sync for easier credential reuse across connections and devices.
- Added reusable terminal quick commands with global scope, and improved shortcut capture, history suggestions, custom serial baud rates, reconnect behavior, and split-pane interaction.
- Markdown editor previews for Mermaid diagrams and math formulas can now be enlarged and switched between source and preview, with improved table controls and rendered-content interaction.
- Added a persistent SQL execution history sidebar to the database workspace for quickly revisiting and reusing previous queries.
- Improved database schema and data comparison with cross-database column-type mapping, better target-table matching, clearer diff navigation, safer sync-plan execution, and fixes for missing indexes and foreign keys on new tables.
- Added Native and pure-Go driver choices for Oracle connections, improved driver-mode preservation when editing connections, and added Oracle 11g query-limit support.
- Added portable x64 and ARM64 Linux packages and improved compatibility with older ARM64 runtimes, Wayland dependencies, and usrmerge-based systems.

#### Fixes and Improvements

- Fixed Escape being intercepted by the terminal clear-selection shortcut, allowing Vim and other terminal applications to receive Escape normally.
- Improved preservation and editing of MySQL BIT and binary values across SQL import/export, table editing, and data grids.
- Improved credential storage, SSH/Telnet login, terminal reconnects, window shortcut handling, and AI Chat sidebar scrolling.
- Public MCP terminal tools can now send raw key input for interactive automation scenarios.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.10.7...v0.10.8

## [v0.10.7] - 2026-08-13

### 中文

#### 更新内容

- 终端工作区新增面板分屏控制，可向左、右、上、下拆分终端，并可将面板恢复为普通标签页。
- 连接侧边栏新增批量选择与管理模式，可选择当前可见连接、批量移动到分组或批量删除，并将相关入口整合到溢出菜单。
- SSH 连接支持配置终端类型，改善不同远程系统和 shell 环境下的兼容性。
- SQL 查询新增无限结果模式和执行中取消能力。
- 统一辅助窗口的关闭行为，并使用对应平台的标准窗口关闭快捷键。

#### 修复与优化

- 表数据导入支持事务执行与二进制安全处理，失败时可回滚，避免留下部分导入数据；SQL 导出现在也会正确保留二进制值。
- 改善 SSH 多因素及 keyboard-interactive 认证流程，保留终端缓冲区并支持继续完成多步认证。
- 修复终端长行在可见视口中的换行，以及调整窗口大小后的内容重新排版问题。
- 限制 AI Chat 会话记录、缓存会话和工具信息的内存占用，提升长时间会话的稳定性。
- 为远程桌面帧增量、扩展驱动 worker、Public MCP 审批队列、SSH 路径补全缓存和远程文件外部编辑会话增加容量或生命周期限制，降低长期运行时的资源堆积风险。
- 优化大型 DML 执行后的数据库缓存失效判断，避免不必要地解析完整 SQL。
- 修复表格复制选择可能超出有效列范围的问题。
- 将 Redis 驱动最低兼容版本更新至 `0.1.4`，以支持原生 pipeline 与连接断开后的恢复能力。

---

### English

#### What's New

- Added terminal pane controls for splitting a terminal to the left, right, top, or bottom, with an option to restore a pane to a regular tab.
- Added batch selection and management to the connection sidebar, including selecting visible connections, moving multiple connections to a group, and deleting them in one operation, with the related actions consolidated into the overflow menu.
- Added configurable SSH terminal types for better compatibility with different remote systems and shell environments.
- Added an unlimited-results mode and cancellation for running SQL queries.
- Unified auxiliary-window close behavior and aligned shortcuts with each platform's standard window-close action.

#### Fixes and Improvements

- Made table imports transactional and binary-safe so failures can roll back without leaving partial data, and fixed SQL exports to preserve binary values correctly.
- Improved SSH multi-factor and keyboard-interactive authentication by preserving terminal buffers and allowing multi-step authentication to continue.
- Fixed terminal soft-wrapping within the visible viewport and content reflow after resizing the window.
- Bounded AI Chat transcripts, cached sessions, and tool information to improve stability during long-running conversations.
- Added capacity or lifecycle limits for remote-desktop frame deltas, extension-driver workers, Public MCP approval queues, SSH path-completion caches, and external remote-file editing sessions to reduce resource buildup during long-running use.
- Optimized database cache invalidation after large DML statements by avoiding unnecessary full-SQL parsing.
- Fixed table copy selections that could extend beyond the valid column range.
- Updated the minimum compatible Redis driver version to `0.1.4` to support native pipelines and recovery after dropped connections.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.10.6...v0.10.7

## [v0.10.6] - 2026-08-11

### 中文

#### 更新内容

- 工作区文件浏览器新增文件和目录的剪切、复制、粘贴操作，并支持 macOS `Cmd-X/C/V` 与其他平台 `Ctrl-X/C/V` 快捷键。
- 全面增强 SSH/SFTP 文件管理：本地与远程操作菜单改为更清晰的下拉菜单，补充文件剪贴板、远程命令执行、复制进度与取消，并重构服务器间复制流程，支持直传认证、自动配置源端 SSH key、保留 SSH proxy 设置以及仅中继模式。
- SFTP 新增未知或变更主机密钥确认，可选择拒绝、仅本次接受或接受并保存；文件列表同时支持显示所有者用户名，并分别保存左右面板的隐藏列配置。
- 降低远程桌面的显示延迟，优化帧呈现、纹理上传与资源回收流程，并支持从 macOS Finder 向远程桌面复制文件。
- 数据库工作区查询支持在可用连接之间选择，并同步当前连接、数据库和 Schema 上下文；关闭未命名 SQL 查询时可选择取消、放弃保存或命名后保存。
- SSH 终端支持在连接过程中请求运行时凭据；Agent 新增可配置的迭代次数上限，并改善聊天消息复制内容。

#### 修复与优化

- 修复 SSH 多因素认证过程中 OTP 提示可能丢失的问题。
- 修复 SFTP 服务器直传可能卡住、缺少源端 key、丢失源端 SSH proxy 设置以及未知主机密钥无法处理等问题。
- 工作区侧栏现在会持久化折叠状态并可隐藏空工作区，同时将工作区名称唯一性限制在同一父工作区内。
- 修复表格多行复制时可能重复生成列的问题。
- 修复 MCP 启动器必须预先解析 `npx` 路径的问题，现在会直接执行 `npx`。
- 改善主页与 Tab 系统的兼容性，修复无 Tab、从主页切换或使用旧版主页导航时 Tab 栏和导航入口可能不可见的问题。
- 修复流式执行 DDL 后 Schema 元数据缓存未及时失效的问题，并优化 SSH 连接表单的界面布局。

---

### English

#### What's New

- Added cut, copy, and paste for files and directories in the workspace explorer, with `Cmd-X/C/V` shortcuts on macOS and `Ctrl-X/C/V` on other platforms.
- Expanded SSH/SFTP file management with clearer drop-down action menus, file clipboard operations, remote command execution, copy progress and cancellation, plus a reworked server-to-server copy flow with direct-transfer authentication, automatic source-side SSH key setup, preserved SSH proxy settings, and a relay-only mode.
- Added confirmation for unknown or changed SFTP host keys, with reject, accept-once, and accept-and-save choices. File listings can also show owner usernames and persist hidden-column preferences independently for the left and right panes.
- Reduced remote desktop display latency, optimized frame presentation, texture uploads, and resource cleanup, and added support for copying files from macOS Finder to a remote desktop session.
- Workspace database queries can now select among available connections while synchronizing the active connection, database, and schema context. Closing an unnamed SQL query now offers cancel, discard, or save-with-a-name choices.
- SSH terminals can request runtime credentials during connection. Agent settings now include a configurable iteration limit, and copied chat-message content has been improved.

#### Fixes and Improvements

- Fixed OTP prompts being lost during SSH multi-factor authentication.
- Fixed direct SFTP server-to-server copies that could hang, omit source-side keys, lose source SSH proxy settings, or fail to handle unknown host keys.
- Workspace sidebar collapse state is now persisted, empty workspaces can be hidden, and workspace-name uniqueness is scoped to the parent workspace.
- Fixed duplicate columns being produced when copying multiple table rows.
- Fixed MCP launcher startup by executing `npx` directly instead of requiring its path to be resolved first.
- Improved compatibility between the home page and the tab system, fixing cases where the tab bar or navigation entry could disappear with no tabs or while switching from the legacy home page.
- Fixed stale schema metadata after streaming DDL execution and improved the SSH connection form layout.

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.10.5...v0.10.6

## [v0.10.5] - 2026-08-07

### 中文

#### 更新内容

- SSH 终端新增可配置字符集，支持 UTF-8、GBK、GB18030、Big5、Shift_JIS、EUC-JP、EUC-KR 和 Windows-1252，改善旧系统及非 UTF-8 环境的显示与输入。
- 终端右键菜单新增“粘贴选中内容”，可直接将当前选中的文本发送到终端。
- SSH 主机指纹发生变化时新增安全确认，展示新旧指纹并提示中间人攻击风险，需明确确认后才能更新或临时接受。

#### 修复与优化

- 修复 Agent 上下文压缩模型调用失败时任务会中断的问题，现在会使用本地摘要继续执行，同时保留取消操作语义。
- 修复弹出菜单在搜索或内容更新后可能丢失键盘焦点的问题，并在关闭时正确恢复此前焦点。
- 修复 AI Chat 切换资源上下文后最新消息可能不可见的问题，现在会自动滚动到最新消息。
- 改善 SSH 和 SFTP 的连接失败诊断以及 SSH 终端运行时错误展示：日志和断开界面会保留完整错误上下文，便于定位连接、输入发送、解析及会话运行问题。
- 改善旧版 SSH 服务器兼容性；明确启用“允许旧版 SSH 算法”后，SSH 和 SFTP 支持更多 SHA-1 密钥交换算法，默认仍保持关闭。

---

### English

#### What's New

- Added configurable SSH terminal encodings, including UTF-8, GBK, GB18030, Big5, Shift_JIS, EUC-JP, EUC-KR, and Windows-1252, improving display and input for legacy and non-UTF-8 environments.
- Added “Paste Selected Text” to the terminal context menu, allowing the current selection to be sent directly to the terminal.
- Added explicit security confirmation when an SSH host key changes, showing the new and previously trusted fingerprints and warning about possible man-in-the-middle attacks before allowing an update or one-time acceptance.

#### Fixes and Improvements

- Fixed Agent tasks stopping when context-compaction model calls fail; a local fallback summary is now used while preserving cancellation behavior.
- Fixed keyboard focus being lost in popovers during search or content updates, and correctly restored the previously focused element when the popover is dismissed.
- Fixed the latest message becoming hidden after changing the AI Chat resource context; the view now scrolls to the newest message automatically.
- Improved SSH and SFTP connection diagnostics and SSH terminal runtime error reporting. Logs and the disconnect UI now preserve full error context for connection, input, parser, and session failures.
- Improved compatibility with older SSH servers by supporting additional SHA-1 key-exchange algorithms for SSH and SFTP when “Allow Legacy SSH Algorithms” is explicitly enabled; the setting remains disabled by default.

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
