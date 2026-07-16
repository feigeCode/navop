# 终端原生分屏与共享侧边栏设计

## 背景

当前应用由 `SplitTabContainer` 在 `TabContainer` 外层提供通用分屏。每个分屏叶子都是完整的 `TabContainer`，终端只是其中一种可分屏的 `TabContent`。该模型允许数据库、SFTP、Redis、MongoDB、Remote Desktop 和终端等页面进入通用分屏，但分屏行为不属于这些页面自身，终端也无法围绕 PTY、焦点、侧边栏和连接上下文形成完整的工作区语义。

`TerminalView` 当前同时拥有单个终端会话和完整内部工具 dock。每个终端实例都会创建自己的工具栏、设置、快捷命令、历史命令、Rich Input、AI Chat、文件管理器和服务器监控。若继续以完整 `TerminalView` 作为分屏叶子，会在每个 pane 中重复创建和渲染侧边栏。

本设计将分屏收敛为终端专属能力：删除通用 `SplitTabContainer`，让一个 Terminal Tab 自己管理多个终端 pane；整个 Terminal Tab 只显示一套 workspace 级侧边栏。

## 目标

- 删除 `SplitTabContainer` 和 `TabContainer` 的通用分屏能力。
- 只有终端支持上、下、左、右分屏和分隔线拖动。
- 一个 Terminal Tab 内可以组合多个 Local、SSH 或 Serial 终端 pane。
- 整个 Terminal Tab 只渲染一套侧边栏和工具栏。
- 侧边栏默认跟随当前激活 pane，保证操作目标与键盘焦点一致。
- 所有 pane 地位对等，不区分主终端和子终端。
- 每个 pane 保持独立 PTY、焦点、选择、IME、滚动、重连和关闭生命周期。
- 文件管理器、服务器监控、搜索和 AI 上下文不得跨连接串台。

## 非目标

- 首版不持久化分屏树和 pane 比例。
- 首版支持把单 pane Terminal Tab 拖入另一个 Terminal Tab；已经包含分屏树的 Terminal Tab 不支持整体合并。
- 首版不复制整个分屏工作区；Duplicate Tab 复制当前激活终端。
- 不保留数据库、SFTP、Redis、MongoDB 或 Remote Desktop 的通用分屏入口。
- 不新增 Tmux、Shell multiplexer 或服务端分屏协议。

## 用户体验

### 页面布局

一个 Terminal Tab 由终端分屏区和唯一侧边栏组成：

```text
┌──────────────────────── Terminal Tab ─────────────────────────┐
│                                                               │
│  ┌──────────────── Split Workspace ───────────────┐ ┌──────┐ │
│  │ ┌────────────────┬───────────────────────────┐ │ │ 工具 │ │
│  │ │ 192.168.8.17   │ 192.168.8.151            │ │ │ 栏   │ │
│  │ ├────────────────┼───────────────────────────┤ │ ├──────┤ │
│  │ │ 192.168.8.152  │ 192.168.8.150            │ │ │ 共享 │ │
│  │ └────────────────┴───────────────────────────┘ │ │ 面板 │ │
│  └────────────────────────────────────────────────┘ └──────┘ │
└───────────────────────────────────────────────────────────────┘
```

工具面板移动到底部时仍作用于整个 Terminal Tab，而不是嵌入某个 pane。

### Pane 标题与状态

多 pane 状态下，每个 pane 右上角显示不占布局高度的浮动 tool，包含连接标题、取消分屏和关闭。标题可以拖回 Tab 栏，把该 pane 恢复成独立 Terminal Tab。单 pane 状态不显示分屏 tool 或 pane 边框。

active pane 使用主题色细边框表示键盘输入和侧边栏目标；所有 pane 的操作和生命周期完全一致。

### 分屏操作

分屏只通过拖动顶层 Terminal Tab 完成，不提供按钮式上下左右分屏。把单 pane Terminal Tab 拖到另一个 Terminal Tab 的任意 pane 后，整个 pane 都是有效 drop 区域，并根据光标距离最近的边缘自动显示左、右、上或下半屏 overlay。释放后，源 tab 的唯一终端会话移动到目标 workspace，并从顶层 Tab 栏移除。

