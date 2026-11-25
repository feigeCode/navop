# Navop 集成 tmux 设计方案（修订版）

> 文档状态：P0–P1.5 及全部后续项已实现
>
> 目标读者：维护 `crates/terminal`（终端后端）与 `crates/terminal_view`（终端视图、分屏、侧栏、Public MCP）的开发 Agent
>
> 编写日期：2026-11-23
>
> 关联文档：`docs/design/ssh-runtime-shell-integration.md`（Shell Integration 运行时注入）、`docs/design/agent-live-resource-context.md`（AI 实时资源作用域）、`docs/superpowers/specs/2026-07-15-terminal-native-split-design.md`（终端原生分屏）

---

## 0. 结论先行

Navop 已按本方案完成 P0–P1.5 的 tmux 集成，并补齐 pane 树、自定义 socket、设置动态门控与自动重连。实现采用「交互 PTY 渲染整块 tmux 画面 + 独立控制通道读取结构化状态」的路线：

| 阶段 | 内容 | 是否首期 |
| --- | --- | --- |
| **P0** | 修复 tmux pane 内 Shell Integration 守卫，并保证 SSH 运行时注入兼容 | 是 |
| **P1.1** | 识别当前 PTY 是否成为 tmux client，精确 attach 对应 socket/session | 是 |
| **P1.2** | 控制模式状态同步、active pane cwd、侧栏跟随 | 是 |
| **P1.3** | AI 作用域与 Public MCP pane 工具 | 是 |
| **P1.5** | Navop 分屏按钮代理到 tmux | 已完成，独立迭代 |
| **P2** | Navop 自行渲染每个 tmux pane | 独立立项 |

本方案不推翻原设计，但必须先解决四个问题，否则不适合进入实现：

1. **不能把「机器上安装了 tmux」当成「当前终端在 tmux 内」**。错误 attach 到无关 session 比不支持更危险。
2. **控制通道默认会收到全部 pane 输出，且 `refresh-client -C` 会主动参与窗口尺寸计算**。P1 必须使用 `no-output`，不能发送 `-C`。
3. **P0 守卫的新多行结构会破坏现有 SSH 运行时注入**，因为注入链路当前硬编码跳过脚本前 3 行。
4. **只靠 `%window-add` 和 `list-panes` 不能建出可靠状态树**；必须同时维护 window、active window、active pane，并对结构事件做合并后的全量 reconcile。

首期完成标准是：用户在 Navop 里进入 tmux 后，Navop 能可靠知道「哪一个 socket、哪一个 session、哪一个 active window/pane」，侧栏 cwd 和 AI/MCP 作用域跟随，且不会改变用户 tmux 客户端的窗口尺寸、不会重复接收整屏输出。

---

## 1. 背景与问题

### 1.1 现状

- 终端后端 `crates/terminal`：`TerminalBackend` trait、alacritty 端末模拟、`OscStreamParser`、`ExecSupervisor`。SSH 走 `ssh_backend.rs`，本地走 `pty_backend.rs`。
- 终端视图 `crates/terminal_view`：`TerminalView` 持一个 `Entity<Terminal>`；`TerminalWorkspace` 用 `TerminalSplitNode` 管理 Navop 自己的分屏。
- Shell Integration：连接时通过 PTY 注入 `shell_integration.sh`，使用 OSC 133、OSC 7 和 OSC 1337。
- Public MCP：`terminal.read`、`terminal.exec`、`terminal.control`、`terminal.write_command`、`terminal.write_keys` 按 session 注册。
- 侧栏 AI：每轮发送前把终端现场投影成资源作用域，cwd 作用域键与 `connections.list_sessions` 共用。

### 1.2 用户在 Navop 里敲 `tmux` 后会发生什么

| 能力 | tmux 内的表现 | 根因 |
| --- | --- | --- |
| `terminal.exec` | 可能一直 `Busy` / `ReadinessUnknown` | pane shell 的 OSC 133 被 tmux 消费，不到达 Navop |
| `terminal.control(interrupt)` | 可能 `NotRunning` / `ReadinessUnknown` | 同上 |
| 侧栏文件管理器跟随 `cd` | 不跟随 | `current_working_dir()` 依赖 OSC 7，而 pane 内 OSC 7 不到达 Navop |
| AI cwd 作用域 | 停在进入 tmux 前的目录 | 同上 |
| `ssh.exec` 省略 cwd | 可能使用陈旧目录 | `SESSION_CWD_SCOPE` 仍来自旧 OSC 7 |
| Navop 分屏 | 新开独立连接，与 tmux 分屏互不感知 | 两套模型 |
| Ctrl+B | 正常（终端聚焦时透传 C0 `0x02`） | 需覆盖 app 级侧栏切换快捷键，见 §5.6 |

### 1.3 两个根因

**根因 A：Shell Integration 守卫误伤 tmux pane shell。**

当前 `crates/terminal/src/shell_integration.sh:1-4` 使用进程环境变量做幂等：

```sh
[[ $- != *i* ]] && return
[[ -n "${_ONETCLI_SHELL_INTEGRATED:-}" ]] && return
export _ONETCLI_SHELL_INTEGRATED=1
```

tmux server 从已集成的 shell 启动后，新 pane shell 继承 `_ONETCLI_SHELL_INTEGRATED=1`，导致它直接 return，不安装任何 hook。

**根因 B：tmux 消费 pane 内的 OSC 133/7，而不是直接透传给外层终端。**

tmux 将 pane 内 OSC 133 用于自己的命令状态，OSC 7 用于更新 pane path。`allow-passthrough` 只对应用程序显式包裹的 DCS `\ePtmux;...\e\\` 生效，不能解决 shell hook 直接发送的 OSC。

