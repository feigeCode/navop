# Navop 首页配色与侧栏精修：开发交接文档

> 审阅日期：2026-09-06  
> 目标：在现有主页布局上做视觉收敛，不重写主页，不更换品牌色。  
> 状态：**源码审阅后的建议方案；尚未实施、尚未做运行时视觉验收。**

## 1. 先读结论

**需要调整配色，但不是重新设计一套主题。保留 Navop 蓝 `#3B82F6`，重点修复侧栏的灰阶层次、选中态、功能图标和账户区。**

现在“不够好看”不是单一颜色造成的：

1. 侧栏、主画布都非常浅，区域层次不够明确。
2. 侧栏选中背景是中性灰，选中文字却从另一组 token 回退到深蓝，配对不够主动。
3. “连接”的浅蓝房子、“会话日志”的浅紫终端来自彩色 SVG；代码注释说单色，不代表实际按单色渲染。
4. 侧栏菜单、账户行和底部设置区的缩进、悬停方式不完全一致，选中块又比较贴边。
5. 黄色字母头像来自 Avatar 自动生成色，与品牌主色无关。
6. 卡片普通悬停使用选中边框色，而且无条件 hover 样式存在覆盖 selected 背景的风险。

期望效果是：**略深的中性侧栏 + 浅色内容画布 + 白色卡片 + 少量浅蓝选中态**。功能入口安静一致，连接类型保留品牌辨识色。借鉴参考界面的层次和一致性，不照搬它的深色背景、大 Logo 或大尺寸按钮。

---

## 2. 工作目录与实施边界

### 2.1 实际工作树

用户提到的开发目录，本次定位为：

```text
navop/.worktrees/unified-home-layout
branch: feat/unified-home-layout
HEAD: 41c2ae07fa26b979906cffec8cfb857e3aed611e
```

下文路径均相对于这个 worktree。行号为审阅时定位辅助，**以符号名和当前工作区文件为准**。

该工作树存在大量未提交/未跟踪文件，包含正在开发的首页、主题设置、TabContainer 和其他业务模块。HEAD 不是当前界面的完整快照。

实施前必须：

- 阅读当前 `AGENTS.md`、相关 GPUI / gpui-component 技能与规范。
- 查看 `git status` 和目标文件 diff，保留已有改动。
- 只修改这份文档列出的视觉范围，不执行 reset/checkout 回退用户代码。
- 不在父仓库、其他 worktree 或系统安装版应用上验证后声称本工作树通过。
- 不修改 Cargo git checkout/cache 中的依赖源码，不为本次美化升级依赖。

### 2.2 和已有设计稿的关系

已有 `home-layout-demo/DESIGN.md`、`design-docs/navop-home-redesign/README.md` 以及本仓库的 UI 优化规范可提供背景；**本次精修以当前源码和最近截图为基线**。

以下旧稿或早期建议不要重新引入：

- 最近连接的金色星标、逐卡“最近”标签；
- 亮蓝团队角标；
- 单个分组中的卡片自动拉伸占满一整行；
- 侧栏中段/边缘悬浮的折叠按钮；
- 常驻用户邮箱；
- 给普通“新建连接”恢复强强调填充色；
- 卡片阴影、悬停抬升；
- 将早期示例蓝 `#4F75D8` 当成项目品牌色；
- 旧 Start Center、全局 Rail 改造与本次首页侧栏精修混做。

---

## 3. 实际主题：不是凭截图猜颜色

### 3.1 Navop 内置色值

来源：`themes/navop.json` 的 `Navop Light` / `Navop Dark`。

