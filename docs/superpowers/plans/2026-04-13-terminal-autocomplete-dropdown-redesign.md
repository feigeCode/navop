# 终端历史补全下拉菜单重设计

## 问题分析

### 当前实现的核心缺陷

对比 Termius 的行为，当前实现存在 3 个根本性问题：

#### 1. 状态机过于激进地失效（"有时候弹出，有时候无法"）

当前采用三态设计 `Active → Suspended → Invalidated`，以下操作会导致失效且**无法自动恢复**：

| 用户操作 | 当前行为 | 问题 |
|---------|---------|------|
| Left/Home/End/Delete | `suspend()` → 隐藏建议 | 再次输入时 `resume_with_input` 用单字符重建 input，丢失上下文 |
| PageUp/PageDown | `invalidate()` → 彻底失效 | 必须 Enter/Ctrl+C 后 `clear()` 才能恢复 |
| 鼠标点击终端 | `invalidate()` → 彻底失效 | 同上 |
| 任何未识别的修饰键组合 | `invalidate()` → 彻底失效 | 如 Ctrl+L 清屏后补全永远不出 |
| Vi 模式触发 | `invalidate()` → 彻底失效 | 退出 Vi 模式后也不恢复 |

**结论**：用户日常操作中大量路径会导致失效，且恢复条件苛刻。

#### 2. 鼠标交互模式错误（"鼠标上下选择是直接就选中内容"）

| 行为 | Termius | 当前实现 |
|------|---------|---------|
| 鼠标悬停 | 高亮当前项 | 无响应 |
| 鼠标点击 | 选中并填入 | 选中并立即填入（正确但无悬停预览） |
| 上下键 | 高亮切换，可预览 | 高亮切换，但不更新预览 |

#### 3. 下拉菜单显示不稳定

`render_history_prompt_overlay` 有过多的 early-return 条件：
```rust
if !self.history_prompt_enabled(cx) || !self.history_prompt.is_active() {
    return None;  // is_active() 要求 TrackingState::Active
}
if !search_mode && matches.is_empty() {
    return None;  // 50ms 防抖期间 matches 为空，闪烁
}
```

---

## 设计方案

### 核心思路：简化状态 + 可靠触发 + 丰富交互

```
┌────────────────────────────────────────────────────┐
│                 新状态机（两态）                      │
│                                                    │
│   Dismissed ◄──── Enter/Ctrl+C/Escape ───── Shown  │
│       │                                      ▲     │
│       │          任何可打印字符输入             │     │
│       └──────────────────────────────────────┘     │
│                                                    │
│   鼠标点击/Left/Right/Home/End → 隐藏下拉框          │
│   但保留 input，下次输入字符时立即恢复               │
└────────────────────────────────────────────────────┘
```

### 变更 1：状态机简化（`history_prompt.rs`）

**删除 `Suspended` 状态**，改为两态：

```rust
pub enum TrackingState {
    /// 活跃：跟踪输入，显示下拉菜单
    Active,
    /// 已关闭：Enter/Ctrl+C 后，需要新的输入字符才能重新激活
    Dismissed,
}
```

**关键规则变更**：

| 操作 | 旧行为 | 新行为 |
|------|--------|--------|
| Left/Right/Home/End | `Suspended` (需 `resume_with_input`) | **保持 Active**，仅临时隐藏下拉（`dropdown_visible = false`） |
| PageUp/PageDown | `Invalidated` (需 `clear()`) | **保持 Active**，隐藏下拉 |
| 鼠标点击终端 | `Invalidated` (需 `clear()`) | **保持 Active**，隐藏下拉 |
| Ctrl+L 清屏 | `Invalidated` | **保持 Active**，`input` 清空 |
| 下一个可打印字符 | 需要 `resume_with_input` | **直接追加到 input**，显示下拉 |
| Enter / Ctrl+C / Ctrl+U | `clear()` | `Dismissed`，清空 input |

新增 `dropdown_visible: bool` 字段，与 `TrackingState` 解耦：
- `Active + dropdown_visible=true` → 显示下拉
- `Active + dropdown_visible=false` → 隐藏下拉，但 input 保留，下次输入立即显示
- `Dismissed` → 一切重置

### 变更 2：鼠标交互增强（`view.rs` 渲染部分）

```
下拉菜单交互模型：

┌──────────────────────────────┐
│ 🕒 cd /data/seeyon/ai-mana… │  ← 鼠标悬停 = 高亮
│ 🕒 cd /data/                 │  ← 上下键 = 切换高亮
│ 🕒 cd /data/seeyon/ai-mana… │  ← 点击 = 选中填入
│ 🕒 cd /data/seeyon/applic…  │  ← Enter/Right = 接受当前高亮项
│ 🕒 cd /data/seeyon/ai-mana… │  ← Tab = 接受当前高亮项（可选）
└──────────────────────────────┘
     ↑ 最大显示 8 条，超出滚动
```