因此：**不能通过交互 PTY 读取 tmux pane 状态，必须建立带外控制通道。**

### 1.4 本次修订的设计原则

- 交互画面仍由现有 alacritty 渲染，tmux 控制通道只读结构化状态并执行显式 tmux 命令。
- 不修改用户 `.tmux.conf`，不向远端写文件，不自动创建 tmux session。
- 所有自动探测和带外通道受 `disable_shell_integration` 与用户设置共同门控。
- 任何状态同步失败都不能阻断交互终端；控制通道失败只关闭 tmux 集成。
- tmux 的任意 pane 树不映射到 `TerminalSplitNode`。

---

## 2. 目标与非目标

### 2.1 首期目标

1. tmux pane 内的 shell 正常安装 Shell Integration。
2. 能识别当前交互 PTY 对应的 tmux client，并精确 attach 到同一个 tmux socket/session。
3. 获取 session、window、pane 的 ID、标题、cwd、当前命令和 copy-mode 状态。
4. 侧栏文件管理器、AI cwd 作用域、`ssh.exec` 默认 cwd 跟随 tmux active pane。
5. Public MCP 新增 pane 列表和 pane 输入工具，且写入工具要求显式 target。
6. 控制通道不接收重复的整屏输出，不参与 tmux 窗口尺寸计算。

### 2.2 非目标

- 不做 Navop 自渲染 tmux pane（P2）。
- 不把 tmux pane 树映射成 `TerminalSplitNode` 二叉树。
- 不修改 `.tmux.conf`，不写远端文件，不自动 `tmux new -s navop`。
- 不支持 GNU screen。
- 自定义 tmux socket 通过显式设置处理，不自动猜测。
- 控制通道断开后自动重连；交互客户端已退出时不自动创建 tmux session。

---

## 3. 架构总览

```text
交互 PTY（现有）
  用户 shell / tmux client 画面
        │
        ▼
alacritty / OscStreamParser / ExecSupervisor
        │
        │ 仅处理非 tmux 路径的 OSC 133/7
        ▼
Terminal 模型
        │
        ├─ 普通终端：现有行为不变
        └─ tmux 已识别：TmuxSessionState
                │
                ▼
       视图层：侧栏 cwd、AI 作用域、MCP 工具

带外控制通道（新增）
  exec channel / 本地子进程
        │
        ▼
tmux -C attach-session（默认 no-output）
        │
        ▼
TmuxController actor
        │
        ├─ %begin/%end 命令回复
        ├─ %window-* / %session-* / %pane-* 通知
        └─ refresh-client -B subscriptions（tmux >= 3.2）
```

### 3.1 为什么使用控制模式

| 候选 | 结论 |
| --- | --- |
| OSC passthrough | 不采用。tmux 消费 shell 的 OSC 133/7，DCS passthrough 不是默认且需要用户配置 |
| `set-hook` 修改 tmux 配置 | 不采用。违反零写入原则，且不同 tmux 版本行为差异大 |
| 仅轮询 `list-panes` | 不作为唯一路径；控制通道每秒 reconcile，tmux >= 3.2 同时使用 format subscription |
| `tmux -C` 控制模式 | 采用。纯文本协议，适合 SSH exec channel 和本地子进程 |

### 3.2 激活状态机

必须把「tmux 能力探测」和「当前 PTY 是否在 tmux 中」分开。建议使用以下状态机：

```text
Disabled
  -> WaitingForShellReady
  -> WaitingForTmuxClient
  -> Attaching
  -> Attached
  -> Detached / Failed
```

触发规则：

- `WaitingForShellReady`：Shell Integration 已就绪后，记录交互 PTY 的 tty，并等待用户执行命令。
- `WaitingForTmuxClient`：在 shell 命令开始后做有界、去抖的 tmux client 探测；不要持续高频轮询。
- `Attaching`：找到当前 tty 对应的 client/session 后，打开控制通道。
- `Attached`：控制通道完成初始 snapshot，注册 MCP 能力。
- `Detached`：控制通道退出，清除 tmux state，保留交互 PTY。
- `Failed`：没有 tmux、无权限、无法确定 session 等；记录 debug，不阻断终端。

---

## 4. P0：Shell Integration 守卫与注入兼容

### 4.1 守卫目标

守卫必须同时满足：

1. 同一 shell 内重复 source 不重复追加 hook；
2. tmux pane 内的新 shell 能安装 hook；
3. 同一 pane 内执行 `exec bash`、启动新 shell、`su -` 后，函数丢失时能重新安装；
4. 不能破坏 `ssh_shell_integration.rs` 的运行时注入。

建议守卫改为「pane marker 匹配且当前 shell 函数确实存在」才跳过：

```sh
[[ $- != *i* ]] && return

_onetcli_guard_key="${TMUX_PANE:-}"
_onetcli_hook_present=0
if typeset -f __onetcli_precmd_common >/dev/null 2>&1; then
    _onetcli_hook_present=1
fi

if [[ "${_ONETCLI_SHELL_INTEGRATED:-}" == "$_onetcli_guard_key" &&
      "$_onetcli_hook_present" == "1" ]]; then
    return
fi

export _ONETCLI_SHELL_INTEGRATED="$_onetcli_guard_key"
unset _onetcli_guard_key _onetcli_hook_present
```

说明：

- 非 tmux 内 `TMUX_PANE` 为空，pane marker 为空；函数不存在时仍会安装，解决子 shell 函数丢失问题。
- tmux 内不同 pane 的 `TMUX_PANE` 不同，可正常安装。
- 同一 pane 中 `exec bash` 后，虽然 marker 仍是同一个 pane，但函数不存在，因此重新安装。
- 不要只依赖 `_ONETCLI_SHELL_INTEGRATED` 或 `TMUX_PANE` 单一条件。