| 角色 | JSON token | Light 当前值 | Dark 当前值 |
| --- | --- | --- | --- |
| 基础表面 | `background` | `#FFFFFF` | `#171717` |
| 主文字 | `foreground` | `#333333` | `#E5E5E5` |
| 通用边界 | `border` | `#E5E7EB` | `#404040` |
| 品牌主色 | `primary.background` | `#3B82F6` | `#3B82F6` |
| 主色 hover | `primary.hover.background` | `#2563EB` | `#2563EB` |
| 主色 pressed | `primary.active.background` | `#1D4ED8` | `#1D4ED8` |
| 强调背景 | `accent.background` | `#E0F2FE` | `#1E3A8A` |
| 强调前景 | `accent.foreground` | `#1E3A8A` | `#BFDBFE` |
| 次级背景 | `secondary.background` | `#F3F4F6` | `#262626` |
| 次级文字 | `secondary.foreground` | `#4B5563` | `#E5E5E5` |
| 弱化背景 | `muted.background` | `#F9FAFB` | `#1F1F1F` |
| 弱化文字 | `muted.foreground` | `#6B7280` | `#A3A3A3` |
| 侧栏背景 | `sidebar.background` | `#FAFAFA` | `#1E1E1E` |
| 侧栏边界 | `sidebar.border` | `#E5E5E5` | `#333333` |
| 侧栏文字 | `sidebar.foreground` | `#171717` | `#E5E5E5` |
| 侧栏选中背景 | `sidebar.accent.background` | `#E5E5E5` | `#2D4A6F` |
| 侧栏选中文字 | `sidebar.accent.foreground` | **未显式配置** | **未显式配置** |
| 列表选中背景 | `list.active.background` | `#3B82F610` | `#3B82F620` |
| 列表选中边框 | `list.active.border` | `#3B82F6` | `#3B82F6` |
| 列表 hover | `list.hover.background` | `#F3F4F6` | `#262626` |
| Tab 背景 | `tab.background` | `#F3F4F6` | `#171717` |
| 活跃 Tab 背景 | `tab.active.background` | `#FFFFFF` | `#262626` |

八位颜色的尾两位是透明度；`#3B82F610` 不是不透明浅蓝。最终观感还取决于承载背景和组件的 token 解析方式。

### 3.2 运行时覆盖链

源码入口：`crates/core/src/themes.rs`：

- `apply_appearance`（约 146 行）：解析模式和主题，调用 `Theme::change`，再应用自定义强调色。
- `resolve_theme_pair`（约 163 行）：优先用户选择的 Light/Dark 主题，再回退 Navop 内置主题、组件默认主题。
- `apply_custom_accent`（约 182 行）：覆盖 `primary`、`button_primary`、`accent`、`ring`、若干 selected border、`sidebar_primary`、`selection` 等。
- **它不覆盖 `sidebar_accent`、`sidebar_accent_foreground`、`list_active` 背景。**

因此：

1. 上表是内置 JSON，不是对用户当前运行配置的读取结果。
2. 不能将 `theme.accent` 一概理解为“浅蓝背景”：开启自定义强调色后，它可能是饱和红/紫，前景也会被改成白/黑。
3. 本轮保留现有自定义强调色作用范围，不顺便改主题设置语义；侧栏继续使用主题自身的选中配色，不承诺跟随自定义强调色变色。
4. “让所有选中背景跟随自定义强调色”需要单独设计、测试和交付；不要只改背景不配前景。

### 3.3 组件版本与回退事实

当前 `Cargo.toml:52–60` 的 `gpui-component` 实际 package 为 `gpui_ce_components`，git rev 为 `65bc4ab5`。不要照网络上其他版本的 API 编写。

该版本依赖源码：

- `crates/ui/src/theme/schema.rs:995`：
  `sidebar_accent_foreground` 缺省时回退到 `accent_foreground`。
- `crates/ui/src/sidebar/menu.rs:264–307`：
  active 使用 `sidebar_accent` / `sidebar_accent_foreground`；
  非 active hover 使用 `sidebar_accent.opacity(0.8)` / 同一个前景。

所以当前内置浅色主题侧栏的选中组合是**灰底 + 深蓝文字**，不是缺少选中文字，也不是回退到普通黑色。补显式字段的价值是明确配对、减少耦合。

---

## 4. 当前页面是怎样使用颜色的

