# Cditor Markdown 语义级 Round-trip 设计

> 本文是 `2026-07-15-notes-richtext-markdown-dual-mode-design.md` 的 Cditor 专项设计。Cditor 负责 Markdown 与富文本模型之间的安全转换；Notes 负责 `.md` 文件、源码编辑器、双视图状态机、document index 和外部文件冲突。

## 背景

Cditor 当前公开以下第三方集成 API：

```rust
EditorDocument::from_markdown(document_id, source)
EditorDocument::to_markdown()

EditorHandle::set_markdown(source, cx)
EditorHandle::get_markdown(cx)
```

`from_markdown` 已能把常用 Markdown 解析为 Cditor block 和 inline marks，但 `to_markdown` 当前调用 `export_plain_markdown`。现有 exporter 主要通过 `block.payload.plain_text()` 提取文本，再补标题、列表、引用和代码围栏等块级标记。

这种实现适合“尽力导出纯 Markdown”，不适合作为 `.md` 唯一真源的持久化路径。它可能丢失：

- Bold、Italic、Strike。
- Inline code。
- Link href。
- 多 mark 嵌套关系。
- 嵌套列表结构和缩进。
- Markdown 无法表达的富文本块。
- 无法识别但需要原样保留的 Markdown 扩展。

Notes 的 Markdown WYSIWYG 模式必须建立在严格、可诊断、禁止静默丢失的 Cditor conversion contract 上。

## 目标

- 为支持的 Markdown 语法提供语义级 parse → export → parse round-trip。
- 为源码规范化提供明确的 compatibility/fidelity 报告。
- 遇到无法安全表达的富文本内容时，Strict export 必须失败且不产生可写回结果。
- 遇到无法安全往返的 Markdown 源码时，parser 必须标记为 SourceOnly，而不是静默降级为普通段落。
- 为第三方宿主提供带报告的 import、export、apply 和只读 preview API。
- 保持现有 Cditor JSON round-trip、独立应用和普通富文本编辑行为不变。

## 非目标

- Cditor 不实现 Markdown 源码编辑器。
- Cditor 不管理 `.md` 文件路径和原子写入。
- Cditor 不管理 Notes document ID index。
- Cditor 不实现文件 watcher、外部修改检测或冲突 UI。
- 首版不提供 Markdown 字符级 source preservation。
- 首版不自动生成 Navop 私有 Markdown 扩展语法。
- 不向第三方宿主暴露内部 `DocumentRuntime`。

## 设计原则

### 严格和宽松导出分开

Markdown 作为唯一真源时必须使用 Strict；复制、预览和兼容旧调用方时可以使用 BestEffort。不能让一个无返回报告的字符串 API 同时承担两种语义。

### 不可表达内容禁止静默降级

例如颜色、下划线、附件、白板和数据库块不能在 Strict 模式中退化成 plain text 后覆盖 `.md`。

### 语义级无损允许源码规范化

首版允许：

- `_italic_` 规范化为 `*italic*`。
- 有序列表统一使用 `1.`。
- 块间空行规范化。
- Heading 和 fence 使用统一风格。

但重新解析后必须保持支持内容的语义等价。

### Parser fallback 必须可观察

Parser 把未知语法当普通段落时，必须区分“用户本来输入普通文本”和“不支持语法导致 fallback”。后者需要 diagnostic。

## 公共类型