### 4.2 运行时注入必须改成结构化提取

当前 `crates/terminal/src/ssh_shell_integration.rs:57-60` 使用：

```rust
let body = minified_shell_integration_body(
    &script.lines().skip(3).collect::<Vec<_>>().join("\n"),
);
```

守卫一旦扩展为多行，`skip(3)` 会把部分守卫注入到 eval body，`return` 可能导致整个运行时注入失败。

建议在 `shell_integration.sh` 中加入明确分隔符：

```sh
# __ONETCLI_BODY_START__
__onetcli_emit_osc() {
    ...
}
```

运行时注入按分隔符提取，不再按固定行数跳过：

```rust
fn runtime_shell_integration_body() -> String {
    let script = crate::shell_integration::embedded_shell_integration_script();
    let body = script
        .split_once("# __ONETCLI_BODY_START__")
        .map(|(_, body)| body)
        .expect("shell integration body marker");
    minified_shell_integration_body(body)
}
```

同时补充测试，断言注入命令不包含守卫、`return` 或 `_ONETCLI_SHELL_INTEGRATED` 设置逻辑。

### 4.3 P0 测试矩阵

必须覆盖 bash 和 zsh：

1. 父 shell 已集成，进入 tmux pane 后安装 hook；
2. 同一 pane 再次 source 不重复追加；
3. 同一 pane 内 `exec bash` 或启动新 shell 后重新安装；
4. 不同 tmux pane 会安装；
5. SSH 运行时注入 body 不包含守卫；
6. 现有 `PROMPT_COMMAND` / `precmd_functions` 和 prompt trailer 测试保持通过。

---

## 5. P1.1：识别当前 tmux client 与控制通道

### 5.1 不能把 `tmux -V` 当作激活条件

`tmux -V` 只能说明 tmux 已安装，不能说明当前交互 PTY 在 tmux 内。以下行为必须禁止：

- 探测到 `tmux -V` 后直接 `tmux -CC attach`；
- 不带 `-t` attach 到「最近的 session」；
- attach 失败后自动 `tmux new -s navop`；
- 把 `$TMUX` 的 socket 环境值当作 session 名称传给 `-t`。

### 5.2 记录交互 PTY 的 tty

Shell Integration 就绪后，通过现有注入链路执行一次性私有 OSC：

```sh
printf '\033]1337;NavopTty=%s\007' "$(tty)"
```

在 `osc.rs` 增加 `NavopTty` 事件。该事件必须在用户进入 tmux **之前**采集，因为进入 tmux 后 pane 内的 OSC 会被 tmux 消费。

该一次性命令必须复用现有 runtime setup 的 history 抑制机制，不能把 `NavopTty` 注入命令留在用户 shell history 中。

本地终端可额外使用 PTY 子进程信息做兜底，但 SSH 与本地建议共用 tty 识别模型。

### 5.3 识别 tmux client

用户执行命令后，利用 shell integration 的 `CommandStart` 事件调度一次有界探测，而不是持续高频轮询。例如在命令开始后延迟 250ms、750ms、1500ms 检查：

```sh
tmux list-clients -F '#{q:client_tty}|#{q:client_session}'
```

匹配规则：

1. `client_tty` 等于当前交互 PTY 的 tty；
2. 匹配到唯一 `client_session`；
3. 若多个结果匹配同一 tty，视为歧义，不自动 attach；
4. 若没有匹配，回到等待态，下一次 shell 命令再尝试；
5. 用户退出 tmux 后，`list-clients` 不再匹配，控制通道会被清理。

默认 socket 场景可直接使用上述命令。自定义 socket（例如 `tmux -L custom`）无法从默认 `list-clients` 推断，首期应：

- 显示「未找到 tmux session」而不是猜测；
- 提供手动输入 socket/session 的入口；或
- 在设置中提供自定义 tmux socket 配置。

**不允许自动创建 session。**

### 5.4 控制客户端启动

匹配到 session 后，用精确 socket 和 session ID 启动控制客户端：

```text
tmux [-S <socket>] -C attach-session -t <session-id>
```

要求：

- SSH exec channel 没有交互 shell，启动命令本身由 `exec` 字符串执行，必须对 socket/session 做 shell 单引号转义；
- 使用单个 `-C`，不要默认使用 `-CC`；
- 如果未来必须用 `-CC`，协议层必须显式吞掉 DCS `ESC P1000p` 和退出时的 ST，而不是把首行交给普通行解析器；
- 控制通道成功 attach 后立即发送：

```text
refresh-client -f no-output
```

- **不要发送 `refresh-client -C <cols>x<rows>`**。tmux 控制客户端在不使用 `-C` 时本身不影响窗口尺寸；发送 `-C` 后会参与 `window-size=latest`，正好引入本方案要避免的问题。

### 5.5 控制通道生命周期

建议拆分所有权：

- `TmuxControllerActor`：持有 transport、命令发送端、pending 回复表和 actor JoinHandle；
- `TmuxControllerHandle`：只持有可 clone 的 command sender，交给 Terminal / Public MCP；
- `Terminal`：拥有 actor 生命周期；
- `TerminalView` / Public MCP：只克隆 handle，不拥有 transport。

断开时必须：

1. 完成所有 pending 命令为明确错误；
2. 发送 `TmuxDetached`；
3. 清理 Public MCP tmux handle；
4. 不关闭交互 PTY；
5. 允许用户手动重新探测。

### 5.6 终端快捷键与 tmux prefix

tmux 默认 prefix 是 `Ctrl+B`，而 Navop 连接侧栏也默认使用 `Ctrl+B`。GPUI 的 app 级快捷键会先于终端 `on_key_down` 消费该按键，表现为输入 `Ctrl+B %` 时只有 `%` 到达 shell，tmux 无法分屏。