| 区域 / 符号 | 当前实现 | 对精修的影响 |
| --- | --- | --- |
| `home_layout.rs::render_home_layout` | 内容画布 `theme.muted`，外围 `theme.background` | 画布已经正确分层，不需要全部重涂 |
| `sidebar.rs::render_sidebar` | `theme.sidebar`，右边线却用 `theme.border` | 修改 `sidebar.border` 不会自动作用到此处 |
| `sidebar_navigation.rs::render_application_navigation` | `SidebarMenuItem`；连接 `.active(true)` | 这是首页启动入口，不是全局页面路由状态 |
| 同上 | 图标 `Icon::new(...)`，18px | 默认模式可能是 Color，注释不能约束 SVG |
| `account_menu.rs::AccountTrigger` | 27px Avatar，账户 hover 用 `theme.muted` | 自动色头像和菜单 hover 与导航不一致 |
| `toolbar.rs::render_toolbar` | 白色基础表面；搜索 `theme.muted` | 保留目前主次，不增加大色块 |
| `connection_card.rs::render_connection_card` | 卡片 `theme.background`；选中 `list_active` + `list_active_border` | 不要假设它用 `popover` 或不存在的 `card` token |
| 同上 | 普通 hover 也用 `list_active_border`；无条件 `.hover` 覆盖背景 | hover/selected 区分不足，应检查优先级 |
| `connection_badge.rs::render_team_badge` | 中性 muted 系配色 | 保留，不改成蓝色强调 |
| `crates/core/src/tab_container.rs`（约 3917 行） | Tab 栏默认读 `theme.tab`；活跃边框 `primary.opacity(0.85)` | 只修改 `tab_bar.background` 无法控制这条渲染路径 |

---

## 5. 推荐色彩方案：小范围改动，不整体换肤

### 5.1 目标层次

浅色模式：

```text
侧栏：#F3F4F6       比主画布略深，形成稳定的导航区域
主画布：#F9FAFB     保留
卡片/工具栏：#FFFFFF 保留
普通文字：中性深灰   不让所有导航文字接近纯黑
选中导航：浅蓝底 + 深蓝字 / 图标
品牌蓝：#3B82F6     保留，只用于确实有强调意义的状态
```

不要同时增加强分割线、侧边彩条、厚蓝边框、重阴影。选中项使用**一层浅蓝底 + medium 字重 + 同色功能图标**即可。

### 5.2 内置主题建议补丁

以下是**建议值，不是已经修改的值**。颜色字面量只进入主题文件，不散落在页面 `.bg()` / `.text_color()` 中。

| 主题 | 字段 | 当前 | 建议 |
| --- | --- | --- | --- |
| Light | `sidebar.background` | `#FAFAFA` | `#F3F4F6` |
| Light | `sidebar.foreground` | `#171717` | `#4B5563` |
| Light | `sidebar.border` | `#E5E5E5` | `#E5E7EB` |
| Light | `sidebar.accent.background` | `#E5E5E5` | `#E8F0FE` |
| Light | `sidebar.accent.foreground` | 缺省，回退 `#1E3A8A` | 显式 `#1E3A8A` |
| Dark | `sidebar.accent.foreground` | 缺省，回退 `#BFDBFE` | 显式 `#BFDBFE` |

Dark 第一轮保留现有侧栏背景 `#1E1E1E`、选中背景 `#2D4A6F`、文字和边界；不要机械套用浅色灰阶，也不要直接复制参考图的深色调色板。

保留 `primary.*`、`accent.*`、`background`、`muted.*`、状态色、编辑器高亮与终端独立主题。

### 5.3 重要：这些不是“首页私有 token”

修改 `themes/navop.json` 的 sidebar token 会影响其他消费者。已确认：

```text
main/src/persistent_connection_sidebar/mod.rs
SidebarPalette::app（约 52–69 行）
  foreground = theme.sidebar_foreground
  hover      = theme.sidebar_accent
  muted      = theme.sidebar_accent
  border     = theme.sidebar_border
```

将 `sidebar_accent` 改浅蓝后，持久连接树的 hover 也可能变蓝。**必须把“首页组件精修”和“内置主题补丁”拆成可独立评审的步骤。**