新增 Markdown conversion contract：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownFidelity {
    Semantic,
    Normalized,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownDiagnostic {
    pub severity: MarkdownDiagnosticSeverity,
    pub code: &'static str,
    pub message: String,
    pub source_range: Option<std::ops::Range<usize>>,
    pub block_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkdownCompatibility {
    Editable,
    EditableWithNormalization(Vec<MarkdownDiagnostic>),
    SourceOnly(Vec<MarkdownDiagnostic>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownExportMode {
    Strict,
    BestEffort,
}

pub struct MarkdownImportResult {
    pub document: EditorDocument,
    pub compatibility: MarkdownCompatibility,
}

pub struct MarkdownExportResult {
    pub markdown: String,
    pub fidelity: MarkdownFidelity,
    pub diagnostics: Vec<MarkdownDiagnostic>,
}
```

语义：

- `Semantic`：支持内容可语义级往返，不需要规范化提示。
- `Normalized`：内容语义可保持，但源码风格会变化。
- `Unsupported`：至少有一项不能安全写回 Markdown。
- `Editable`：允许 WYSIWYG 编辑和 Strict export。
- `EditableWithNormalization`：用户接受规范化后允许 WYSIWYG。
- `SourceOnly`：源码可编辑，Cditor 只能提供只读 projection 或不进入 WYSIWYG。

## EditorDocument API

新增：

```rust
impl EditorDocument {
    pub fn from_markdown_with_report(
        document_id: impl Into<String>,
        source: &str,
    ) -> Result<MarkdownImportResult, EditorError>;

    pub fn export_markdown(
        &self,
        mode: MarkdownExportMode,
    ) -> Result<MarkdownExportResult, EditorError>;
}
```

保留现有 API 兼容：

```rust
pub fn from_markdown(
    document_id: impl Into<String>,
    source: &str,
) -> Result<Self, EditorError> {
    Ok(Self::from_markdown_with_report(document_id, source)?.document)
}
```

现有 `to_markdown()` 暂时映射到 BestEffort：

```rust
pub fn to_markdown(&self) -> Result<String, EditorError> {
    Ok(self
        .export_markdown(MarkdownExportMode::BestEffort)?
        .markdown)
}
```

其文档必须明确说明它可能规范化或降级，不能用于 `.md` 唯一真源持久化。

Notes 必须调用：

```rust
document.export_markdown(MarkdownExportMode::Strict)
```

## Inline Markdown serializer

新增：

```text
crates/core/src/rich_text/markdown/inline_export.rs
```

入口：

```rust
pub struct InlineMarkdownExport {
    pub markdown: String,
    pub fidelity: MarkdownFidelity,
    pub diagnostics: Vec<MarkdownDiagnostic>,
}

pub fn export_inline_spans(
    spans: &[InlineSpan],
    mode: MarkdownExportMode,
) -> InlineMarkdownExport;
```

### Plain text escaping

普通文本至少处理：

```text
\ * _ ~ ` [ ] < >
```

在行首或块级敏感位置还需处理：

```text
#
>
-
+
数字 + "."
```

escaping 必须根据上下文进行，不能对代码 span 和 URL 使用普通文本规则。

建议新增：

```text
crates/core/src/rich_text/markdown/escape.rs
```

分别实现：

```rust
escape_inline_text
escape_link_label
escape_link_destination
escape_table_cell
choose_code_span_delimiter
choose_code_fence
```

### Bold

```rust
InlineMark::Bold
```

规范化输出：

```markdown
**text**
```

### Italic

规范化输出：

```markdown
*text*
```

不保留输入使用 `*` 还是 `_`。

### Bold + Italic

规范化输出：

```markdown
***text***
```

内部 marks 顺序不能影响结果。

### Strike

```markdown
~~text~~
```

### Inline code

delimiter 长度必须大于内容中最长连续反引号长度。例如内容包含一个反引号时使用两个：

```markdown
``hello `world` ``
```

内容首尾包含反引号或空格时按 CommonMark code span 规则增加 padding。

### Link

输出：

```markdown
[label](<https://example.com/a(b)>)
```

需要独立转义 label 和 destination，不能复用普通文本 escaping。

### Mark canonical order

marks 必须先规范化顺序再序列化。建议从外到内：

```text
Link
Strike
Bold
Italic
Code
```

Code span 内部不能继续解析强调，因此 `Code + Bold` 等无法同时用标准 Markdown 表达的组合：

- Strict：返回 Unsupported。
- BestEffort：选择 code 并返回降级 warning。

### Markdown 无法表达的 inline marks

首版 Strict Unsupported：

```text
Underline
Color
Background
```

diagnostic code：

```text
markdown.inline.underline_unsupported
markdown.inline.color_unsupported
markdown.inline.background_unsupported
markdown.inline.mark_combination_unsupported
```

首版不自动输出 inline HTML。

## Block Markdown serializer

新增：

```text
crates/core/src/rich_text/markdown/block_export.rs
```

不能继续按 `document.blocks.iter()` 平铺输出。必须根据 `parent_id` 和 `depth` 重建逻辑块树，再递归序列化。

入口：

```rust
pub fn export_document_blocks(
    document: &RichTextDocument,
    mode: MarkdownExportMode,
) -> MarkdownExportResult;
```

### Paragraph

调用 inline serializer，块之间使用 canonical blank line。

### Heading

支持 level 1–6：

```markdown
### Heading
```

非法 level：

- Strict：Unsupported。
- BestEffort：限制到 1–6 并产生 warning。

### Bulleted list

```markdown
- parent
  - child
    - grandchild
```

嵌套缩进首版固定两个空格。

### Numbered list

canonical 输出：

```markdown
1. first
1. second
```

输入原始编号不同但语义相同时返回 Normalized。

### Todo

```markdown
- [ ] pending
- [x] completed
```

### Quote

每一行都增加 quote prefix：

```markdown
> line 1
> line 2
```

### Callout

规范化为 GitHub 风格：

```markdown
> [!WARNING]
> content
```

### Code block

根据内容中最长连续 fence 选择外层 fence 长度，不能固定三个反引号。

保留合法 language tag；非法 language tag 在 Strict 中返回 diagnostic。

### Table

必须覆盖：

- Pipe escaping。
- Backslash escaping。
- Alignment。
- Header row。
- Inline marks。

Markdown 无法表达的 rowspan、colspan 或 merge：Strict Unsupported。

### Divider

统一输出：

```markdown
---
```

### RawMarkdown

存在 `raw_fallback` 时原样输出。

缺少 `raw_fallback`：

- Strict：Unsupported。
- BestEffort：plain text + warning。

### Mermaid

payload 能稳定恢复源码时输出：

````markdown
```mermaid
graph TD
```
````

### Math

payload 能恢复块级公式源码时输出：

```markdown
$$
formula
$$
```

否则 Strict Unsupported。

### Image

有稳定 URL 和 alt 时输出：

```markdown
![alt](url)
```

只有内部 bitmap、asset ID 或无法访问资源时 Strict Unsupported。

### HTML

保留原始 HTML source 时可以输出并标记 Normalized 或 SourceOnly。

只有渲染后结构而没有原始 source 时 Strict Unsupported。

### 富文本专属块

首版 Strict Unsupported：

```text
File
Attachment
Whiteboard
MindMap
Embed
Database
Custom
```

diagnostic 必须包含 block ID 和建议“改存为富文本文档”。

## Markdown importer compatibility

保留现有 parser 入口，新增带报告版本：

```rust
pub fn parse_markdown_document_with_report(
    source: &str,
    options: MarkdownImportOptions,
) -> MarkdownParseResult;
```

返回：

```rust
pub struct MarkdownParseResult {
    pub document: ParsedMarkdownDocument,
    pub compatibility: MarkdownCompatibility,
    pub diagnostics: Vec<MarkdownDiagnostic>,
}
```

Parser 必须报告：

- Frontmatter 未支持。
- Reference links 未支持。
- Footnotes 未支持。
- Raw HTML 未稳定支持。
- 未知 fenced block。
- 自定义 Markdown 扩展。
- 无法保持的 nested inline marks。
- 不支持语法 fallback 成普通 paragraph。

### Fallback 规则

以下两种输入必须区分：

```markdown
ordinary text
```

与：

```markdown
[^1]: unsupported footnote
```

前者是真正 paragraph；后者若被 parser 当普通文本，必须生成 diagnostic 并至少标记 `SourceOnly`。

### Compatibility 判定

```text
无 diagnostics                      -> Editable
只有规范化 diagnostics              -> EditableWithNormalization
存在不能安全重新导出的 diagnostics   -> SourceOnly
```

SourceOnly 文档可以生成只读 projection，但不能进入可写 WYSIWYG 并覆盖源文件。

## Integration 公共 API

新增：

```text
crates/app/src/integration/markdown.rs
```

从 `cditor_app` 顶层 re-export：

```rust
MarkdownCompatibility
MarkdownDiagnostic
MarkdownDiagnosticSeverity
MarkdownExportMode
MarkdownExportResult
MarkdownFidelity
MarkdownImportResult
MarkdownApplyMode
DocumentReplaceReason
```

### EditorHandle export

```rust
impl EditorHandle {
    pub fn export_markdown<C: AppContext>(
        &self,
        mode: MarkdownExportMode,
        cx: &C,
    ) -> Result<MarkdownExportResult, EditorError>;
}
```

### EditorHandle apply

```rust
pub enum MarkdownApplyMode {
    Editable,
    ReadOnlyPreview,
}

impl EditorHandle {
    pub fn apply_markdown<C: AppContext>(
        &self,
        source: impl Into<String>,
        mode: MarkdownApplyMode,
        cx: &mut C,
    ) -> Result<MarkdownImportResult, EditorError>;
}
```

行为：

- `Editable` 只接受 Editable 或已获宿主确认的 EditableWithNormalization。
- SourceOnly + Editable 返回明确错误，不替换当前 runtime。
- SourceOnly + ReadOnlyPreview 可以替换为只读 projection。
- apply 成功后刷新 persistence baseline，不能立即触发 autosave。

## Source projection 和同步回声

Source 模式切回 WYSIWYG 时，宿主会把已经保存的 `.md` 重新投影为 Cditor runtime。该操作不是用户在 Cditor 内编辑，不能触发：

```text
Source 保存
-> apply_markdown
-> Cditor Changed
-> autosave
-> 再写 Source
```

增加：

```rust
pub enum DocumentReplaceReason {
    ExternalReload,
    SourceModeCommit,
    Programmatic,
}
```

建议 API：

```rust
pub fn replace_document<C: AppContext>(
    &self,
    document: EditorDocument,
    reason: DocumentReplaceReason,
    cx: &mut C,
) -> Result<(), EditorError>;
```

replace 必须：

- 替换 runtime。
- 重置 integration persistence baseline。
- 不标记 dirty。
- 不调度 autosave。
- 不把 projection replace 发成普通 Changed。

可以新增事件：

```rust
EditorEvent::DocumentReplaced {
    document_id: String,
    reason: DocumentReplaceReason,
}
```

如果现有 `set_document` 已保证这些语义，可以保留实现并补测试，但公开 API 仍建议显式携带 reason。

## EditorEvent

现有：

```text
Ready
Changed
SaveStateChanged
Saved
SaveFailed
LoadFailed
```

建议新增：

```rust
EditorEvent::DocumentReplaced {
    document_id: String,
    reason: DocumentReplaceReason,
}
```

Compatibility 可以通过 apply 返回值交给宿主，不强制新增长期事件。

`Saved` 已足够让 Notes 在 WYSIWYG persistence 成功后，从 shared state 更新源码编辑器。

## Error 类型

增加明确错误：

```rust
pub enum EditorError {
    // existing variants...
    MarkdownUnsupported {
        diagnostics: Vec<MarkdownDiagnostic>,
    },
    MarkdownSourceOnly {
        diagnostics: Vec<MarkdownDiagnostic>,
    },
}
```

如果 `EditorError` 需要保持轻量，可以用单独错误类型：

```rust
pub struct MarkdownConversionError {
    pub message: String,
    pub diagnostics: Vec<MarkdownDiagnostic>,
}
```

错误必须能让宿主展示具体 block 和建议，不应只返回 `invalid Markdown`。

## 文件结构

```text
crates/core/src/rich_text/markdown/
├── mod.rs
├── block.rs
├── inline.rs
├── table.rs
├── export.rs
├── inline_export.rs
├── block_export.rs
├── escape.rs
├── compatibility.rs
└── tests/
    ├── parse.rs
    ├── export_inline.rs
    ├── export_blocks.rs
    ├── round_trip.rs
    └── unsupported.rs

crates/app/src/integration/
├── mod.rs
├── document.rs
├── handle.rs
├── markdown.rs
├── events.rs
└── persistence.rs
```

每个文件保持单一职责，避免继续扩张现有 `export.rs` 和 integration 文件。

## 测试

### Inline round-trip

覆盖：

```markdown
plain
**bold**
*italic*
***bold italic***
~~strike~~
`code`
[link](https://example.com)
```

断言 parse → export → parse 后 inline marks 和 href 语义等价。

### Escaping

覆盖：

```text
\ * _ ~ ` [ ] ( ) < > |
```

以及中文、emoji、组合字符。

### Nested list

```markdown
- parent
  - child
    1. first
    2. second
```

断言 parent ID、depth、kind 和顺序语义等价。

### Code span 和 fence

覆盖内容中出现一个到多个连续反引号，以及 fenced code 内嵌 fence。

### Table

覆盖 pipe、反斜杠、inline code 和 link escaping；merged table Strict Unsupported。

### Unsupported

分别构造：

```text
Underline
Color
Background
Attachment
Whiteboard
Database
Custom
Merged table
```

Strict export 必须：

- fidelity 为 Unsupported。
- diagnostics 包含 block ID。
- 不返回可被宿主误认为安全的结果。

### Normalization

输入：

```markdown
1. first
2. second
```

允许规范化为：

```markdown
1. first
1. second
```

但 fidelity 必须是 Normalized。

### Parser SourceOnly

覆盖 frontmatter、footnote、reference link、raw HTML 和未知扩展，确认不会无 diagnostic 地降级成 paragraph。

### Integration

- `apply_markdown` 返回 compatibility。
- SourceOnly 不能以 Editable 应用。
- ReadOnlyPreview 不能编辑。
- `export_markdown(Strict)` 拒绝 unsupported。
- Source projection replace 不触发 dirty/autosave loop。
- document ID 保持不变。
- EditorHandle focus 和 keymap 不回归。
- JSON persistence 和 `EditorDocument::to_json/from_json` 不受影响。

## 实施顺序

1. 定义 compatibility、diagnostic、fidelity 和 export mode。
2. 为新 API 写编译 contract 和失败测试。
3. 实现 escaping 和 inline exporter。
4. 实现递归 block exporter。
5. 完成 table、code fence 和 raw Markdown。
6. 为 unsupported rich blocks 增加 Strict gate。
7. 增强 parser compatibility report。
8. 增加 `EditorDocument` with-report API。
9. 增加 `EditorHandle::export_markdown/apply_markdown`。
10. 明确 source projection replace 和事件语义。
11. 完成 round-trip、unsupported 和 integration 测试。
12. 更新 integration guide，推送新 revision，Navop 固定依赖。

## 验收标准

1. Notes 不需要调用现有有损 `to_markdown()` 保存 `.md`。
2. 支持语法在 Strict 模式中语义级 round-trip。
3. Bold、Italic、Strike、Inline code 和 Link 不丢失。
4. 嵌套列表层级不丢失。
5. Table 和 code fence 正确 escaping。
6. Unsupported rich content 阻止写入，不静默降级。
7. Unsupported Markdown source 返回 SourceOnly。
8. Source projection replace 不产生 autosave 同步回声。
9. 旧 Cditor JSON 文档、独立应用、焦点和 keymap 无回归。
10. 新增公共类型和 API 从 `cditor_app` 顶层可访问。

验证至少包括：

```bash
rtk cargo test -p cditor-core markdown
rtk cargo test -p cditor-app integration
rtk cargo check -p cditor-app --no-default-features
rtk cargo fmt --all -- --check
rtk cargo clippy -p cditor-core -p cditor-app --all-targets -- -D warnings
```

## 与 Notes 的边界

Cditor 输出：

```text
MarkdownImportResult
MarkdownExportResult
MarkdownCompatibility
MarkdownDiagnostic
EditorHandle::apply_markdown
EditorHandle::export_markdown
```

Notes 消费这些 API 并负责：

```text
.md 文件原子读写
Markdown 源码 InputState
双视图同步状态机
document ID index
外部修改 fingerprint
冲突处理
模式切换 UI
自动保存调度
关闭和删除门禁
```

该边界让 Cditor 保持通用第三方富文本组件，同时让 Notes 对本地文件和产品交互拥有完整控制权。

## Navop 当前接入接缝

Navop 已把 Cditor Markdown 依赖收敛到：

```text
crates/notes/src/markdown_adapter.rs
```

当前实现已直接消费 strict contract：

```rust
build_markdown_projection(source, store)
apply_markdown_source(source, normalization_accepted)
export_markdown_strict(handle)
```

`Editable` projection 可直接编辑；`EditableWithNormalization` 在用户确认前只读；`SourceOnly` 始终只读。WYSIWYG 通过 `MarkdownDocumentPersistence` 调用 Strict export，保存到与 Source editor 共用的 `MarkdownFileStore`，因此不会使用当前有损 exporter 覆盖 `.md`。

Cditor API 已提供以下宿主语义，并由 adapter 隔离：

```rust
pub struct MarkdownProjection {
    pub handle: EditorHandle,
    pub compatibility: MarkdownCompatibility,
}

pub enum StrictMarkdownExport {
    Writable { markdown: String, diagnostics: Vec<MarkdownDiagnostic> },
    Blocked { diagnostics: Vec<MarkdownDiagnostic> },
}
```

adapter 当前职责：

1. 通过 `from_markdown_with_report` 创建 projection。
2. 根据 compatibility 设置 Editable、NormalizationRequired 或 SourceOnly。
3. Source 更新 projection 时使用“不产生 dirty/autosave 回声”的 replace API。
4. WYSIWYG 保存只调用 Strict export。
5. Strict export 返回 Unsupported/Blocked 时保留 Cditor dirty，并禁止覆盖 `.md`。
6. diagnostics 以 compatibility 和数量进入 Notes toolbar，后续可扩展详细问题面板。

Navop workspace 直接依赖 Cditor `main` branch，并通过 `cargo update -p cditor-app` 更新。Cditor `main` 已包含 Navop GPUI revision、embedded focus、公开 keymap init 和 `default-features=false` 下可选 SQLite；不再通过独立兼容分支提供 Navop revision。

Navop 不应使用以下兼容 API作为 Markdown persistence：

```rust
EditorDocument::to_markdown()
EditorHandle::get_markdown()
```

它们只能继续服务 BestEffort 导出、复制或旧调用方。这个约束是 `.md` 作为唯一真源的数据安全门禁。