浮动 tool 提供“取消分屏”，把当前 pane 作为后台 Tab 恢复到同一 TabContainer，并把焦点留在剩余 pane；也可以直接拖动浮动标题回 Tab 栏并激活恢复出的 Tab。任意 pane 均可关闭或取消分屏，但最后一个 pane 不能通过 pane 级操作移除。

### 侧边栏目标

侧边栏始终跟随 active pane，不提供 Pin 模式，也不增加 workspace 统一标题头。点击或通过键盘聚焦任意 pane 后，唯一侧边栏立即切换到该 pane 的连接上下文。

## 架构

### 顶层结构

`OnetCliApp` 直接持有普通 `TabContainer`：

```text
OnetCliApp
└── TabContainer
    ├── Home Tab
    ├── Database Tab
    ├── SFTP Tab
    └── TerminalView
        ├── TerminalSplitWorkspace
        └── TerminalSidebarHost
```

`TerminalWorkspace` 是唯一实现 `TabContent` 的终端 workspace。它管理对等 pane、分屏树、active pane 和共享侧边栏。

### `TerminalPane`

从现有 `TerminalView` 提取不包含侧边栏的 `TerminalPane`。它拥有单个终端会话的全部运行和渲染状态：

- `Entity<Terminal>` 和连接复制来源；
- Blink、字体、Canvas bounds 和 PTY resize；
- 键盘、鼠标、IME、选择和滚动；
- History Prompt、Shell 状态、MFA 和重连；
- Scrollbar、RenderCache 和 AddonManager；
- Public MCP 注册和 Broadcast client；
- pane 级关闭确认和事件订阅。

`TerminalPane` 不实现 `TabContent`，也不创建 `TerminalSidebar`、工具栏、AI Chat、文件管理器或 Server Monitor。

核心事件：

```rust
pub enum TerminalPaneEvent {
    Focused { pane_id: TerminalPaneId },
    TitleChanged { pane_id: TerminalPaneId },
    StateChanged { pane_id: TerminalPaneId },
    CloseRequested { pane_id: TerminalPaneId },
    SplitRequested {
        pane_id: TerminalPaneId,
        placement: TerminalSplitPlacement,
    },
}
```

### `TerminalView`

```rust
pub struct TerminalView {
    active_pane_id: TerminalPaneId,
    panes: HashMap<TerminalPaneId, Entity<TerminalPane>>,
    split_root: TerminalSplitNode,
    sidebar_host: Entity<TerminalSidebarHost>,
    pane_factory: TerminalPaneFactory,
    _subscriptions: Vec<Subscription>,
}
```

`active_pane_id` 同时决定键盘输入、Tab 标题、复制来源、连接标识和侧边栏目标。所有 pane 使用统一的创建、关闭、取消分屏和迁移 contract。

### 分屏树

```rust
pub enum TerminalSplitNode {
    Pane {
        pane_id: TerminalPaneId,
    },
    Group {
        split_id: TerminalSplitId,
        axis: Axis,
        children: Vec<TerminalSplitNode>,
        resize_state: Entity<ResizableState>,
    },
}
```

相同 axis 的相邻分屏合并到同一 Group，避免生成不必要的深层嵌套。不同 axis 才创建子 Group。`TerminalSplitId` 必须稳定，不能用树路径作为 keyed state ID，否则 pane 删除和树归一化会导致比例重置或串组。

渲染继续复用 `h_resizable`、`v_resizable` 和 `resizable_panel`。每个 pane 显式设置终端专用最小宽高，不修改通用 `PANEL_MIN_SIZE`。拖动改变 Canvas bounds 后继续由 `TerminalPane` 的 `resize_if_needed` 向 PTY 发送尺寸。

### `TerminalSidebarHost`

侧边栏属于 workspace，不由任何单个 pane 持有。它维护一套展示状态和按 pane 隔离的工具上下文，并始终展示 active pane 的连接上下文：

```rust
pub struct TerminalSidebarHost {
    presentation: TerminalSidebarPresentation,
    contexts: HashMap<TerminalPaneId, TerminalSidebarContext>,
}
```

