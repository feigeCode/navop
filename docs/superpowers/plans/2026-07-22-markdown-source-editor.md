# Markdown 源码保真编辑器实施记录

## 目标

在 Navop 内建立原生 Markdown 源码保真内核，使 Markdown 源码成为唯一权威数据。未被用户编辑的字节必须保持不变，富文本投影只能通过经过验证的局部补丁写回。

## 架构决策

- 新建无 UI 依赖的 `crates/markdown-source` crate。
- 核心 crate 不依赖 GPUI、Notes 或 Workspace Explorer。
- 使用 `markdown-rs` 的带字节位置 MDAST 建立块级与行内 Source Map。
- 新建 `crates/markdown-editor` GPUI crate，作为 Markdown 文件独立编辑器。
- Notes 已收敛为纯 Markdown 工作区，不再创建、扫描或打开 `.cditor.json`，也不再依赖 Cditor。
- 原 Notes 快捷键设置入口保留，并改为绑定 `markdown-editor` 原生 action；已有粗体、斜体、下划线、删除线、行内代码、标题、列表、引用、代码块、块移动、复制、删除和 Undo/Redo。
- 候选结果按语义 fingerprint 与原源码块对齐。语义未变化的节点直接复用原始源码，真正变化的节点生成 `SourceTransaction`。
- 未知 directive、HTML 和 frontmatter 显式标记为源码编辑节点，不允许静默转换。

## 2026-07-23 用户反馈统一验收清单

本节是当前阶段的唯一执行清单。后续实现、视觉调整和验证必须逐项回填，
不能再只针对单张截图做彼此独立的局部修补。

状态说明：

- `[x]`：代码已实现，并已有直接回归保护；仍需参加最终完整门禁。
- `[~]`：已有部分实现，但仍存在明确边界或缺少视觉/交互验收。
- `[ ]`：尚未完成。

### A. 产品边界与源码保真

- [x] 只保留原生 Markdown 编辑链路；Notes 不再创建、扫描或打开
  `.cditor.json`，不再依赖 Cditor。
- [x] Notes、Workspace Explorer 和系统打开 `.md` 文件共享同一个
  `MarkdownEditor`，不维护第二套富文本编辑状态。
- [x] Markdown 源码始终是唯一权威数据；未编辑节点、未知语法、HTML、
  frontmatter 和自定义 directive 必须原样保留。
- [x] Source 与所见即所得模式共用同一份源码事务和 Undo/Redo 历史。
- [x] 原 Notes Markdown 快捷键保留，并绑定到原生 Markdown action。

### B. Typora 式原位编辑稳定性

- [~] 点击进入编辑态后，块宽高、内容起点、字体族、字号、字重、行高、
  基线和换行位置必须与预览态一致；当前正文与长列表已统一为 16/24px，
  Heading 仍按层级字号渲染，需要继续做全类型视觉验收。
- [x] 鼠标激活当前可见块时不得自动将块滚动到文档中央；程序化跳转到远端块
  仍可显式滚动。
- [x] Markdown 内嵌 Input 不显示自己的滚动条，也不保留会改变换行点的
  末尾布局安全边距。
- [~] 正文、Heading、引用、无序列表、有序列表和任务列表必须共用一致的
  预览/编辑布局度量；当前文字起点和长行换行已基本一致，任务列表编辑态
  checkbox 尚未完成绘制验收。
- [x] 行内粗体、斜体、删除线、行内代码、链接和数学公式的源码 marker
  必须以不占正文布局宽度的 overlay/inlay 显示；光标进入节点时不能因为
  `**`、反引号、`$` 等 marker 触发临界行重新换行。
- [x] 块级代码继续使用 Input 编辑，并保持多行、换行与 Tab/Shift-Tab 缩进。
- [ ] 建立覆盖字体度量、块 bounds、后续块 y 坐标、换行点和外层滚动偏移的
  统一视觉回归，确保“进入编辑态只多出光标和必要源码标记”。

### C. 列表编辑

- [~] 无序列表 bullet、任务列表 checkbox 与正文使用固定 marker gutter；
  当前预览态正常、编辑态普通 bullet 正常，编辑态每一行 task checkbox
  仍需修复并截图确认。
- [x] 同级有序列表即使源码每行都写 `1.`，编辑态也显示连续编号；支持自定义
  起始值和 `.` / `)` 分隔符。
- [x] 嵌套有序列表按缩进层级维护独立计数器；进入和退出子列表时不能串号，
  每一级保留自己的起始值和分隔符。
- [x] 列表点击激活不改变文档滚动位置，Input 不出现内部滚动条。
- [x] 列表 Enter 延续正确 marker 和下一编号，Shift-Enter 插入普通换行。

### D. 预览能力不能回退