- 先完成第 6 节局部修正，再应用主题补丁并做跨页面核验。
- 扫描所有 sidebar token 消费点，至少检查持久连接树、设置页和一个实际工作 Tab。
- 若连接树不应随之改变，可在其 app palette 中将普通 hover 改为现有中性语义色；不要改 `From<&TerminalColors>` 的终端调色板分支。
- 无法验证跨页面影响时，保留原主题值，仅交付局部修正，并明确主题补丁尚未验收。
- 第三方导入主题不被覆写；首页读取其主题语义色，不按 Light/Dark 强制注入上述蓝色。

---

## 6. 侧栏具体修改要求

### 6.1 先修图标，而不只是给图标换色

已确认：

- 依赖 `assets/icons/home.svg` 有固定填充 `#A3D2FF`。
- 依赖 `assets/icons/terminal.svg` 有固定填充 `#C7D2F1`。
- `Icon::new` 通过生成的图标元数据选择模式；该版本测试明确断言 `IconName::Terminal` 默认 Color。
- `main/src/navigation_applications.rs::icon` 将 `SessionLogs` 映射到 `Terminal`。
- `home_layout.rs::TabContent::icon` 还显式使用 `IconName::Home.color()`。

实施要求：

1. 首页功能导航统一单色线性风格，默认继承普通/选中前景，尺寸统一。
2. 会话日志优先改用 `IconName::SquareTerminal` 线性资源；当前依赖的 Sidebar story 已有该枚举用例。依赖也包含 `terminal_line.svg`，如选它则先核对生成枚举和 metadata，不凭文件名猜 API。
3. Home：`.mono()` 只能解决染色，**不能把实心房子变成线稿**。先查已有资源；没有合适资源时，在 Navop 自有 `resources/icons/` 增加一枚简单线性首页图标，经现有资源加载路径注册。不要修改依赖缓存或批量替换全局 Home 资源。
4. 首页侧栏与首页 Tab 的房子建议保持同一图形语言；Tab 路径作为随后的小范围一致性修正。
5. `navigation_applications.rs` 是共享注册表，改它之前核对 Toolbox 等调用方。只适用于侧栏的变化放在导航渲染映射，不无意改变共享身份图标。
6. 数据库、Linux、Redis 等连接身份图标保留原色，不做全局 `.mono()`。
7. 使用 `FunctionalIcon` 前确认 metadata 分类，避免将不兼容的图标交给受约束构造器导致 panic。

### 6.2 导航状态应明确区分

| 状态 | 背景 | 文字 / 图标 | 其他 |
| --- | --- | --- | --- |
| 默认 | 透明，露出 `sidebar` | `sidebar_foreground` | 普通字重 |
| hover，非选中 | 中性轻背景 | `sidebar_foreground` | 不伪装成选中项 |
| 选中 | `sidebar_accent` | `sidebar_accent_foreground` | medium 字重 |
| 选中 + hover | 保留选中组合 | 保留选中前景 | 不被普通 hover 覆盖 |
| 键盘焦点 | 保留底层状态 | 保留对应前景 | 清晰的焦点边界 |
| disabled（若有） | 不响应 hover | 弱化但可辨认 | 不触发事件 |

推荐把本页所需颜色集中到一个**轻量的本地 palette/helper**：语义值从 `cx.theme()` 读取；普通 hover 可由 `sidebar` 与低透明度 `sidebar_foreground` 混合得到，建议以 6% 前景作为视觉起点，检查 Dark 与导入主题，不采用固定白色蒙层。

注意实现限制：

- 当前 `SidebarMenuItem` 没有可直接覆盖内部 active/hover 的样式字段；外层 `.bg()` 不会可靠覆盖内部 `h_flex`。
- 不要写不存在的 `.active_bg(...)`、`.hover_bg(...)` API。
- 如需拆分 hover/selected，可在首页内建立一个小型导航行渲染 helper，保留原有点击、tooltip、折叠和禁用语义；不要抽成全仓导航框架。
- 若改用自定义行，必须显式处理可聚焦、键盘触发、可访问名称和命中区，不能只有 `.on_click`。
- `apply_appearance` 当前设置 `focus_ring = false`。不要为侧栏开启全局外扩 ring；沿用/实现清晰焦点边界并实测。

首页可见时，“连接”保持 active 是现有设计。点击 AI、笔记、团队等打开/复用对应 Tab；不要为了“切换高亮”更改原有 Tab 模型，或让点击“连接”重置搜索、分组和滚动。

