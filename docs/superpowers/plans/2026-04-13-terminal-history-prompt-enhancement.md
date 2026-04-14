# Terminal History Prompt 全面优化计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 解决终端历史命令"记不全"和提示"不够丝滑"两大核心问题。参考 fish shell / Atuin / zsh-autosuggestions / Warp 的业界最佳实践，将历史提示系统从"基础可用"提升到"丝滑好用"。

**Architecture:** 分 4 个阶段渐进实施，每阶段独立可交付：
1. **数据采集完善** — 本地终端 Shell Integration 自动注入 + OSC 事件解析
2. **匹配算法升级** — InlineSuggest 策略链 + Frecency 排序
3. **交互体验打磨** — 部分接受、减少激进失效、异步防抖
4. **历史持久化** — 富元数据 + SQLite 跨会话持久存储

**Tech Stack:** Rust, gpui, terminal_view, terminal

---

## 当前问题诊断

### 历史命令记不全（数据层）

| 问题 | 根因 | 参考 |
|------|------|------|
| 本地终端无 shell integration | SSH 自动部署 `shell_integration.sh`，本地终端完全不注入 | fish/Atuin 均自动注入 |
| alacritty 后端不解析 OSC 1337 | `GpuiEventProxy` 不处理 `CommandRecorded` 事件 | SSH 后端已实现 |
| 只有相邻去重 | `push_history_entry` 仅检查 `entries.back()` | Atuin 按 command 全局去重 |
| 无元数据 | 只存 `String`，不记录 exit_code/cwd/timestamp | Atuin 存 7+ 字段 |
| 会话历史不持久 | `session_history` 纯内存，重启丢失 | Atuin 用 SQLite |
| 容量太小 | session 256, persisted 512 | Atuin 无限，fish 默认 256000 |

### 提示不够丝滑（匹配层 + 交互层）

| 问题 | 根因 | 参考 |
|------|------|------|
| InlineSuggest 只做前缀匹配 | `command.starts_with(prefix)` | zsh-autosuggestions 策略链 |
| 无 frecency 排序 | 按固定 session→persisted 顺序 | Atuin/Warp 用 frecency |
| 无部分接受 | 只有 Right/Tab 全部接受 | fish Ctrl+Right 逐词接受 |
| 失效太激进 | Left/Home/End 直接 invalidate | fish 光标移动不丢失建议 |
| 无异步/防抖 | 同步搜索，每个字符触发全量遍历 | fish 后台线程 + 防抖 |

---

## Phase 1: 本地终端命令采集完善

> 解决"记不全"的根本问题。当前本地终端靠客户端 Enter 键猜测命令，会漏掉 `!!`、`!$`、alias 展开、多行命令等。目标是让本地终端也通过 shell hooks 准确记录。

### Task 1: 本地终端自动注入 Shell Integration

**Files:**
- Modify: `crates/terminal/src/terminal.rs`
- Modify: `crates/terminal/src/shell_integration.sh`
- Reference: `crates/terminal/src/ssh_backend.rs` (SSH 注入逻辑参考)

- [ ] **Step 1: 写失败测试**

添加测试覆盖：
- `local_terminal_injects_shell_integration_env` — 本地终端启动时设置 `ONETCLI_SHELL_INTEGRATION` 环境变量指向脚本路径
- `shell_integration_script_exists_at_expected_path` — 验证脚本文件在预期位置

```
cargo test -p terminal shell_integration -- --nocapture
```
Expected: FAIL

- [ ] **Step 2: 实现本地终端注入机制**

参考 Warp/VS Code 的做法，通过环境变量注入而非修改 rc 文件（避免污染用户配置）：

1. 在 `Terminal::new_local()` 创建 PTY 时，设置环境变量：
   - `ONETCLI_SHELL_INTEGRATION_DIR` → 指向包含 `shell_integration.sh` 的目录
