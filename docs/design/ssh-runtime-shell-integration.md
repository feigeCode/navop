# SSH Shell Integration 运行时注入（无远端写入）设计

> 提交：`82e3c5a3 feat(terminal): runtime SSH shell integration without remote writes`

## 背景与动机

旧版 SSH Shell Integration 在每次连接时通过独立 exec 通道向远端写入：

- `$HOME/.config/onetcli/shell_integration.sh`（集成脚本）
- 在 `.bashrc` / `.zshrc` / `.bash_profile` / `.profile` 中插入 `# BEGIN/END ONETCLI SHELL INTEGRATION` managed block

这是"向目标系统文件写入"，带来三个问题：

1. **合规风险**：对生产服务器、客户环境写入用户未显式同意的文件，需要默认禁用 + 风险提示 + 卸载入口等一整套安全设计；
2. **残留污染**：managed block 逻辑复杂（awk 过滤、幂等替换、目录冲突恢复），出错时可能破坏用户 rc 文件；
3. **缓存复杂**：安装结果按 SSH client 生命周期缓存（`ShellIntegrationSetup`），重连/失效/过期写入需要专门防串。

参考 meatshell 的做法（运行时通过 PTY 注入，不落盘），navop 改为**每次连接时把集成脚本当作一条用户敲入的命令注入交互 shell**，会话结束一切消失，远端零写入。

## 核心方案

```
┌─ 连接建立 ─────────────────────────────────────────────┐
│ exec 通道: case $SHELL in *bash*|*zsh*) → 只读探测      │
│   ├─ 支持   → 交互通道 pty+shell，标记"请求注入"        │
│   └─ 不支持 → 交互通道 pty+shell，纯裸终端              │
└────────────────────────────────────────────────────────┘
          ↓ 登录 expect 完成，首段有效输出（prompt 出现）
┌─ 注入 ────────────────────────────────────────────────┐
│ PTY 输入: " : __ONETCLI_RUNTIME_SETUP_1; eval $'...'\r" │
│ （脚本单行化 + ANSI-C 引号；期间用户输入进暂存队列）     │
└────────────────────────────────────────────────────────┘
          ↓ 远端回显 + 脚本执行
┌─ 回显抑制 ────────────────────────────────────────────┐
│ 输出进抑制缓冲，直到私有标记 OSC 1337;ShellIntegration  │
│ Ready=1；跨 chunk 拆分可对齐，64KB 截断保护             │
└────────────────────────────────────────────────────────┘
          ↓ 标记之后的输出恢复转发
┌─ 就绪握手 ────────────────────────────────────────────┐
│ 等首个 OSC 133;B（prompt 渲染完成）→ 放行暂存的用户输入│
│ 5s 超时 → 发 Ctrl+C、丢弃缓冲、降级为裸终端            │
└────────────────────────────────────────────────────────┘
          ↓ 正常会话
OSC 133 A/B/C/D + OSC 7 + OSC 1337 协议与旧版完全一致
```

## 关键实现细节

### 1. 注入命令构造（`ssh_shell_integration.rs`）

```
" : __ONETCLI_RUNTIME_SETUP_1; eval $'<脚本体>'\r"
```

- **前导空格**：`HISTCONTROL=ignorespace`（多数发行版默认）使该行不进 shell history；
- **`eval $'...'`（ANSI-C quoting）**：把 99 行脚本压成单行。单引号内的 `'` 转义为 `\'`，换行转义为 `\n`。bash/zsh 都支持；
- **bash 历史兜底清理**：脚本尾部用 `history -d` 删除本条目（防 `ignorespace` 未生效）；
- **`\r` 结尾**：模拟终端 Enter（网络设备 CLI 也只认 CR）。

### 2. 脚本体来源（`shell_integration.sh`）

复用嵌入式脚本（`include_str!`），仅两处适配：

- 跳过文件头部的交互守卫与 `_ONETCLI_SHELL_INTEGRATED` 幂等守卫（前 3 行，运行时注入天然满足这些条件）；
- `__onetcli_precmd_common` 中注入轮次识别：`_ONETCLI_RUNTIME_SETUP` 存在时跳过命令记录（避免把注入命令自己记为"用户命令"）。

### 3. 探测（`ssh_backend.rs`）

```sh
case "${SHELL:-}" in *bash*|*zsh*) printf '__ONETCLI_SHELL_SUPPORTED__=1\n';; esac
```

- 通过独立 exec 通道执行（SSH exec 经登录 shell 解析，`$SHELL` 与交互 shell 一致）；
- 完整排空到 Close，避免与服务端 channel 复用竞争；
- 1s 超时；任何失败只记 warn，降级为不注入，不阻断连接；
- ash/dash 解析不了 bash 函数语法（实测 `Syntax error: "(" unexpected`），fish 被 `$SHELL` 探测排除——探测是必需的安全门。