- [x] 行内数学公式保持渲染；激活当前公式时只显示该公式源码，其余公式继续
  保持渲染。
- [x] 块级数学公式与 Mermaid 保持异步 SVG 渲染，点击后可进入源码编辑。
- [x] 图片及链接图片保持预览、alt/path 编辑和完整 wrapper 删除语义。
- [x] 代码块保持高亮预览与 Input 源码编辑。
- [x] 表格保持结构化网格预览，不显示 delimiter row。
- [ ] 最终回归必须同时覆盖数学公式、Mermaid、图片、代码块和表格，不能以
  修复排版为理由降级或删除任一渲染能力。

### D1. 公式与 Mermaid 扩展加载性能

- [ ] 数学公式和 Mermaid 的扩展发现、WASM/渲染器初始化、编译和 SVG 生成
  不得在 GPUI UI 线程同步执行。
- [ ] 打开文档、首次滚动到公式/Mermaid、切换活动块和输入时，UI 线程必须
  保持可响应；前台只负责提交任务、展示 loading/fallback 和应用结果。
- [ ] 相同扩展、相同源码和相同渲染参数要去重并缓存，不能因虚拟列表重绘或
  光标移动重复初始化扩展。
- [ ] 异步结果必须带请求版本/源码 fingerprint；旧任务晚到时不能覆盖新源码
  或新主题下的结果。
- [ ] 渲染失败必须保留可读源码，提供明确的失败状态和再次触发渲染的路径，
  不能阻塞编辑器或让块消失。
- [ ] 增加慢渲染器 contract：后台任务延迟期间仍可点击、输入、滚动和切换
  活动块；完成后只更新对应块。
- [ ] 需要运行公式/Mermaid 相关测试、虚拟列表可见性测试和主工程启动检查，
  确认扩展加载不再卡住 UI 线程。

### E. Typora 风格表格编辑

- [x] 表格激活前后预留相同工具条空间，表格 bounds、单元格内容位置和后续块
  y 坐标不变化。
- [x] 鼠标点击单元格时根据真实 Input 布局定位光标；程序化激活默认将光标放到
  内容末尾，不能固定在文本最前面。
- [x] 激活表格时显示浮动工具条，提供上/下插入行、删除行、左/右插入列、
  删除列、左/中/右对齐和删除表格。
- [~] 提供 6×6 尺寸选择网格并执行表格 resize；尚需补展开态交互截图、
  hover 高亮和按钮真实点击回归。
- [x] 所有增删行列、对齐和 resize 都使用 `SourceTransaction`，支持 Undo/Redo，
  且重新解析后保持最接近的活动单元格。
- [x] 表格 delimiter 的 `:---`、`:---:`、`---:` 同时驱动预览与编辑 Input
  的左、中、右对齐。
- [x] 表格上下文快捷键已覆盖插入/删除行列和左/中/右对齐；非表格状态下
  action 必须继续传播，不能吞掉其他组件快捷键。
- [ ] 工具条视觉继续收敛到 Typora：使用统一图标和中文 Tooltip，高亮当前列
  对齐状态，尺寸网格 hover 时高亮左上矩形并显示 `列 × 行`。

### F. 完成定义与验证门禁

- [ ] 定向修复 task checkbox 绘制，并用 GPUI layout bounds 测试和 Headless
  截图共同确认所有 marker 可见。
- [x] 完成嵌套有序列表编号 contract 与编辑器回归测试。
- [x] 完成行内 marker 不占宽方案和临界换行回归测试。
- [ ] 完成表格尺寸 Popover 展开态、真实按钮点击和工具条视觉回归。
- [ ] 运行 `cargo test -p markdown-editor`、`cargo test -p markdown-source`、
  `cargo test -p notes`。
- [ ] 运行 Markdown 相关 crate 的 Clippy、`cargo fmt --all -- --check`、
  `git diff --check` 和 `cargo check -p main`。
- [ ] 根据最新测试与截图逐项回填本清单；只有所有 `[ ]` 和 `[~]` 收敛后，
  才能声明 Typora 风格 Markdown 编辑阶段完成。

## 已实现