首期采用终端上下文覆盖：

- 在 `TerminalView` 上下文注册 `Ctrl+B` action，将原始 C0 字节 `0x02` 写入 PTY；
- 终端聚焦时优先保证 tmux 默认键位可用；侧栏仍可点击切换，离开终端上下文后 `Ctrl+B` 继续作为 app 快捷键；
- 该行为不能只在 tmux 状态下注册，否则检测状态与首帧输入之间存在竞态；
- 回归测试同时约束 action 注册、初始/刷新 keybinding 和 `0x02` 写入。

---

## 6. P1.2：协议、状态同步与 cwd

### 6.1 控制模式协议

控制模式输入输出以行为单位，但 `%output` 可能包含非 UTF-8 字节；transport 的读取接口应返回字节，而不是直接返回 `String`。

建议协议类型：

```rust
pub enum TmuxControlLine {
    Begin { epoch: u64, cmdno: u64, flags: String },
    End { epoch: u64, cmdno: u64, flags: String },
    Error { epoch: u64, cmdno: u64, flags: String, output: String },
    Output { pane: String, data: Vec<u8> },
    WindowAdded { id: String },
    WindowClosed { id: String },
    WindowRenamed { id: String, name: String },
    WindowPaneChanged { window: String, pane: String },
    LayoutChanged {
        window: String,
        layout: String,
        visible_layout: String,
        flags: String,
    },
    PaneModeChanged { pane: String },
    SessionChanged { id: String, name: String },
    SessionRenamed { id: Option<String>, name: String },
    SessionWindowChanged { session: String, window: String },
    SessionsChanged,
    SubscriptionChanged {
        name: String,
        session: String,
        window: String,
        window_index: String,
        pane: Option<String>,
        value: String,
    },
    Exit { reason: Option<String> },
    Other(String),
}
```

实现要求：

- `%begin/%end/%error` 必须保存相同的 `epoch/cmdno/flags`，pending 请求按 `cmdno` 匹配；
- `%subscription-changed` 的实际格式包含 session/window/pane 上下文和单独的 `:` value 分隔符，不能只解析 `{name, value}`；
- 不同 tmux 版本的 `%session-renamed` 参数形态存在差异，协议测试必须使用 3.2 和 3.8 的真实样本；
- `%extended-output`、未知 `%xxx` 行、空行、CRLF 和 DCS 前缀都必须可安全忽略；
- 既然 P1 使用 `no-output`，`%output` 只作为调试/防御性解析，不作为主状态来源。

### 6.2 初始全量快照

不能只执行 `list-panes`。初始状态至少需要：

```text
list-windows -t "$session_id" -F '#{q:session_id}|#{q:window_id}|#{q:window_index}|#{q:window_name}|#{window_active}'
list-panes -s -t "$session_id" -F '#{q:session_id}|#{q:window_id}|#{q:window_index}|#{q:pane_id}|#{pane_width}|#{pane_height}|#{window_active}|#{pane_active}|#{q:pane_current_path}|#{q:pane_current_command}|#{pane_in_mode}|#{q:pane_title}|#{pane_pid}'
```

tmux >= 3.8 可以额外读取：

```text
pane_command_running
pane_command_start_time
pane_command_status
```

低版本使用 `pane_current_command` 作为近似值，不要声称能获得精确命令生命周期。

格式字符串中的所有字符串字段都必须使用明确的 tmux quoting/unescaping，不能直接按 `|` 切分 `pane_current_path` 或 `pane_title`；Unix 路径和标题都可能包含分隔符。建议统一使用 `#{q:...}` 并实现对应的格式反转义。

### 6.3 状态模型

```rust
pub struct TmuxWindow {
    pub id: String,
    pub index: String,
    pub name: String,
    pub active: bool,
}

pub struct TmuxPane {
    pub id: String,
    pub window_id: String,
    pub width: u16,
    pub height: u16,
    pub window_active: bool,
    pub active_in_window: bool,
    pub cwd: Option<String>,
    pub command: String,
    pub command_running: Option<bool>,
    pub command_status: Option<i32>,
    pub in_mode: bool,
    pub title: String,
    pub pid: u32,
}

pub struct TmuxSessionState {
    pub session_id: String,
    pub session_name: String,
    pub version: Option<TmuxVersion>,
    pub windows: Vec<TmuxWindow>,
    pub panes: Vec<TmuxPane>,
    pub attached: bool,
}
```

必须明确 active 判定：

```rust
fn active_window(&self) -> Option<&TmuxWindow>;
fn active_pane(&self) -> Option<&TmuxPane> {
    let window = self.active_window()?;
    self.panes
        .iter()
        .find(|pane| pane.window_id == window.id && pane.active_in_window)
}
```

`pane_active` 只表示 pane 在其所属 window 内 active，一个 session 可能有多个 `pane_active=true`。不能用「第一个 active pane」推断全局 active pane。

### 6.4 增量同步策略

结构变化采用「标记 dirty + 去抖后全量 reconcile」：

- `%window-add`
- `%window-close`
- `%window-renamed`
- `%layout-change`
- `%session-window-changed`
- `%window-pane-changed`
- `%pane-mode-changed`
- `%subscription-changed`

建议合并 50–100ms 内的多个通知，然后重新执行 `list-windows + list-panes`。这比尝试从 `%window-add` 单行推出 window name、pane 列表和 layout 更可靠。

`list-panes -s` **必须带 `-t <session_id>`**。不带 `-t` 时 tmux 可能按服务器最近活动 session 解析目标，而不是控制 client 当前附着的 session；同一台机器有多个 session 时会把其他 session 的 pane 混入当前 Navop 终端状态。window、pane、session name 查询都应使用 discovery 得到的 session id 作为显式 target。