### 4. 回显抑制状态机（`RuntimeShellIntegration`）

```
Disabled ────────────────────────────────────────┐ (未请求注入)
WaitingForFirstOutput ──should_inject──► Injecting│
       │ (期间收到 133;B)                        │
       └────────────► Integrated (legacy 跳过注入)│
Injecting ──ready marker──► AwaitingPrompt        │
    │                                            │
    └──5s 超时──► PlainAwaitingOutput ──► Plain   │
AwaitingPrompt ──133;B──► Integrated              │
```

- `Injecting` 期间所有输出进 `suppressed` 缓冲（上限 64KB，防止挂死 shell 无限撑内存）；
- 在缓冲中跨 chunk 搜索完成标记 `OSC 1337;ShellIntegrationReady=1`，找到后标记之后的 suffix 恢复转发；
- 注入期间 `accepts_terminal_input() == false`：actor 把用户 `Write`/`TerminalResponse` 命令压入 deferred 队列，就绪后按序重放。

### 5. Actor 接入（`ssh_backend.rs` 连接循环）

输出处理顺序：

```
decode → login_expect.advance → filter_output(抑制/转发)
  → osc_parser.push → exec_supervisor.on_terminal_chunk
  → TerminalEvent 分发（133;B 时 on_input_start + shell_ready）
  → should_inject? → 发送注入命令 + 起 5s 定时器
  → shell_ready 后发送 init_commands（integrated 无延迟；plain 保持 250ms 间隔）
```

- 注入完成（收到 ready marker）即取消定时器；
- 超时触发：`send 0x03`（Ctrl+C）→ 降级 Plain → 下一段输出携带 `ShellIntegrationReady::Plain` 恢复 `shell_ready`，放行暂存输入。

### 6. 旧版遗留兼容

- 远端 rc 仍带旧持久注入的机器：首个 prompt 自带 `OSC 133;B`，在 `WaitingForFirstOutput` 阶段被 `on_input_start()` 捕获，直接进入 `Integrated`，跳过重复注入；
- `SshBackend::uninstall_shell_integration`（10s 超时 + 成功标记确认）保留，表单提供"清理旧版文件"入口，一次性清理 `~/.config/onetcli` 与 rc managed block。

### 7. 删除的内容

- `build_shell_integration_setup_script` / `managed_shell_integration_block`（持久安装）
- `ShellIntegrationSetup` 结构体与 `SshSessionManager` 的 `cached_shell_integration` / `set_shell_integration`（每会话注入无需缓存）
- 表单"禁用 Shell 集成"开关（无远端写入后无风险可禁）
- `terminal.rs` 解析默认值回到 `unwrap_or(false)`（默认启用；已存储的显式禁用值仍被尊重）

## 验证

| 层级 | 内容 |
|---|---|
| 单元契约（7 个） | 命令单行化/不含远端路径与 mkdir；跨 chunk 回显抑制；超时降级；禁用路径直通；登录未完成/expect 轮延迟注入；legacy 133;B 检测；**真实 bash/zsh 执行注入命令并输出完成标记** |
| ssh_backend mock | 探测通道只读（Exec+Close）；不支持 shell 跳过注入；channel open 失败单通道重连；探测超时降级 |
| 真机（181，`#[ignore]`） | 旧残留自动卸载 → 探测 → 注入 → 回显无泄漏 → 133;B 就绪 → `echo` 经 OSC 1337 记录 → **连接前后远端 md5 快照一致** |
| 回归 | terminal 446+5、ssh 144、terminal_view 35 全通过 |

真机测试入口：

```sh
NAVOP_LIVE_SSH=user:password@host:port \
cargo test -p terminal --test ssh_runtime_integration_live -- --ignored --nocapture
```

（密码含 `@` 时从右往左解析。）

## 权衡与已知边界

- **每连接注入一次**：新开终端tab 都要注入（~百毫秒级），换来零残留；相比旧版首次安装+缓存略慢但更稳；
- **注入命令回显**：依赖完成标记对齐，理论上 shell 极端挂死时 5s 超时兜底（丢弃回显、Ctrl+C、降级）；
- **探测基于 `$SHELL`**：登录 shell 与实际交互 shell 不一致的边缘场景（罕见）会误注入/漏注入，bash/zsh 误注入也只是语法错误提示，不破坏会话；
- **`eval $'...'` 是 bash/zsh 特性**：正因如此探测只放行 bash/zsh；未来要支持 fish 需单独的注入体。