- `SourceMarkdownDocument`、revision、块/行内节点、UTF-8 字节范围。
- Heading、paragraph、blockquote、list、code fence、table、frontmatter、HTML、raw Markdown 映射。
- Emphasis、strong、inline code、link、image、linked image、hard break 映射。
- 表格行/单元格、内容范围、delimiter row、pipe 风格映射。
- 稳定语义 fingerprint；忽略强调 marker、链接尖括号、列表起始编号归一化、表格空格以及默认/显式左对齐差异。
- revision、allowed range、UTF-8 boundary、重叠检查和逆向补丁。
- 源码级 Undo/Redo 栈。
- LCS/greedy 块对齐和结构变化补丁。
- 表格单元格修改/清空、图片 alt/路径修改和自链接图片整体删除操作。
- 光标进入行内语法节点时返回该节点原始源码的 contract，为 Typora 式局部源码显隐提供基础。
- 独立 `MarkdownEditor` Entity 使用真实 `InputState`、显式焦点、宿主主题和源码级 Undo/Redo。
- 行内 emphasis、strong、inline code、link、image 在非活动状态隐藏 marker；光标进入时只展开当前节点的原始源码，其他行内节点保持隐藏。
- 行内 emphasis、strong、inline code、link、image、delete 已应用斜体、粗体、代码、链接和删除线等语义样式；进入节点后仍就地显示其原始 marker 并可编辑。
- 显示偏移、源码偏移和替换终点分别建表，覆盖中文、emoji、嵌套链接、linked image 和表格行内节点。
- 源码级 Undo/Redo 已接管 Markdown 编辑器快捷键并恢复源码选区。
- 自链接图片的 Backspace/Delete 会删除完整 wrapper，表格单元格编辑只修改映射的内容范围。
- Notes Markdown 主路径直接保存 `MarkdownEditor` 持有的权威源码；无用户编辑时不写文件。
- Source/WYSIWYG 模式切换共享同一个 `SourceMarkdownDocument`。
- Source 模式使用最小 UTF-8 diff 生成 `SourceEditor` 事务，与混合编辑共用历史和选区恢复。
- 非活动 Heading、Paragraph、Blockquote、List、Code Fence、Table 和 HTML 使用现有 `TextView` 富文本渲染；点击后仅当前块切换为原位 `InputState` 编辑。
- Frontmatter 和未知扩展以明确的 Raw 源码卡片显示，点击后只编辑节点源码范围。
- 表格使用独立网格 UI，不显示 delimiter row；单元格编辑和 Clear 只替换 `content_range`。
- 活动图片提供 alt/path 属性条，Save 在一个事务内更新两个范围，Delete 删除完整 wrapper 并保留外层链接语义。
- 活动块提供上移、下移、引用、代码围栏和删除操作；列表 Enter 延续原 marker，有序列表编号递增。
- 空文档显示可聚焦编辑面，可直接输入首个块。
- `SourceHistory` 支持多轮 Undo/Redo 后继续保留原始前后选区。
- 单块安全编辑执行增量解析，保持块 ID 并平移后续范围；结构变化自动回退全量解析。
- Markdown session 使用 `notify` watcher 监听文件；本地干净时自动重载，本地脏时进入冲突流程，自保存回环按权威源码忽略。
- Workspace Explorer 继续通过 Notes Markdown 宿主使用相同链路。

## 当前验证

- `cargo test -p markdown-source`
- `cargo clippy -p markdown-source --all-targets -- -D warnings`
- `cargo test -p markdown-editor`
- `cargo clippy -p markdown-editor --all-targets -- -D warnings`
- `cargo test -p notes`
- `cargo clippy -p notes --all-targets --no-deps -- -D warnings`
- `cargo test -p workspace_explorer markdown_`

完整 Notes Clippy（包含依赖）仍会被仓库既有 `one-core` 告警阻塞；本次改动范围使用 `--no-deps` 验证。

## 后续阶段

1. 完成视觉回归基线，细化活动 Heading 的字号、列表缩进和代码块编辑态，使编辑态与非活动富文本态更接近 Typora。
2. 增加点击编辑器空白区域退出活动块、跨块键盘导航，以及列表空项 Enter 退出列表等完整结构交互。
3. 增加外部修改后的节点级选区重定位和可视化差异查看。
4. 建立大文件解析、输入延迟和 watcher 压力基准。

## 验收原则

- 打开、预览和模式切换不产生无意义写入。
- 修改一个节点不能改变未编辑节点的源码拼写。
- 所有未知语法必须原样保留。
- 无法证明安全的投影必须回退到源码编辑。
- Source 与混合 WYSIWYG 使用宿主主题，并保持 UTF-8 字节范围与 UI 光标位置之间的显式映射。

## 当前明确边界

- 已完成 Typora 式行内源码显隐的交互语义：默认隐藏 marker，进入节点仅展开该节点源码。
- 已完成行内语义样式、真实光标切换测试和 Markdown 源码级 Undo/Redo 快捷键路由。
- 已完成块级富文本预览、当前块原位编辑、HTML 预览、Raw 卡片、表格网格、图片属性和块结构事务。
- 活动 Heading 已按层级保留字号和字重；细节仍可继续与 `TextView` 的精确行高、段前段后距对齐。
- 当前交互边界是点击另一个块可以切换活动块，但点击空白区域不会自动回到纯预览态。
- 增量解析覆盖安全的单块编辑；跨块、结构变化和无法证明安全的情况会全量解析，不冒险复用失效范围。