`%pane-mode-changed` 只告诉发生了 mode 变化，不知道进入还是退出；必须重新查询 `pane_in_mode`。

### 6.5 tmux 版本与订阅

- tmux >= 3.2 支持 `refresh-client -B` 和 `%subscription-changed`；不要把它错误地限制为 3.8+。
- 首期最低支持 tmux 3.2；更旧版本不自动 attach，避免在旧协议上产生错误状态。
- 控制 actor 每秒执行一次 `list-windows + list-panes` reconcile；3.2+ 的 subscription 用于补充事件触发。
- tmux >= 3.8 使用 `pane_command_running` / `pane_command_status` 等格式变量补充命令状态。
- 版本比较按 `(major, minor)`，不要使用 `major >= 3 && minor >= 8`，否则 `4.0` 会被误判。
- `3.3a` 这类版本要定义 suffix 的保存方式；只保留 major/minor 也必须在测试中明确。

### 6.6 Terminal 模型接入

`TerminalEvent` 可新增：

```rust
TmuxAttached(TmuxSessionState),
TmuxChanged(TmuxSessionState),
TmuxDetached { reason: String },
```

`TerminalModelEvent` 建议统一为：

```rust
TmuxChanged(Option<TmuxSessionState>),
```

`Terminal` 保存：

```rust
tmux: Option<TmuxSessionState>,
tmux_controller: Option<TmuxControllerHandle>,
```

`current_working_dir()` 只有在 tmux attached 且 active pane 路径存在时才优先返回 tmux path，否则回退 OSC 7。

新增：

```rust
pub fn tmux_reported_working_dir(&self) -> Option<ReportedWorkingDir>;
```

该方法必须从 `tmux.active_pane().cwd` 构造路径，并明确 host/provenance；**不能像原方案那样在 `TmuxChanged` 分支里读取旧的 `reported_working_dir()`**，否则侧栏仍会使用进入 tmux 前的路径。

host 规则：

- 本地终端：`host=None`；
- SSH：只在能确认 tmux server 与当前连接目标同机时复用当前连接的上报 host；
- 多跳或 host 不确定时保留 `None` 或标记 unknown，不要让旧 OSC 7 的内层 host 继续代表 tmux 路径；
- 文件管理器跟随失败必须是无害的，不能把错误路径强行套到 SFTP。

---

## 7. 侧栏、AI 与 Public MCP

### 7.1 侧栏 cwd 跟随

`crates/terminal_view/src/view/terminal_events.rs` 的 `TmuxChanged` 分支必须：

1. 调用 `terminal.tmux_reported_working_dir()`；
2. 仅在 attached 且 active pane cwd 可用时同步；
3. 复用现有 `sync_workspace_explorer_path` / `set_file_manager_initial_dir` / `sync_file_manager_path`；
4. 在 detached 时停止跟随，不要清空用户当前浏览目录。

### 7.2 AI 作用域

新增 `SCOPE_TMUX`，摘要至少包含：

```text
session "main" ($0); active window 1:@3 "editor"; active pane %4
cwd=/srv/app; command=running:false; status=0; mode=normal
other panes: %5 bash /var/log; %6 vim /srv/api
```

3.8+ 使用真实命令状态；旧版本只报告 `pane_current_command` 并明确标为近似值。

系统提示必须告诉模型：

- `cwd` 是 active pane 的目录；
- 操作其他 pane 使用 `terminal_tmux_list_panes` / `terminal_tmux_send_keys`；
- copy-mode 下按键会被 tmux 消费；
- `terminal.exec` 在 tmux pane 内不可靠不是错误配置。

### 7.3 Public MCP Handle

建议不要把同步方法和异步 actor 混在一起：

```rust
pub trait TerminalTmuxSessionHandle: Send + Sync + 'static {
    fn snapshot(&self) -> TerminalSessionSnapshot;
    fn list_panes(&self) -> Result<Vec<TmuxPaneInfo>>; // 读取缓存
    fn send_keys(
        &self,
        request: TmuxSendKeysRequest,
        cancellation: TerminalTmuxCancellation,
    ) -> TerminalTmuxFuture;
}
```

实现要求：

- `list_panes` 不直接做网络 round-trip，读取 `TmuxSessionState` 缓存；
- `send_keys` 通过 actor 执行，支持 timeout、取消和断线失败；
- actor 断开时所有 pending future 返回明确错误；
- `TmuxControllerHandle` 可 clone，`TmuxControllerActor` 不放进 Public MCP handle 的 `Option` 字段。

建议工具：

- `terminal.tmux_list_panes`：`read_only=true`、低风险、target 可省略；
- `terminal.tmux_send_keys`：高风险、target 必填、不得加入 `DEFAULT_TARGET_TOOLS`。

建议的公开数据类型：

```rust
pub struct TmuxPaneInfo {
    pub pane_id: String,
    pub window_id: String,
    pub window_name: String,
    pub active: bool,
    pub cwd: Option<String>,
    pub command: String,
    pub in_mode: bool,
    pub width: u16,
    pub height: u16,
}

pub struct TmuxSendKeysRequest {
    pub pane_id: String,
    pub keys: String,
    pub submit: bool,
}

pub struct TmuxSendKeysResult {
    pub pane_id: String,
    pub queued: bool,
}
```

### 7.4 命令序列化

控制模式 stdin 仍然是 tmux 命令语言，不是 `execve(argv)`。即使 Rust 侧使用 `Vec<String>`，最终也必须安全序列化成一行 tmux 命令。

因此：