2. 修改 `shell_integration.sh`，在脚本头部添加环境变量检测，支持被外部 source
3. 在 PTY 启动参数中注入 `--init-command` 或修改 `BASH_ENV` / `ZDOTDIR` 来自动 source：
   - **Bash**: 设置 `BASH_ENV=/path/to/shell_integration.sh`（非交互 shell）+ 在 `~/.bashrc` 末尾自动追加（首次时）
   - **Zsh**: 通过设置自定义 `ZDOTDIR` 指向含 `.zshrc` 的临时目录，该 `.zshrc` 先 source 原始 `~/.zshrc` 再 source `shell_integration.sh`
   - **推荐方案**: 类似 VS Code 的方式，设置 `ONETCLI_SHELL_INTEGRATION=1` 环境变量，并在 PTY 的 init command 中直接 `source /path/to/shell_integration.sh`

注意：
- 检查 `_ONETCLI_SHELL_INTEGRATED` 防止重复注入
- 对于已经手动 source 过的用户不会有副作用（脚本自带 guard）

- [ ] **Step 3: 运行测试验证**

```
cargo test -p terminal shell_integration -- --nocapture
```
Expected: PASS

### Task 2: 本地 PTY 后端解析 OSC 1337 CommandRecorded 事件

**Files:**
- Modify: `crates/terminal/src/pty_backend.rs`
- Modify: `crates/terminal/src/terminal.rs`
- Reference: `crates/terminal/src/ssh_backend.rs` (`extract_osc_events` / `parse_osc_payload`)

- [ ] **Step 1: 写失败测试**

添加测试覆盖：
- `local_pty_parses_osc_1337_command_recorded` — 模拟 PTY 输出含 `ESC]1337;Command=<base64>BEL` 的字节流，验证 `TerminalEvent::CommandRecorded` 被正确发出
- `local_pty_parses_osc_133_d_command_finished` — 解析 `ESC]133;D;0BEL` 得到 exit_code=0

```
cargo test -p terminal osc_event -- --nocapture
```
Expected: FAIL

- [ ] **Step 2: 在 alacritty 事件流中拦截 OSC 事件**

当前 `GpuiEventProxy` 实现了 alacritty 的 `EventListener` trait，但 alacritty 的 VTE parser 会消费 OSC 序列。有两种实现路径：

**路径 A（推荐）**: 在 alacritty 的 `Perform` trait 实现中添加 OSC 1337/133 处理：
- 查找 `alacritty_terminal` 中 `osc_dispatch` 方法
- 在其中检测 `1337;Command=` 前缀，发出 `TerminalEvent::CommandRecorded`
- 检测 `133;D;` 前缀，发出 `TerminalEvent::CommandFinished`

**路径 B**: 在 PTY 读取循环中，像 SSH 后端一样在数据写入 VTE parser 之前先扫描 OSC 序列：
- 复用 `ssh_backend.rs` 中的 `extract_osc_events()` + `parse_osc_payload()` 逻辑
- 将这两个函数提取到公共模块 `crates/terminal/src/osc.rs`
- 在本地 PTY 的数据处理流程中调用

- [ ] **Step 3: 移除 Enter 键客户端记录的 fallback（或降级为 backup）**

在 `view.rs` 中：
- 当 shell integration 已激活（收到过 OSC 133;A prompt_start 事件）时，不再在 Enter 键处理中记录命令
- 保留 Enter 键记录作为 fallback，仅在未检测到 shell integration 时启用

- [ ] **Step 4: 运行测试验证**

```
cargo test -p terminal osc_event -- --nocapture
cargo test -p terminal_view history_prompt -- --nocapture
```
Expected: PASS

### Task 3: 验证 Phase 1 集成

- [ ] **Step 1: 编译验证**

```
cargo check -p terminal -p terminal_view
```
Expected: PASS

- [ ] **Step 2: 手动验证**

在本地终端中：
1. 打开新终端 tab
2. 输入 `echo hello` 并执行
3. 输入 `!!` 执行历史展开
4. 输入 `echo` 并按 Tab 补全文件名后执行
5. 验证以上命令都被正确记录到历史中

