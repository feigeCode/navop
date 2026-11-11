# 内置 AI 的资源实时状态（ResourceContextRefresher）

> 文档状态：Implemented Design
>
> 目标读者：维护 AI 资源池（`ai_chat_view`）与各侧边栏 AI 面板（`terminal_view` / `db_view` /
> `redis_view` / `mongodb_view`）的开发 Agent
>
> 编写日期：2026-10-26

---

## 1. 问题

侧边栏内置 AI 的资源池（`ResourceContext`）**只在面板创建时构建一次**：终端侧边栏早在
`TerminalSidebar::new` 就把 `terminal_ai_resource` 交给 `DefaultAgentChatPanel`，之后不再回写。
于是模型看到的永远是「终端刚打开时」的样子：

- 用户 `cd` 到别的目录、在 `vim` 里编辑文件、选中一段报错，AI 全都不知情；
- 只有主动调用 `terminal.read` 才能看到内容，而模型往往不会主动调用，或要先
  `connections.list_sessions` 绕一圈（SSH 侧边栏的默认目标还是连接而非终端 session）。

模型「感知现场」的成本，本可以为零。

## 2. 设计

分两层，**状态主动推送、内容按需拉取**：

| 层 | 机制 | 位置 |
| --- | --- | --- |
| 状态 | 每轮发送前重建资源池，把终端实时状态投影成资源作用域 | `ai_chat_view::ResourceContextRefresher` + `terminal_view::ai_live_context` |
| 内容 | 完整屏幕内容仍由 `terminal.read` 按需读取，可省略 target 且返回全屏/选中/光标现场 | `public_mcp` |

### 2.1 每轮重建（地基）

`AgentChatView` 新增可选回调：

```rust
pub type ResourceContextRefresher =
    Arc<dyn Fn(&App) -> Option<ResourceContext> + Send + Sync>;
```

`start_local_submission` 在应用 `@` 提及之前调用它，命中时 `session.set_resources(...)`，
后续 `AgentTask` 每步重建 system prompt 时自然带上最新资源池
（`agent_runtime::tasks::agent` 每步用 `ctx.resources` 重新渲染 `{{resource_info}}`）。
回调在主线程执行，因此可以安全读取 `Entity<Terminal>` 等实体状态；返回 `None` 表示沿用旧池。

资源池由宿主注入（`DefaultAgentChatPanel::set_resource_context_refresher` → `AgentChatView`），
所以任何侧边栏都能复用，不需要改 `agent_runtime` 的 prompt 管道。

### 2.2 终端状态投影

`crates/terminal_view/src/ai_live_context/` 把 `Terminal` 的现场投影成作用域：

| 作用域键 | 取值来源 | 说明 |
| --- | --- | --- |
| `cwd` | `current_working_dir()`（OSC 7） | 用户当前目录 |
| `screen` | `mode().contains(ALT_SCREEN)` + `title()` | 普通 shell 还是全屏程序（vim/less/htop），后者提示「按键会进程序」 |
| `selection` | `selection_text()` | 行数/字符数 + 单行摘要，正文仍走工具 |
| `size` | `rows()` / `cols()` | 排版相关建议需要 |
| `process` | `child_exited()` | 终端进程已退出时提示 |

投影只替换本模块负责的键（`apply_live_scopes`），宿主自己写入的作用域（如连接信息）保留。
选中内容只进摘要、不进正文：正文每步重发会被反复计费，且会把终端输出反复送出。

### 2.3 默认目标

分两种侧边栏，默认目标保持不变：

| 侧边栏 | 默认目标 | 资源列表 | 模型拿到的信息 |
| --- | --- | --- | --- |
| 本地终端 | 终端自身 | 终端 + 连接候选 | 终端的 `cwd`/`screen`/`selection`… |
| SSH 终端 | 仍是连接 | 连接（首位）+ 这台终端 + 其它连接 | 连接上 `terminal=<终端 id>`，终端上 `connection=<连接 id>` 与实时状态 |

SSH 终端不抢默认目标，是因为连接级工具（`sftp.*`、`ssh.exec`）原本以连接为默认目标，
改默认目标会让它们全部需要显式 target。代价只是模型要多看一行 `terminal=…`；
换来的是它不必先 `connections.list_sessions` 再筛 session，且选中/全屏状态第一轮就可见。