- 为 tmux 命令实现专用 serializer，不能直接 `args.join(" ")`；
- `send-keys -l --` 的文本参数必须正确转义空格、分号、单引号、双引号和反斜杠；
- `keys` 只接受不含换行和控制字节的 UTF-8 文本；`submit=true` 时再发送独立的 `Enter`；
- 每次 `Run` 都要有 timeout；
- 测试必须覆盖特殊字符、Unicode、长文本和断线时的 pending 命令。

---

## 8. P1.5：分屏代理

已实现的行为：

1. `TerminalWorkspace::split_pane(target, placement)` 中 target 是 Navop pane；代理前从该 pane 的 tmux state 读取 active tmux pane id，并始终用 `-t <pane_id>` 显式指定目标。
2. Navop 分屏方向映射到 tmux：
   - Left：`split-window -h -b -t %N`
   - Right：`split-window -h -t %N`
   - Top：`split-window -v -b -t %N`
   - Bottom：`split-window -v -t %N`
3. tmux attached 时不再创建新的 Navop `TerminalSplitNode` pane；用户看到的仍是同一个 Navop pane，tmux client 自己在其中渲染分屏。
4. 未进入 tmux 或 tmux 集成不可用时，继续执行原有 Navop 分屏逻辑。
5. 分屏命令异步失败时通过 toast 提示，不把失败伪装成 Navop 布局变化。
6. 「取消分屏」继续禁用；tmux 的 `break-pane` 是拆成新 window，不等价于 Navop 的独立 Tab。
7. tmux-only 分屏时 Navop pane 数仍为 1，因此不会显示 Navop 的 cancel-split 按钮。

旁路 pane 树已实现以下语义：

1. 点击非 active pane 行时，先 `select-window -t @W`，再 `select-pane -t %N`。
2. 「最后一个 pane」必须区分：
   - 当前 tmux window 的最后一个 pane；
   - 当前 session 的最后一个 window；
   - Navop Tab 的最后一个 pane。
3. `kill-pane` 可能导致 tmux 自动关闭 window/session；不能无条件把它映射成 Navop 的 Tab 关闭。

---

## 9. 设置与门控

设置：

```rust
pub tmux_integration: bool;
pub tmux_socket: String;
```

实现：

- 当前实现通过 `AppSettings::terminal_tmux_integration` 控制，默认 `true`；用户可以显式关闭。
- `AppSettings::terminal_tmux_socket` 保存自定义 socket 路径；空值表示 tmux 默认 socket。
- 设置持久化在 `AppSettings`；`TerminalSettings` 只是派生快照。
- `tmux_integration && !disable_shell_integration` 才允许探测和控制通道。
- 终端设置面板可即时切换开关和 socket；变化会关闭旧 controller、注销 MCP、清理 state，并按新配置重新探测。
- SSH 连接表单继续复用现有 `disable_shell_integration`，不新增重复的安全开关。

---

## 10. i18n

至少新增：

```yaml
TerminalWorkspace:
  tmux_pane_tree:
    en: tmux Panes
    zh-CN: tmux 窗格
    zh-HK: tmux 窗格
  tmux_detached:
    en: tmux control channel disconnected
    zh-CN: tmux 控制通道已断开
    zh-HK: tmux 控制通道已斷開
  tmux_session_ambiguous:
    en: Multiple tmux sessions matched; choose one explicitly
    zh-CN: 匹配到多个 tmux 会话，请显式选择
    zh-HK: 匹配到多個 tmux 會話，請明確選擇
  tmux_session_unsupported_socket:
    en: This tmux socket requires explicit configuration
    zh-CN: 此 tmux socket 需要显式配置
    zh-HK: 此 tmux socket 需要明確配置
  tmux_cancel_split_unsupported:
    en: Not supported inside tmux
    zh-CN: tmux 内不支持
    zh-HK: tmux 內不支持
  tmux_split_failed:
    en: "Failed to split tmux pane: %{error}"
    zh-CN: "tmux 分屏失败：%{error}"
    zh-HK: "tmux 分屏失敗：%{error}"
  tmux_select_failed:
    en: "Failed to switch tmux pane: %{error}"
    zh-CN: "切换 tmux 窗格失败：%{error}"
    zh-HK: "切換 tmux 窗格失敗：%{error}"
```

---

## 11. 跨模块影响清单

| 模块 | 文件 | 改动 |
| --- | --- | --- |
| terminal | `shell_integration.sh` | pane + hook 存在性守卫；body marker |
| terminal | `ssh_shell_integration.rs` | 按 marker 提取注入 body；禁止 skip(3) |
| terminal | `osc.rs` | `NavopTty`；只解析 tmux 外的 OSC |
| terminal | `tmux/`（新） | version、protocol、control、state、transport |
| terminal | `pty_backend.rs` | 本地 tty 采集、探测状态机、本地 tmux transport |
| terminal | `ssh_backend.rs` | tty 采集、`list-clients`、SSH tmux transport |
| terminal | `terminal.rs` | tmux state、controller handle、`tmux_reported_working_dir` |
| terminal_view | `settings.rs` / `session_settings.rs` | 持久化 tmux 开关与门控 |
| terminal_view | `view/terminal_events.rs` | `TmuxChanged` 使用 tmux cwd，而不是旧 OSC 7 |
| terminal_view | `sidebar/mod.rs` | pane 树面板生命周期与 window/pane 选择事件 |
| terminal_view | `sidebar/tmux_panes_panel.rs` | window / pane 树、active 状态、点击切换 target |
| terminal_view | `ai_live_context/mod.rs` | tmux 作用域与 3.8 命令状态 |
| terminal_view | `public_mcp.rs` | `TmuxControllerHandle`、注册/注销 |
| public_mcp | `registry.rs` / `tools/` | list/send_keys、Tmux capability、异步 send_keys |
| tool_runtime | `resource.rs` | `ResourceCapability::Tmux` |
| terminal_view | `workspace/actions.rs` | tmux attached 时先代理 `split-window`，否则回退 Navop 分屏 |

