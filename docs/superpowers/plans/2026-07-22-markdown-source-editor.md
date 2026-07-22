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