- [ ] **Step 3: 提交**

```bash
git add crates/terminal/ crates/terminal_view/
git commit -m "feat(terminal): 本地终端自动注入 shell integration 并解析 OSC 命令记录"
```

---

## Phase 2: 匹配算法升级

> 解决"提示不够丝滑"的匹配质量问题。InlineSuggest 从纯前缀匹配升级为策略链，并引入 frecency 排序。

### Task 4: InlineSuggest 策略链

**Files:**
- Modify: `crates/terminal/src/history.rs`
- Test: `crates/terminal/src/history.rs`

- [ ] **Step 1: 写失败测试**

添加测试覆盖：
- `inline_suggest_matches_substring` — 输入 `test` 应匹配 `cargo test`（当前只能匹配 `test-xxx`）
- `inline_suggest_matches_token_prefix` — 输入 `status` 应匹配 `git status`
- `inline_suggest_prefix_ranked_first` — 前缀匹配结果排在子串匹配前面
- `inline_suggest_uses_strategy_chain` — 策略链按顺序尝试：prefix → token_prefix → substring

```
cargo test -p terminal history -- --nocapture
```
Expected: FAIL（当前 `collect_history_suggestions` 只做前缀匹配）

- [ ] **Step 2: 实现策略链匹配**

修改 `collect_history_suggestions()`：

```
策略链（参考 zsh-autosuggestions 的 ZSH_AUTOSUGGEST_STRATEGY）:
1. Prefix match: command.starts_with(query)           → rank 0
2. Token prefix: 任意 token 以 query 开头              → rank 1  
3. Substring: command.contains(query)                  → rank 2
```

注意：InlineSuggest 的 ghost text 只显示第一个匹配的后缀部分。对于非前缀匹配（token_prefix / substring），ghost text 应显示完整命令而非只追加后缀。需要修改 `HistoryPromptAccept` 增加 `ReplaceWithCommand(String)` 变体。

**或者更简单的方案**: InlineSuggest 的 ghost text 只用 rank 0（前缀匹配）的第一个结果，但**下拉列表**同时显示所有策略的结果。这样保持 ghost text 的直觉性，同时让下拉列表更丰富。

- [ ] **Step 3: 运行测试验证**

```
cargo test -p terminal history -- --nocapture
```
Expected: PASS

### Task 5: Frecency 排序

**Files:**
- Modify: `crates/terminal/src/history.rs`
- Modify: `crates/terminal/src/terminal.rs`
- Test: `crates/terminal/src/history.rs`

- [ ] **Step 1: 引入 HistoryEntry 结构体**

将历史存储从 `VecDeque<String>` 升级为 `VecDeque<HistoryEntry>`：

```rust
pub struct HistoryEntry {
    pub command: String,
    pub timestamp: Instant,      // 最后执行时间
    pub use_count: u32,          // 在当前会话中的使用次数
    pub cwd: Option<String>,     // 执行时的工作目录
    pub exit_code: Option<i32>,  // 退出码（0=成功）
}
```

- [ ] **Step 2: 写失败测试**

添加测试覆盖：
- `frecency_ranks_frequent_commands_higher` — 使用频率高的命令排在前面
- `frecency_ranks_recent_commands_higher` — 最近使用的命令排在前面
- `frecency_balances_frequency_and_recency` — 兼顾频率和时近度
- `frecency_deprioritizes_failed_commands` — exit_code != 0 的命令排名靠后（不过滤，只降权）
- `frecency_boosts_same_directory_commands` — 当前目录下执行过的命令获得额外加分

```
cargo test -p terminal frecency -- --nocapture
```
Expected: FAIL

- [ ] **Step 3: 实现 frecency 评分**

评分公式（参考 Atuin/Firefox frecency）：