---

## 12. 实施顺序

### Step 1：P0 Shell Integration

- 修改守卫；
- 引入 body marker；
- 修运行时注入提取；
- bash/zsh/嵌套 shell/注入测试。

验收：不进入 tmux、只进入 tmux、同一 pane 重开 shell、SSH 运行时注入均通过。

### Step 2：Tmux discovery

- 采集交互 PTY tty；
- `CommandStart` 去抖探测 `list-clients`；
- 精确 socket/session attach；
- 歧义、无 session、自定义 socket 的错误路径；
- 禁止自动创建 session。

验收：不连接无关 session；进入 tmux 后能精确 attach；退出 tmux 后 controller 清理。

### Step 3：控制协议和状态快照

- 字节级 transport；
- `no-output`；
- `list-windows + list-panes` 全量 snapshot；
- 3.2 subscription + 每秒 reconcile；
- 结构事件 reconcile、active window/pane；
- 版本兼容测试。

验收：任意 window/pane 结构变化后 1.5s 内状态一致，active cwd 正确。

### Step 4：cwd、AI 和 MCP

- `tmux_reported_working_dir`；
- 侧栏跟随；
- AI tmux scope；
- `terminal.tmux_list_panes`；
- 异步 `terminal.tmux_send_keys`；
- 断线/pending/timeout 测试。

验收：文件管理器、AI cwd、MCP 三处看到的是同一个 active pane 路径，非 active pane 写入不影响当前焦点。

### Step 5：P1.5 分屏代理

- 已完成 `TmuxSplitPlacement` 与 `split-window` 命令序列化；
- `TerminalControllerHandle::split_window` 异步执行并支持超时/断线错误；
- workspace 分屏先尝试 tmux，未 attach 时回退原 Navop 分屏；
- VNC 真机已验证向右、向左分屏。

### Step 6：后续完整项

- `TmuxPanesPanel`：侧栏 window / pane 树，attach 后自动出现，detach 后自动关闭。
- 自定义 socket：discovery、version、attach、local transport、SSH exec 全链路使用同一个 `-S` 路径。
- 动态门控：设置开关/socket 改变会递增 tmux config generation，关闭旧 controller 并重新探测。
- 自动重连：控制通道意外断开后按 0.5s→30s 有界指数退避重连；config generation 阻止旧探测回写。

---

## 13. 验证计划

### 13.1 单元与契约

- `protocol`：3.2/3.8 真实控制模式样本、`%begin/%end/%error`、DCS、CRLF、非 UTF-8 `%output`；
- `state`：`list-windows + list-panes`、多 window active pane、`%session-window-changed`、`%pane-mode-changed`、`%subscription-changed`；
- `control`：Fake Transport、pending 命令、timeout、disconnect、shutdown；
- `shell_integration`：bash/zsh、`exec bash`、运行时注入 body 不含守卫；
- `terminal_view`：`TmuxChanged` 不再读取旧 OSC 7，而是使用 `tmux_reported_working_dir`；
- `public_mcp`：命令转义、特殊字符、断线错误、`send_keys` 不在默认 target 工具中。

### 13.2 真实 tmux

至少使用一台 tmux 3.2 和一台 tmux 3.8 环境；推荐在 Linux 容器中把这两条版本纳入 CI。

| 场景 | 断言 |
| --- | --- |
| 连接后未启动 tmux | 不自动创建 session，不出现错误提示 |
| 用户执行 `tmux new -s navop` | 精确 attach，侧栏出现 pane 树 |
| 机器已有其他 session | 不 attach 到其他 session，tty 匹配正确 |
| 多个 tmux session | 不自动猜测；歧义提示或显式选择 |
| tmux 自定义 socket | 设置路径后 discovery 与控制 client 都使用该 socket |
| pane 内 `cd /tmp` | active cwd 更新，文件管理器跟随 |
| Ctrl+B `%` 分屏 | tmux 内 pane 数增加，侧栏 pane 树同步更新 |
| pane 树点击 window / pane | `select-window` / `select-pane` 落到正确 target |
| 关闭 tmux 集成 | 控制 client 消失、交互 PTY 保留、pane 树关闭、MCP tmux 注销 |
| 重新开启 tmux 集成 | controller 重建、pane 树恢复 |
| 只断开控制 client | 交互 client 保持，Navop 自动重连控制通道 |
| Navop 分屏按钮（tmux attached） | 不新增 Navop pane；tmux pane 数增加，方向和 active pane 正确 |
| Navop 分屏按钮（tmux 外） | 保持原 Navop 分屏行为 |
| 非 active pane 写入 | 目标 pane 收到，active pane 不变 |
| 同服务器多 session | 只返回当前 client 所属 session 的 window/pane，不串入其他 session |
| `exec bash` 后继续使用 | shell integration 恢复，不再丢 hook |
| tmux 退出再启动 | controller 清理并可重新 attach |
| `kill-server` | 控制通道 Detached，交互 PTY 不受影响 |
| 控制通道断线时执行 MCP | pending 命令返回明确错误，不永久挂起 |
| tmux 外普通终端 | shell integration、cwd、exec、分屏行为不变 |

### 13.3 回归命令

```sh
cargo test -p terminal -p public_mcp -p terminal_view
cargo clippy -p terminal -p public_mcp -p terminal_view -- --deny warnings
cargo fmt --check
cargo machete
```

---

## 14. 风险与开放问题