具体实现：

```rust
// 每个 dropdown item 增加 hover 事件
.on_mouse_move({
    let view = view.clone();
    move |_, _, cx| {
        cx.stop_propagation();
        view.update(cx, |this, cx| {
            // 悬停 → 仅更新高亮，不触发接受
            this.history_prompt.select_match(index);
            cx.notify();
        });
    }
})
.on_mouse_down(MouseButton::Left, {
    let view = view.clone();
    move |_, _, cx| {
        cx.stop_propagation();
        view.update(cx, |this, cx| {
            this.history_prompt.select_match(index);
            this.try_accept_history_prompt(cx);
        });
    }
})
```

### 变更 3：防抖优化（`view.rs`）

当前 50ms 防抖在快速输入时导致下拉菜单闪烁（matches 先清空再填充）。改为：

```rust
fn schedule_debounced_refresh(&mut self, cx: &mut Context<Self>) {
    self.suggestion_debounce.take();
    self.suggestion_debounce = Some(cx.spawn(async move |this, cx| {
        cx.background_executor()
            .timer(Duration::from_millis(30)) // 缩短到 30ms
            .await;
        let _ = this.update(cx, |this, cx| {
            this.refresh_history_prompt_matches(cx);
            cx.notify();
        });
    }));
}
```

**关键改进**：防抖期间保留上一次的 matches 不清空，避免闪烁。只在新结果返回时替换。

### 变更 4：渲染稳定性（`view.rs`）

```rust
fn render_history_prompt_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
    // 简化显示条件：只要 Active 且 dropdown_visible 且有 matches
    if !self.history_prompt_enabled(cx) {
        return None;
    }
    if !self.history_prompt.is_active() || !self.history_prompt.dropdown_visible() {
        return None;
    }
    let matches = self.history_prompt.matches();
    if matches.is_empty() && self.history_prompt.mode() != HistoryPromptMode::Search {
        return None;
    }
    // ... 渲染下拉菜单
}
```

### 变更 5：视觉改进

```
当前样式：                       目标样式（参考 Termius）：
┌─────────────────┐             ┌──────────────────────────────────┐
│ git status      │             │ 🕒  cd /data/seeyon/ai-manager/  │
│ git stash       │             │ 🕒  cd /data/                     │
│ git switch      │             │ 🕒  cd /data/seeyon/temp/config   │
└─────────────────┘             │ 🕒  cd /data/seeyon/application   │
                                └──────────────────────────────────┘
```

- 每项前添加 🕒 图标（历史记录用 `IconName::History`）
- 宽度自适应内容（`min_w(px(300))` → `max_w(px(500))`）
- 选中项用更明显的背景色
- ghost text（内联灰色预览）与下拉菜单同时显示

---

## 实现步骤

### Step 1: 状态机重构（`history_prompt.rs`）
- 删除 `TrackingState::Suspended`
- 新增 `dropdown_visible: bool` 字段
- 修改 `suspend()` → `hide_dropdown()`（保留 input，仅隐藏）
- 删除 `resume_with_input()`（不再需要）
- `invalidate()` 重命名为 `dismiss()`
- 所有现有测试适配新 API

### Step 2: 视图层适配（`view.rs`）
- `handle_key_event` 中：
  - Left/Right/Home/End → `hide_dropdown()` 替代 `suspend()`
  - PageUp/PageDown → `hide_dropdown()` 替代 `invalidate()`
  - 可打印字符 → `append_text()` + `show_dropdown()` + debounce refresh
  - Enter/Ctrl+C → `dismiss()`
- 删除 `suspend_history_prompt()`、`invalidate_history_prompt()` 的大量调用
- 鼠标点击终端 → `hide_dropdown()` 替代 `invalidate()`

### Step 3: 鼠标悬停交互（`view.rs` 渲染）
- dropdown item 添加 `on_mouse_move` → `select_match(index)` + `cx.notify()`
- `on_mouse_down` 保持 → `select_match(index)` + `try_accept_history_prompt()`
- 分离 `select_history_prompt_match`：仅高亮，不自动接受

### Step 4: 防抖优化
- 防抖期间不清空旧 matches
- 缩短防抖时间 50ms → 30ms

### Step 5: 视觉改进
- 添加历史图标
- 加宽下拉框
- 选中项更明显的高亮色

---

## 文件变更清单

| 文件 | 变更类型 | 内容 |
|------|---------|------|
| `crates/terminal_view/src/history_prompt.rs` | 重构 | 删除 Suspended 态，新增 dropdown_visible，API 重命名 |
| `crates/terminal_view/src/view.rs` | 修改 | 状态转换调用适配，鼠标悬停，渲染改进，防抖优化 |

预计改动量：~200 行修改，净增 ~30 行。