连接被「禁止 AI 操作」时连接不进 catalog，此时刷新器返回 `None`，沿用宿主静态资源池 ——
不能只把终端塞进池子，否则等于绕过连接级策略。

### 2.4 SSH 终端的接线

`TerminalSidebar::new` 在连接模式下把 AI 默认目标的资源 id 记在
`ai_connection_resource` 字段里，`view/initialization.rs` 建完侧边栏后把它交给
`live_terminal_resource_refresher`。刷新器每轮重新读 `load_terminal_ai_connections`，
所以连接的改名、删除、AI 策略变更都会在下一轮体现。

### 2.5 内容层：`terminal.read` 省略 target 与现场字段

状态层只回答「在哪、在干什么」，正文仍按需拉取，但去掉两次绕路：

- **target 可省略**：`public_mcp` 的 agent 适配器（`agent_runtime_adapter`）在建
  `ToolSpec` 时把 `target` 从 `required` 里摘掉，并替换 `target` 的描述；模型不传
  target 时，`execute` 用 `ResourceContext::default_target_of_kind` 补上会话里唯一的
  终端资源。MCP 直连表面（外部 CLI）的 schema 不变，仍强制 target，避免把
  「当前资源」这种只存在于 agent 会话的概念泄漏给外部客户端。
- **补不上就明说**：没有终端资源、或同类资源多于一个时，工具直接报错并让模型先
  `connections.list_sessions` 再传 id，不猜。
- **返回现场**：`TerminalReadResult` 增补 `alt_screen`（全屏程序占屏，此时正文是
  它的界面）、`display_offset`（用户是否在回看历史）、`cursor_row` / `cursor_column`、
  `selection`（用户当前选中的原文）。都在终端锁内一次取全，避免多次加锁读到不一致状态。

有了这三项，模型「读一次」就能判断该按 esc 还是该输入命令，而不是把 vim 界面当
shell 输出继续分析。

### 2.6 意图层：选中正文随消息发送

状态层只说「选中了 12 行」，正文本身由 `ContextAttachment` 通道送进这一轮消息：

- **宿主提供正文**：`AgentChatView::set_context_attachment_provider` 接收
  `Fn(&App) -> Option<ContextAttachment>`；终端侧边栏在初始化时接上
  `live_terminal_context_attachment_provider`，每次发送前读一次终端选区。
- **拼在用户输入前面**：`compose_prompt_with_attachment` 把正文块放在用户问题之上
  （空正文原样返回）。发送后的转录里能看到同一份正文，用户回看时知道自己送出了什么。
- **标签界定**：正文用 `<terminal_selection source=… lines=… truncated=…>` 包住，
  选中内容里含代码围栏或角括号也不会串行；超过 8000 字截断并标 `truncated="true"`，
  剩下的部分交给 `terminal.read`。
- **与作用域配合**：`selection` 作用域追加 `full text is attached to this message`，
  模型知道正文就在消息里，不用再调工具。
- **只作用于本地内置会话**：ACP 会话有自己的输入通道，不经过这里。

### 2.7 cwd 进入资源池，并成为 `ssh.exec` 的默认目录

§2.2 的作用域只对「侧栏内置 AI 的当前轮」有效，另有两处缺口，现已补齐：

- **资源池**：`PublicMcpSessionInfo.cwd` 本来就随终端事件刷新，但
  `terminal_session_resource`（Public MCP / 工作台）与 `agent_resource_from_session`
  （侧栏初始资源）都没把它写成作用域，`connections.list_sessions` 返回的终端于是只剩
  标题里的 `root@host:/path`。现在两处都写成 `cwd` 作用域，键统一为
  `public_mcp::registry::SESSION_CWD_SCOPE`，与 `ai_live_context` 共用，避免两套视图漂移。
- **执行**：`ssh.exec` 省略 `cwd` 时使用目标终端上报的当前目录
  （`SshRemoteOpsHandle::effective_cwd`），显式 `cwd`（含 `~` / `$HOME`）仍然优先。
  这样即使模型忽略作用域，相对路径也落在用户眼前那个目录，而不是 channel 默认的
  `$HOME`。cwd 前缀统一走 `cwd_prefix`：普通路径加单引号（OSC 7 路径可能含空格），
  `~` / `$` 开头保持未引用以便 shell 展开。