| 风险 | 影响 | 处理 |
| --- | --- | --- |
| 自定义 tmux socket | 默认 `list-clients` 看不到对应 client | 已提供显式 socket 设置并贯穿 discovery/attach |
| 多个 session 共享同一 tty 信息 | 可能无法唯一判断 | 进入歧义态，要求用户选择 |
| tmux 3.8 事件与 3.2 man page 差异 | 协议实现偏差 | 使用两个版本真实样本测试，不只用 wiki 表 |
| 控制通道默认 `%output` | 重复网络流量和 CPU | attach 后立即 `refresh-client -f no-output` |
| `refresh-client -C` 影响其他 client | 改变用户 tmux 窗口尺寸 | 首期禁止发送 `-C` |
| 自定义格式化字段含 `|` | 状态解析错位 | 使用 `#{q:...}` 和专门 unescape |
| tmux pane 内再 ssh | cwd 属于跳板机或外层机器 | 保留 host provenance，不能盲目跟随 SFTP |
| 自动重连 | 旧 controller 与新 session 状态混淆 | 使用配置 generation 丢弃旧事件，退避重连 |
| 分屏语义与 Navop 不一致 | 用户误以为在修改 Navop 布局 | P1.5 已显式 target；取消分屏仍禁用 |

**仍需产品确认的问题：**

1. tmux 集成默认关闭，还是首次检测到 tmux 后询问用户开启？
2. 多 session 歧义时，是在侧栏选择，还是在设置页配置固定 session？
3. 自定义 tmux socket 已提供全局设置；未来是否需要 connection-level 覆盖？
4. P1.5 当前跟随 `terminal_tmux_integration`；是否再提供独立的「接管 Navop 分屏按钮」开关仍需产品确认。

---

## 15. 当前实现落地状态

- P0：Shell Integration 守卫、body marker、SSH 运行时注入兼容已落地。
- P1.1：交互 PTY tty 上报、`list-clients` 精确匹配、Local/SSH 控制通道、`no-output` 已落地；自定义 socket 仍走显式配置/不支持提示。
- P1.2：控制协议、window/pane 快照、active pane cwd、每秒 reconcile、3.8 命令状态字段已落地。
- P1.3：AI tmux scope、`terminal.tmux_list_panes`、`terminal.tmux_send_keys` 和异步取消/断线错误已落地。
- P1.5：Navop 四个分屏方向已代理到 tmux active pane；tmux 外保留原 Navop 分屏回退。
- Pane 树：window/pane 层级、active 状态、点击切窗/切 pane 已落地。
- 自定义 socket：全局设置项已贯穿 discovery、version、attach 和 transport。
- 动态门控：开关/socket 变化会重建 controller 并与 Public MCP 注销/注册联动。
- 自动重连：控制通道断开后按 generation 隔离旧事件并指数退避重连。

真实环境验收补充：

- Docker SSH 测试环境已包含 tmux 3.4；2222 保留 OTP 流程，2223 提供密码认证供 `NAVOP_LIVE_SSH` 使用。
- `crates/terminal/tests/tmux_control_live.rs` 已覆盖真实 SSH + tmux attach、window/pane 快照、active cwd、tmux 分屏、向非 active pane `send_keys` 且 active pane 不变、decoy session 多 session 隔离，以及自定义 socket 全链路。
- VNC 真机已验证：tmux 内 `Ctrl+B %` 可正常分屏；点击 Navop 分屏菜单的「向右分屏」「向左分屏」后，tmux pane 数正确增加且没有新增 Navop pane；切到右 pane 后 `cd /tmp`，侧栏文件管理器跟随到 `/tmp`。
- VNC 真机已验证 pane 树自动出现，点击非 active pane 与另一个 window 后 tmux active target 正确切换。
- VNC 真机已验证关闭 tmux 开关后 control client 消失且交互 client 保留；重新开启后 control client 与 pane 树恢复。
- VNC 真机已验证手动 detach control client 后约 1.5 秒自动重连。
- VNC 真机已验证自定义 socket 路径驱动完整 attach，pane 树显示自定义 socket 上的 session。
- 同服务器多 session 隔离已修复：控制通道统一按发现到的 session id 执行 `list-windows` / `list-panes` target，避免混入其他 session 的 pane。
- 真机测试发现并修复了两个生命周期问题：attach 启动块导致命令回复错位，以及 GPUI `Task` 被丢弃导致探测任务取消。
- VNC 真机测试还发现并修复了 `Ctrl+B` 与连接侧栏快捷键冲突：终端上下文现在显式把 `Ctrl+B` 作为 `0x02` 写入 PTY，默认 tmux prefix 不再被 app 快捷键吞掉。

### 15.1 P1.5 分屏边界

- 已验证 **tmux 自身分屏**：`Ctrl+B %`、控制通道 reconcile、active pane cwd、非 active pane 定向输入。
- 已验证 **Navop 分屏按钮代理到 tmux**：四个方向序列化单测覆盖，VNC 真机覆盖 left/right，命令使用显式 tmux pane target，且不创建 Navop pane。
- 已验证 **tmux pane 独立侧栏树**：attach 自动出现、window/pane 选择、active 高亮、detach 自动关闭。

---

## 16. 参考资料

- tmux Control Mode：https://github.com/tmux/tmux/wiki/Control-Mode
- tmux 3.2 man page：https://github.com/tmux/tmux/blob/3.2/tmux.1
- tmux 3.8 man page：https://github.com/tmux/tmux/blob/3.8/tmux.1
- tmux Hooks/Events：https://github.com/tmux/tmux/wiki/Events
- 项目内相关设计：`docs/design/ssh-runtime-shell-integration.md`、`docs/design/agent-live-resource-context.md`、`docs/superpowers/specs/2026-07-15-terminal-native-split-design.md`