```
score = use_count × recency_weight × dir_bonus × success_bonus

recency_weight:
  - 最近 5 分钟内:  100
  - 最近 1 小时内:  70
  - 最近 24 小时内: 50
  - 更早:           30

dir_bonus:
  - 相同 cwd: 1.5
  - 不同 cwd: 1.0

success_bonus:
  - exit_code == 0 或 None: 1.0
  - exit_code != 0:         0.5
```

修改 `collect_history_suggestions()` 和 `collect_history_search_results()`：
- 在 rank 相同时用 frecency score 排序替代简单的 recency index

- [ ] **Step 4: 在命令记录时更新元数据**

修改 `push_history_entry()` → `push_history_entry_rich()`：
- 接受 `HistoryEntry` 而非 `&str`
- 全局去重：如果已存在相同命令，更新其 `timestamp` 和 `use_count`，而非添加新条目
- 保留旧的 `push_history_entry()` 作为兼容层（从 `String` 构造默认 `HistoryEntry`）

修改 `record_history_entry()`:
- 从 `TerminalEvent::CommandFinished { exit_code }` 捕获退出码
- 从 `Terminal` 的当前 `cwd` 字段获取工作目录

- [ ] **Step 5: 运行测试验证**

```
cargo test -p terminal frecency -- --nocapture
cargo test -p terminal history -- --nocapture
```
Expected: PASS

### Task 6: 验证 Phase 2 集成

- [ ] **Step 1: 全量测试**

```
cargo test -p terminal -- --nocapture
cargo test -p terminal_view history_prompt -- --nocapture
```
Expected: PASS

- [ ] **Step 2: 编译验证**

```
cargo check -p terminal -p terminal_view
```
Expected: PASS

- [ ] **Step 3: 提交**

```bash
git add crates/terminal/ crates/terminal_view/
git commit -m "feat(terminal): InlineSuggest 策略链匹配 + frecency 排序"
```

---

## Phase 3: 交互体验打磨

> 对标 fish shell 的丝滑体验。部分接受、减少激进失效、异步防抖。

### Task 7: 部分接受（逐词接受）

**Files:**
- Modify: `crates/terminal_view/src/history_prompt.rs`
- Modify: `crates/terminal_view/src/view.rs`
- Test: `crates/terminal_view/src/history_prompt.rs`

- [ ] **Step 1: 写失败测试**

添加测试覆盖：
- `accept_next_word_returns_first_word_of_suffix` — 输入 `git`，建议 `git status --short`，Ctrl+Right 应只接受 ` status`（到下一个空格/词边界）
- `accept_next_word_updates_input` — 接受后 `input` 变为 `git status`
- `accept_next_word_on_last_word_accepts_all` — 只剩一个词时等同于全部接受

```
cargo test -p terminal_view history_prompt -- --nocapture
```
Expected: FAIL

- [ ] **Step 2: 在 HistoryPromptState 中添加部分接受逻辑**

添加 `accept_next_word()` 方法：
```rust
pub fn accept_next_word(&mut self) -> Option<HistoryPromptAccept> {
    let candidate = self.selected_match()?.to_string();
    let suffix = candidate.strip_prefix(self.query_input())?;
    if suffix.is_empty() { return None; }
    
    // 找到下一个词边界（空格/路径分隔符）
    let word_end = suffix.trim_start()
        .find(|c: char| c.is_whitespace() || c == '/')
        .map(|i| i + suffix.len() - suffix.trim_start().len())
        .unwrap_or(suffix.len());
    
    let word = &suffix[..word_end];
    self.input.push_str(word);
    // 不清除 selected，保持同一条建议继续部分接受
    Some(HistoryPromptAccept::AppendSuffix(word.to_string()))
}
```

- [ ] **Step 3: 绑定键位**

在 `view.rs` 的 `handle_key_event` 中：
- **Ctrl+Right** / **Alt+F** → 调用 `accept_next_word()`
- 保持 **Right** / **Tab** → 全部接受（现有行为不变）

- [ ] **Step 4: 运行测试验证**