展示状态包括位置、宽度、底部高度、可见性和当前工具。连接上下文包括 connection、SSH session、文件管理器、监控、搜索、Rich Input 和 AI Chat。文件管理器、监控和 AI 等重资源面板按 pane 懒加载，未打开时不创建。

作用域规则：

| 工具或行为 | 作用域 |
|---|---|
| Font、Theme、通用 Settings | workspace / 全局设置 |
| Quick Command、History、Rich Input | active pane |
| Search、Paste Code、Sync Path | active pane |
| File Manager、Server Monitor | active pane |
| AI Chat | active pane，pane 间保留独立会话 |
| Keyboard、IME、右键粘贴 | active 或被点击 pane |
| Broadcast Input | 当前 TerminalView 的全部 pane |
| Public MCP | 每个 pane 独立注册稳定 session ID |

## Pane 创建与迁移

首版通过拖入单 pane Terminal Tab 创建新 pane：从源 `TabContainer` 原子取出 tab，把源 `TerminalWorkspace` 的唯一 `TerminalView` 转移到目标 workspace，并重新建立 pane 事件订阅。

首版不支持把已有复杂分屏树直接拖入另一个 workspace。拖动源必须通过 split tree contract 确认为单 pane；复杂 workspace 继续保留顶层 tab 排序能力，但不会显示终端分屏 drop overlay。转移不触发关闭确认，因为 PTY 和连接实体仍由目标 workspace 持有；若目标插入失败，源 tab 必须恢复，禁止丢失会话。

## 关闭生命周期

关闭任意 pane 时先执行该 pane 的关闭保护；成功后从 `panes` 和 split tree 删除，归一化零/单子节点 Group，并把 active pane 切换到最近邻。最后一个 pane 只能通过关闭整个 Terminal Tab 释放。

关闭整个 Terminal Tab 时必须检查所有 pane。若存在运行中的任务，使用一次汇总确认；用户确认后统一关闭全部会话，取消则不关闭任何 pane，禁止逐个关闭后在中途取消造成半关闭状态。

取消分屏不关闭连接，而是把 pane 重新包装为单 pane `TerminalWorkspace` 和 `TabItem`。按钮取消以后台 Tab 插入并保留当前 workspace 焦点；拖动标题回 Tab 栏则激活恢复出的 Tab。

## 删除通用分屏

实现终端原生分屏后删除：

- `SplitTabContainer`、`SplitNode`、`SplitTabContainerEvent` 和 `TabPaneFactory`；
- `OnetCliApp::split_container`；
- `TabContainerEvent::SplitRequested` 和 `MoveToPrimaryRequested`；
- `split_enabled`、`is_primary_pane` 和 `will_split_placement`；
- `with_split_enabled`、`with_primary_pane`；
- Tab 内容边缘通用分屏 drop overlay 和通用分屏菜单；
- `TabContent::can_split`、`TabContentView::can_split` 及所有 crate 的实现；
- `tab_container_split_tests.rs` 和对应通用分屏本地化文本。

普通 Tab 拖动排序、跨 TabContainer 的非分屏行为和 sidebar contribution 能力不因本设计扩大范围。

## 模块组织

新增和调整后的终端模块建议为：

```text
crates/terminal_view/src/
├── view.rs                  # TerminalView 对外 TabContent 与协调
├── pane.rs                  # TerminalPane 状态与公共操作
├── pane/render.rs           # 终端画布渲染
├── pane/events.rs           # pane 事件与关闭 contract
├── split/model.rs           # 插入、删除、归一化
├── split/render.rs          # h/v resizable 渲染
├── split/actions.rs         # 分屏、关闭、平均分配
├── split/navigation.rs      # 相邻 pane 焦点导航
├── workspace.rs             # pane、树、sidebar 协调
└── sidebar/workspace_host.rs # 唯一侧边栏和按 pane context
```

现有 `view.rs` 已较大，新逻辑必须拆入上述小模块，不继续扩大单文件和单函数。

## 实施顺序