### 6.3 几何：给选中块一点呼吸空间

当前实际展开宽度 `184px`、折叠宽度 `58px`，来源 `main/src/home_tab.rs:98–102`。截图是缩放后的图像，不可把图片测得的宽度直接写成 GPUI 数值。

建议第一轮：

- **保留 184/58 的宽度**，先看配色和对齐收益，不改成参考图那样的大侧栏。
- 导航区和 footer 的外侧水平内缩由当前 `.p_1()` 所体现的紧贴边缘状态收敛到同一规则；展开态建议从约 8 个逻辑像素起步。
- 折叠态单独检查，不照搬展开态 padding 导致可点击区域过窄。
- 单行目标高度约 32–36 个逻辑像素，以项目已有 geometry/control token 为实现入口；图标建议选现有 16/20px 档中的同一档，不混用多个裸 18px。
- 图标与文字约 8px 间隔，菜单行间隙约 4px。使用既有 spacing/size API，避免新增相近尺寸。
- 主导航、设置行的图标中心线和文字左边线一致。
- 保留现有“常用入口 / 工具入口”分隔，不新增三四个分类标题。
- 侧栏右边线改为读取 `sidebar_border`，保持轻边界，不追加阴影。
- 中间空白可以保留，不用欢迎卡片、推广入口或大 Logo 填满。

以上数值是候选起点，不是截图验收结果。中文、英文、长标签、125%/150% 缩放后不截断重要入口；字号优先维持当前 `text_sm`，不要简单整栏加大。

### 6.4 账户与设置区域

保留现有布局：账户固定底部，常驻只显示用户名，邮箱留在菜单；展开态“设置 + 折叠”同一行，折叠态开关单独居中。

Avatar 问题的真实原因：

```text
gpui-component / crates/ui/src/avatar/avatar.rs::render
  hash(short_name) 决定 hue
  fallback 用这个颜色的 20% 透明背景 + 同色文字
```

建议：

- 有真实头像：保留图片，不 tint。
- 没有头像/未登录：使用本页统一的中性 fallback，背景 `secondary`，前景 `secondary_foreground`；可用单色用户图标，或正确处理 Unicode 的首字母。
- 当前 Avatar 内部给 fallback 自行着色，**不能保证在外层加 `.bg()` / `.text_color()` 就能覆盖它**。采用局部可控 fallback helper 或已有基础 Avatar fallback API，不改全局随机色算法。
- 账户行和弹出菜单头部共用 fallback 策略，避免点击后头像突然换色。
- 用户名从 semibold 收敛到与导航协调的 medium；保留单行省略和完整信息入口。
- hover 使用与本页导航协调的中性背景；检查账户 Popover 打开时的持续状态。`AccountTrigger` 已有 `selected` 字段但当前 `into_element` 未用其显示状态；锁定依赖 `popover.rs:175` 会调用 `trigger.selected(selected || is_open)`，应消费这一状态，而不是新建一套菜单 open 状态。
- 底部 toggle 保持稳定 ID `home-sidebar-toggle` 与 tooltip，不移回侧栏边缘。

---

## 7. 主内容区：保留成果，只改明显的状态问题

### 7.1 必须保留

- 所有分组共享网格列边界，单条分组不拉伸。
- `grid.rs::card_grid_metrics` 的 rem 缩放与列数计算；不要强制永远四列。
- 最近连接更紧凑，使用历史图标；卡片没有重复“最近”标签。
- 最近区与普通区使用不同元素 ID 命名空间，同一连接重复出现不冲突。
- 团队标签中性化，名称/地址截断逻辑不退化。
- 工具栏与内容区左边线对齐；搜索优先；新建连接/终端保持普通操作层级。
- 展开全部/折叠全部留在分组菜单。
- 卡片单击选中，双击打开；上下文菜单和 hover 操作不变。
- 首页与全局连接树的筛选状态独立，最近连接跨分组的既有逻辑不变。

### 7.2 卡片 hover / selected 修正

位置：`main/src/home_tab/connection_card.rs:32–62`。