```
cargo test -p terminal_view history_prompt -- --nocapture
```
Expected: PASS

### Task 8: 减少激进 Invalidation

**Files:**
- Modify: `crates/terminal_view/src/history_prompt.rs`
- Modify: `crates/terminal_view/src/view.rs`
- Test: `crates/terminal_view/src/history_prompt.rs`

- [ ] **Step 1: 分析当前的 invalidation 触发点**

当前会 invalidate 的操作（过于激进）：
- ✅ 合理: Vi mode、鼠标点击、Ctrl+任意键（除 Ctrl-U/C/R）、多行粘贴
- ❌ 过于激进: `Left`、`Home`、`End`、`Delete`、`PageUp`、`PageDown`

问题：用户按 Left 键微调光标位置后，所有建议消失，需要清除后从头输入。

- [ ] **Step 2: 写失败测试**

添加测试覆盖：
- `left_key_suspends_but_does_not_invalidate` — 按 Left 后建议隐藏，但再按字符输入时应恢复跟踪（而非需要清除重来）
- `home_end_suspends_tracking` — Home/End 暂停跟踪但不清除 input
- `resume_tracking_after_suspension` — 暂停后输入字符自动恢复

```
cargo test -p terminal_view history_prompt -- --nocapture
```
Expected: FAIL

- [ ] **Step 3: 引入 "暂停" 状态替代 "失效"**

在 `HistoryPromptState` 中：
- 将 `tracking: bool` 升级为三态 enum:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackingState {
    #[default]
    Active,     // 正常跟踪，显示建议
    Suspended,  // 暂停（光标移动等），隐藏建议，但保留 input
    Invalidated, // 完全失效（鼠标点击、Vi mode 等），需要 clear() 重置
}
```

行为差异：

| 操作 | Active | Suspended | Invalidated |
|------|--------|-----------|-------------|
| 输入字符 | 追加到 input + 刷新建议 | **恢复 Active** + 重置 input 为当前光标位置文本 | 忽略 |
| 显示建议 | ✅ | ❌ | ❌ |
| clear() 后 | → Active | → Active | → Active |

触发映射变更：
- `Left` / `Home` / `End` / `Delete` → `suspend()`（之前是 `invalidate()`）
- `PageUp` / `PageDown` → 保持 `invalidate()`（翻页后光标位置完全不可预测）
- 鼠标点击 → 保持 `invalidate()`
- Vi mode → 保持 `invalidate()`

- [ ] **Step 4: 实现 suspend 机制**

```rust
pub fn suspend(&mut self) {
    if self.tracking_state == TrackingState::Active {
        self.tracking_state = TrackingState::Suspended;
        self.matches.clear();
        self.selected = None;
    }
}