1. 从 `TerminalView` 提取 `TerminalPane`，以单叶节点保持当前单终端行为；
2. 增加 `TerminalSplitNode`、左右分屏和拖动 resize；
3. 增加上下分屏、对等 pane 删除、树归一化和 active pane 导航；
4. 把内部工具 dock 改为唯一 `TerminalSidebarHost`；
5. 实现侧边栏跟随 active pane，并隔离 pane 工具上下文；
6. 增加单 pane Terminal Tab 拖入现有 workspace 的四方向 drop；
7. 增加浮动标题拖回 Tab 栏和取消分屏恢复 Tab；
8. 改 `OnetCliApp` 直接持有 `TabContainer`；
9. 删除 `SplitTabContainer` 和 `TabContainer` 通用分屏 API；
10. 删除所有非终端 `can_split` 实现并完成回归验证。

## 测试策略

本功能属于高风险跨模块行为变更，实施时使用 TDD。

纯逻辑测试覆盖四方向插入、同轴合并、异轴嵌套、任意 pane 删除、最后 pane 保护、stable SplitId、最近邻焦点、单 pane 可转移 contract 和四方向 tab drop 落点。GPUI/结构测试覆盖单 pane 回归、2×2 渲染、拖动 resize、顶层 Terminal Tab 拖入 pane、复杂 workspace 拒绝合并、浮动标题、取消分屏、拖回 Tab 栏、每个 pane 独立输入/IME/滚动、唯一 toolbar 和 active pane 视觉。

连接测试覆盖不同 SSH pane 的文件管理器、监控和 AI 不串台；Quick Command、Rich Input、Search 和 Paste Code 只作用于 sidebar target；Broadcast 只覆盖当前 workspace；每个 pane 的 Public MCP session 可独立注册和释放。

关闭测试覆盖任意 pane 取消、任意 pane 成功关闭、最后 pane 保护、整个 Tab 汇总取消零关闭和确认后全部关闭。

## 风险与取舍

- 从大型 `TerminalView` 提取 `TerminalPane` 容易遗漏 IME、MFA、Addon、Public MCP 或关闭逻辑，必须先保持单 pane 行为等价再增加分屏。
- Tab 标题、连接标识、复制来源和侧边栏都依赖 `active_pane`，焦点变化必须向 `TabContainer` 发出状态事件。
- 每个 pane 的连接级工具状态需要保留，但重资源面板必须懒加载，避免创建多个隐藏 AI、文件管理器和监控实体。
- 关闭整个 workspace 需要汇总确认，不能复用逐 pane 顺序关闭造成部分成功。
- 删除通用分屏会移除非终端页面的现有分屏入口，这是明确的产品取舍。
- 首版不持久化布局，降低状态恢复和连接重建风险；稳定后再设计序列化版本。

## 验收标准

- `OnetCliApp` 和 `TabContainer` 不再依赖 `SplitTabContainer`；
- 只有 Terminal Tab 支持分屏，所有 Tab 右键菜单均可查看支持范围和操作说明；
- 一个 Terminal Tab 可以建立 2×2 Local/SSH 混合分屏；
- 单 pane Terminal Tab 可拖到另一个终端 pane 并按最近边缘转移为对等 pane，复杂 workspace 不允许整体拖入；
- 分隔线可拖动，所有 PTY 收到正确尺寸且界面无残留；
- 整个 Terminal Tab 只渲染一套工具栏和侧边栏；
- 默认点击或键盘聚焦 pane 后，侧边栏目标跟随该 pane；
- 多 pane 时浮动 tool 显示标题、取消分屏和关闭，单 pane 时不显示分屏 chrome；
- 所有 pane 地位对等，任意 pane 可取消分屏回 Tab 或独立关闭，最后 pane 受保护；
- 文件管理器、监控、搜索、命令和 AI 不跨 pane 串台；
- 取消分屏按钮把 pane 作为后台 Tab 恢复，拖动浮动标题回 Tab 栏会激活恢复出的 Tab；
- 取消关闭整个 Tab 时所有 pane 保持，确认后全部释放；
- 非终端 Tab 不再实现或展示通用分屏；
- 相关单元测试、GPUI 测试、`cargo check`、Clippy 和格式检查通过。