当前默认 hover 也使用 `list_active_border`，选中背景设置后又追加无条件 `.hover(...)`。这构成状态覆盖风险，需通过运行时验证确认修复。

期望：

1. 非选中 hover 使用轻微中性背景和较弱边界，不给每一张悬停卡片完整品牌蓝边。
2. selected 始终保留 `list_active` + `list_active_border`。
3. selected + hover 不能回到普通 hover 背景。
4. keyboard focus 与 selected 可区分；活动连接的小圆点仍表达连接状态，而非选中状态。
5. 不引入阴影、动画位移或更多常驻按钮。

本轮不统一修改全局 `list_active`，不重新计算最近连接排序、分组数量，不重排卡片信息。

### 7.3 本轮暂不做

Tab 栏全局配色重构、整套主题系统重写、右侧详情抽屉、侧栏工作区树、欢迎页、仪表盘统计、功能入口重分组、新增主题选择器。

分组间距若仍显松散，留到配色/侧栏前后对比后再单独调整，不把几个单连接分组的留白误判成网格错误。

---

## 8. 建议实施顺序与文件清单

### 阶段 A：局部精修，优先交付

| 文件 | 修改内容 |
| --- | --- |
| `main/src/home_tab/sidebar_navigation.rs` | 功能图标、统一导航行、hover/selected 区分、设置行对齐 |
| `main/src/home_tab/sidebar.rs` | 统一内缩、正确使用侧栏边界/前景，保留滚动与底部锚定 |
| `main/src/home_tab/account_menu.rs` | 中性 fallback、账户状态、文字重量、与导航一致的 hover |
| `main/src/home_tab/connection_card.rs` | 修复 hover/selected 优先级，减弱普通 hover |
| `main/src/home_tab/tests/rendering.rs` 及必要的定向测试 | 状态、ID、布局与行为防回归 |
| `main/src/home_tab/home_layout.rs`（按需） | 仅首页 Tab 功能图标一致性，不重写布局 |
| `resources/icons/`（确无可复用资源时） | 自有线性 Home 资源与注册，不复制参考产品素材 |

### 阶段 B：内置侧栏主题微调

| 文件 | 修改内容 |
| --- | --- |
| `themes/navop.json` | 应用第 5.2 节建议字段，其他主题值不动 |
| `main/src/persistent_connection_sidebar/mod.rs`（仅确有副作用时） | 隔离 app palette 普通 hover，保留终端调色板分支 |
| `crates/core/src/themes.rs` 的测试部分（按需） | 验证显式 sidebar 前景、主题切换/回退不退化；不扩大 custom accent 覆盖面 |

`crates/core/src/tab_container.rs`、业务状态、连接协议和数据库模块默认不在修改范围。

每阶段单独给出 diff 与截图，方便只撤回本轮变更；不要用回滚整个 dirty 文件的方式撤销。

---

## 9. 验收要求

### 9.1 代码与测试

在目标 worktree 内运行；以下是**交给实施模型的待执行命令，本次文档审阅没有运行它们**：

```bash
rtk cargo check -p main
rtk cargo test -p main home_tab::
rtk cargo test -p main navigation_applications::
# 如果修改了主题或持久连接树：
rtk cargo test -p one-core themes::
rtk cargo test -p main persistent_connection_sidebar::
```

按仓库格式检查规则检查本轮改动，不对整个 dirty workspace 批量格式化。

保留并运行已有 contract，尤其：

- `home_redesign_layout_contracts`
- `group_expand_commands_live_in_a_menu`
- `connection_hover_actions_have_stable_ids`
- `grid::tests` 的列宽、窄窗口、rem 缩放
- `recent::tests` 的排序与先筛选后截断

新增测试至少覆盖：

- 非选中 hover 与 selected 使用不同状态路径；selected + hover 保留选中。
- 导航功能图标走预期单色/线性资源，而连接身份图标不被影响。
- 账户无图片 fallback 在 Light/Dark 下使用成对语义色。
- 主题切换、主题名回退、自定义强调色不重置/误改用户设置。
- 折叠态 tooltip、入口触发、展开后滚动/搜索状态保留。