pub fn resume_with_input(&mut self, input: String) {
    self.tracking_state = TrackingState::Active;
    self.input = input;
    self.selected = None;
}
```

- [ ] **Step 5: 运行测试验证**

```
cargo test -p terminal_view history_prompt -- --nocapture
```
Expected: PASS

### Task 9: 异步匹配 + 防抖

**Files:**
- Modify: `crates/terminal_view/src/view.rs`
- Test: `crates/terminal_view/src/view.rs`

- [ ] **Step 1: 写失败测试**

添加测试覆盖：
- `rapid_typing_debounces_suggestion_refresh` — 快速连续输入 5 个字符，验证 `refresh_history_prompt_matches` 不会被调用 5 次
- `suggestion_refresh_fires_after_debounce_delay` — 停止输入 50ms 后触发一次刷新

- [ ] **Step 2: 实现防抖机制**

在 `TerminalView` 中添加防抖逻辑：

```rust
// 在 TerminalView 结构体中添加
suggestion_debounce: Option<gpui::Task<()>>,
```

修改 `apply_inline_input_to_history_prompt()`：
```rust
fn apply_inline_input_to_history_prompt(&mut self, text: &str, cx: &mut Context<Self>) {
    if !self.history_prompt_enabled(cx) {
        self.invalidate_history_prompt();
        return;
    }
    self.history_prompt.append_text(text);
    
    // 取消上一个防抖任务
    self.suggestion_debounce.take();
    
    // 启动新的防抖延迟（50ms）
    self.suggestion_debounce = Some(cx.spawn(|this, mut cx| async move {
        cx.background_executor().timer(Duration::from_millis(50)).await;
        let _ = this.update(&mut cx, |this, cx| {
            this.refresh_history_prompt_matches(cx);
            cx.notify();
        });
    }));
}
```

注意：Search 模式不需要防抖（用户对搜索的即时反馈有更强期望）。

- [ ] **Step 3: 运行测试验证**

```
cargo test -p terminal_view -- --nocapture
```
Expected: PASS

### Task 10: 验证 Phase 3 集成

- [ ] **Step 1: 全量测试**

```
cargo test -p terminal -p terminal_view -- --nocapture
```
Expected: PASS

- [ ] **Step 2: 编译验证**

```
cargo check -p terminal -p terminal_view
```
Expected: PASS

- [ ] **Step 3: 提交**

```bash
git add crates/terminal/ crates/terminal_view/
git commit -m "feat(terminal): 部分接受、暂停态替代激进失效、异步防抖"
```

---

## Phase 4: 历史持久化（进阶，可选）

> 让 OnetCli 内记录的命令跨会话保存，并存储丰富元数据用于智能排序。

### Task 11: SQLite 持久化存储

**Files:**
- Create: `crates/terminal/src/history_db.rs`
- Modify: `crates/terminal/src/history.rs`
- Modify: `crates/terminal/src/terminal.rs`
- Test: `crates/terminal/src/history_db.rs`

- [ ] **Step 1: 设计 SQLite Schema**

```sql
CREATE TABLE IF NOT EXISTS command_history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    command     TEXT    NOT NULL,
    timestamp   INTEGER NOT NULL,  -- Unix timestamp (秒)
    duration_ms INTEGER,           -- 执行耗时（毫秒）
    exit_code   INTEGER,           -- 退出码
    cwd         TEXT,              -- 工作目录
    session_id  TEXT,              -- OnetCli 会话 ID
    hostname    TEXT,              -- 主机名（区分本地/SSH）
    shell       TEXT               -- bash/zsh/fish
);