- **可省略 target 的执行入口**：`terminal.exec` 与 `terminal.read` 一样允许省略 `target`，
  由会话的默认终端兜底；多个终端而又没有当前终端时仍然报错，不猜。
  `terminal.control` / `terminal.write_keys` 继续保持必须显式 target。
- **侧栏系统指令**：`terminal_ai_system_instruction` 增加「作用域即现场」「路径工作走
  `terminal_exec`」「`ssh_exec` 显式覆盖 cwd」「没有 cwd 先确认」四条规则，让模型不依赖
  用户每轮重复贴自定义指令。

多跳会话（堡垒机里再 `ssh`）上报的 cwd 属于内层机器，`ssh.exec` 默认套用会在外层
`cd` 失败——错误是可见的（`cd: No such file`），模型可按提示显式覆盖；这与文件管理器
同样直接采用 OSC 7 路径的既有取舍一致。

ACP 会话仍然拿不到这些状态（见 §4）。

## 3. 不做什么

- **不把整屏塞进 prompt**：40 行约 600–800 token/轮，还会随对话历史反复发送。
- **不写「每轮必须读屏」的硬规则**：多一轮工具调用（3–5s 延迟），且模型会读没用的内容；
  有了 `screen` / `process` 作用域，它自己判断该不该读。
- **不把状态放进用户消息**：会污染历史。资源池每轮重建、不落历史，天然 ephemeral。
- **不把选中正文放进资源作用域**：作用域每个 step 都会重发，长正文会反复计费；
  摘要进作用域、正文随消息发一次。
- **不在数据库/Redis/Mongo 侧边栏先行接入**：它们没有「屏幕现场」概念，等有明确需求再复用
  同一个 `ResourceContextRefresher`。

## 4. 验证

- `cargo test -p terminal_view ai_live_context`：投影/摘要/替换/`describe()`/SSH 接线/附件包装共 16 条单测。
- `cargo test -p ai_chat_view`：943 条既有测试 + 2 条附件拼接测试通过（资源池、提及、会话切换无回归）。
- `cargo clippy -p terminal_view -p ai_chat_view --all-targets`：新增代码无告警。
- `cargo test -p terminal`: `recent_text_tracks_alt_screen_mode`、
  `recent_text_reports_selection_and_display_offset` 覆盖全屏/选中/回滚现场。
- `cargo test -p public_mcp`: 45 条单测（含 4 条 target 可省略：schema、描述、注入、
  多终端不猜）+ 14+ 条集成测试；`terminal.read` 现场字段的透传。
- 手工：终端里 `cd` / 进 `vim` / 选中若干行后提问，观察模型是否在首轮直接引用当前目录与
  全屏状态；`terminal.read` 不传 target 时是否读到当前终端。

## 5. 已知取舍

- **ACP 不经过资源池与侧栏系统指令**：ACP agent 只有 `session/new` 的本地 workspace root，
  看不到远端 cwd；要让它知道现场，需另做 prompt 前言或依赖它主动 `connections.list_sessions`。
- **多跳会话的 cwd 归属**：`ssh.exec` 默认继承 cwd 不校验上报主机，与外层机器不一致时
  表现为 `cd` 失败（可见错误），而不是静默落到错误目录。

- **前缀缓存**：资源池位于 system prompt，作用域变化会让前缀失效。但 `{{current_time}}`
  精确到秒、每个 step 都在变，前缀缓存当前并未生效；若要修复缓存，应把时间与资源池一起
  挪到消息尾部，而不是单独处理资源池。
- **选中内容时效**：只在发送瞬间抓取；发送后用户继续选中不会影响本轮。
- **选区不清空就重复发送**：选区在发送后仍留在终端里，下一次提问会再附带一次。要做到
  「发送即清空」，需要给 provider 可写的上下文（当前签名是只读 `Fn(&App)`）或让侧边栏
  订阅发送事件后清选区，等有实际抱怨再改。
- **状态读取有锁**：`mode()` / `selection_text()` 需要 `FairMutex`，与解析线程争锁的临界区
  很短，尚未观察到发送卡顿；如遇大量输出导致卡顿，可改为非阻塞尝试并回退到上一次的状态。