源码字符串 contract 不能证明实际颜色、布局、键盘可达性；需要真实窗口检查。

### 9.2 视觉矩阵

最少：

- Navop Light、Navop Dark。
- 自定义强调色开启/关闭；再选一种非蓝色强调色。
- 一种第三方导入主题（若测试环境无可用主题，明确记录未验证）。
- 侧栏展开/折叠。
- 1280×800、1440×900，以及较宽窗口；正常缩放和较大缩放。
- 短用户名、长中文/英文用户名、有头像/无头像、登录/未登录。
- 普通、hover、selected、selected + hover、键盘焦点、账户菜单打开。
- 少量连接、多分组长列表、搜索无结果。

对比时固定窗口逻辑尺寸、缩放、主题与数据。记录实际 executable 路径和源码基线，不能只记录“Navop 窗口”。

### 9.3 通过标准

- [ ] 左侧区域更明确，但没有变成沉重的色块。
- [ ] 默认功能图标统一、清晰，没有浅蓝 Home 和浅紫 Terminal 混在深灰线稿中。
- [ ] 导航文字、图标中心线与设置区对齐。
- [ ] 选中项有足够内缩，不紧贴窗口边缘。
- [ ] hover 与选中清楚区分；选中经过 hover 不丢失。
- [ ] 账户 fallback 不再出现低对比度亮黄字，真实头像不变。
- [ ] 主画布、白卡片、团队标签维持现有克制层次。
- [ ] 新建连接没有无理由变成最抢眼的蓝色大按钮。
- [ ] Dark 与导入主题没有固定白底、黑图标消失或前背景错配。
- [ ] 自定义强调色作用范围保持一致，不把饱和 `accent` 误当浅色导航背景。
- [ ] Tab、键盘、筛选、分组折叠、连接打开、上下文菜单没有行为回归。
- [ ] 内置主题补丁对持久连接树等消费者的影响已检查，或明确不交付该阶段。

---

## 10. 可直接粘贴给开发模型的任务

```text
请在 navop/.worktrees/unified-home-layout 中，按
docs/design/home-theme-sidebar-refinement.md 实施首页配色与侧栏精修。

先读项目 AGENTS.md、GPUI/gpui-component 规范和当前工作区 diff。
该目录有大量未提交改动，请保留现有工作，只做 scoped edits。

目标不是重写主页，也不是换品牌色：保留 #3B82F6，
保留现有共享网格、紧凑最近连接、中性团队标签、账户底部布局，
以及普通新建/终端按钮。

先完成阶段 A：统一首页功能导航图标，处理 hover/selected/focus，
改善侧栏内缩和底部对齐，使用中性账户 fallback，
修复卡片 selected 被 hover 覆盖的风险。
再完成阶段 B：按文档微调 Navop 内置 sidebar token，
并核验持久连接树等全局消费者；不能验证时明确保留该阶段。

注意：
1. 当前 Icon::new(Home/Terminal) 可能保留资源原色；mono 不等于线稿。
2. SidebarMenuItem 内部 active/hover 不能靠外层 bg 可靠覆盖。
3. Avatar 内部 fallback 自动着色，外层 bg/text_color 不一定有效。
4. theme.accent 会被用户自定义强调色覆盖，不保证是浅色背景。
5. 不修改 Cargo 缓存、不升级依赖、不改连接业务和 Tab 状态模型。
6. 不将截图像素直接作为 GPUI 逻辑尺寸。

完成后给出文件清单、实际 token 变化、自动测试结果、
正确 worktree 构建的 Light/Dark 展开/折叠前后截图、
自定义强调色和受影响页面的检查记录，以及未验证项。
不能以源码测试代替真实窗口视觉验收。
```

## 11. 本次文档交付的验证边界

已完成：核对开发工作树、内置主题、运行时强调色覆盖、首页/侧栏/账户/卡片实现、锁定版本组件的回退和渲染机制、相关设计规范与已有测试入口。

未执行：应用代码修改、编译、测试、启动当前 worktree 应用、真实键盘操作、Light/Dark 运行时截图对比。

本文件中的建议色值和尺寸需要由实施后的真实窗口验收确认，不能视为已经完成的产品变更。