CREATE INDEX IF NOT EXISTS idx_command ON command_history(command);
CREATE INDEX IF NOT EXISTS idx_timestamp ON command_history(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_cwd ON command_history(cwd);
```

存储位置：`~/.config/onetcli/history.db`

- [ ] **Step 2: 写失败测试**

添加测试覆盖：
- `history_db_inserts_and_queries` — 插入命令后能查询到
- `history_db_deduplicates_by_updating` — 插入重复命令时更新 timestamp 和 use_count
- `history_db_prefix_search` — 前缀搜索返回正确结果
- `history_db_frecency_ordering` — 按 frecency 评分排序
- `history_db_filters_by_cwd` — 支持按工作目录过滤
- `history_db_limits_results` — 限制返回数量

```
cargo test -p terminal history_db -- --nocapture
```
Expected: FAIL

- [ ] **Step 3: 实现 HistoryDb**

使用 `rusqlite`（或项目已有的 SQLite 依赖）：

```rust
pub struct HistoryDb {
    conn: Connection,
}

impl HistoryDb {
    pub fn open(path: &Path) -> Result<Self>;
    pub fn record(&self, entry: &HistoryEntry) -> Result<()>;
    pub fn search_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<HistoryEntry>>;
    pub fn search_fuzzy(&self, query: &str, limit: usize) -> Result<Vec<HistoryEntry>>;
    pub fn search_frecent(&self, prefix: &str, cwd: Option<&str>, limit: usize) -> Result<Vec<HistoryEntry>>;
}
```

- [ ] **Step 4: 集成到 Terminal**

修改 `Terminal`：
- 启动时：打开 HistoryDb 并加载最近 N 条作为 `persisted_history` 初始值（替代读取 shell history 文件）
- 记录时：同时写入 session_history 和 HistoryDb
- 查询时：合并 session_history 和 HistoryDb 查询结果

保留 shell history 文件读取作为 fallback（首次迁移时导入一次）。

- [ ] **Step 5: 提升容量限制**

```rust
pub const SESSION_HISTORY_LIMIT: usize = 1024;    // 256 → 1024
pub const PERSISTED_HISTORY_LIMIT: usize = 10_000; // 512 → 10000（SQLite 无压力）
```

- [ ] **Step 6: 运行测试验证**

```
cargo test -p terminal history_db -- --nocapture
cargo test -p terminal -- --nocapture
```
Expected: PASS

- [ ] **Step 7: 提交**

```bash
git add crates/terminal/
git commit -m "feat(terminal): SQLite 持久化历史记录 + 丰富元数据"
```

---

## 附录 A: 业界参考对照

| 特性 | fish | zsh-autosuggestions | Atuin | Warp | OnetCli (当前) | OnetCli (目标) |
|------|------|---------------------|-------|------|---------------|---------------|
| 数据采集 | Shell 内置 | Shell 内置 | Shell hook | PTY 解析 | Enter 键猜测 | Shell hook |
| 存储 | 文件 | Shell history | SQLite | 未知 | 内存 | SQLite |
| 前缀匹配 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 子串匹配 | ❌ | ❌ | ✅ | ✅ | ❌(搜索模式有) | ✅ |
| 模糊匹配 | ❌ | ❌ | ✅ | ❌ | ❌(搜索模式有) | ✅(搜索模式) |
| Frecency | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ |
| 目录感知 | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ |
| 失败过滤 | ❌ | ❌ | ✅ | ❌ | ❌ | ✅(降权) |
| 部分接受 | ✅(Ctrl+Right) | ❌ | N/A | ❌ | ❌ | ✅ |
| 异步匹配 | ✅ | ❌ | ✅ | ✅ | ❌ | ✅ |
| 防抖 | 隐式(async) | ❌ | 隐式 | ❌ | ❌ | ✅(50ms) |

## 附录 B: 实施优先级

| 优先级 | 任务 | 影响 | 复杂度 | 建议 |
|--------|------|------|--------|------|
| P0 | Task 1+2: Shell Integration 注入 + OSC 解析 | 解决"记不全"根本原因 | 中 | 先做 |
| P0 | Task 4: InlineSuggest 策略链 | 解决"输入 test 匹配不到 cargo test" | 低 | 先做 |
| P1 | Task 7: 部分接受 | fish 核心体验 | 低 | 与 Task 4 并行 |
| P1 | Task 8: 减少激进 invalidation | 避免"光标一动全没了" | 中 | 紧跟 Task 7 |
| P2 | Task 5: Frecency 排序 | 智能排序需要 HistoryEntry 重构 | 中 | Phase 2 |
| P2 | Task 9: 异步防抖 | 性能优化，当前数据量小影响不大 | 低 | Phase 3 |
| P3 | Task 11: SQLite 持久化 | 长期价值高，但短期不阻塞 | 高 | Phase 4 |

## 附录 C: 风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| Shell integration 注入影响用户已有 shell 配置 | 高 | 使用环境变量注入而非修改 rc 文件；添加 `_ONETCLI_SHELL_INTEGRATED` guard |
| alacritty_terminal 不暴露 OSC 回调 | 中 | 路径 B（数据流预扫描）作为备选 |
| HistoryEntry 重构影响现有 API | 中 | 保留旧 API 作为兼容层，内部转换 |
| SQLite 引入新依赖 | 低 | 项目已使用 sqlx/rusqlite（需确认） |
| 防抖可能导致建议延迟感知 | 低 | 50ms 阈值远低于人类感知（~100ms） |
