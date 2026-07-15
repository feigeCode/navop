# Notes 富文本与 Markdown 双格式设计

> 本文是 `2026-07-14-notes-cditor-design.md` 的增量设计。涉及文档格式、Markdown 支持、本地数据结构和编辑器生命周期时，以本文为准；原设计中“首版不提供 Markdown 导入导出”的非目标不再适用。

> Cditor 内部 Markdown round-trip、compatibility 和 integration API 的详细改造见 `2026-07-15-cditor-markdown-roundtrip-design.md`。

## 背景

Notes 当前只支持 Cditor 原生富文本文档，内容保存为 `.cditor.json`。用户希望保留这种无损富文本模式，同时新增纯 Markdown 文档：内容以 `.md` 为唯一真源，既可以在 Cditor 中所见即所得编辑，也可以切换到 Markdown 源码模式直接编辑。

当前 Cditor 已提供 `EditorDocument::from_markdown`、`EditorDocument::to_markdown`、`EditorHandle::set_markdown` 和 `EditorHandle::get_markdown`。但现有 `to_markdown` 使用 `export_plain_markdown`，只能重建部分块级结构，不能可靠序列化 inline marks、嵌套列表和全部富文本块。因此不能直接把现有 exporter 用作 `.md` 持久化，否则可能静默丢失粗体、链接或不可表达的富文本内容。

本设计采用“文档格式分流、Markdown 单一真源、兼容性门禁、语义级 round-trip”的方案。它优先保证数据安全和可恢复性，不追求首版对 Markdown 源码字符级无损。

## 核心决策

### 文档格式和视图模式分开建模

Notes 支持两种文档格式：

```rust
pub enum DocumentFormat {
    RichText,
    Markdown,
}
```

- `RichText` 使用 Cditor 编辑，保存为 `.cditor.json`。
- `Markdown` 保存为 `.md`，提供 WYSIWYG 和 Source 两种视图。

Markdown 的视图模式单独建模：

```rust
pub enum MarkdownViewMode {
    Wysiwyg,
    Source,
}
```

不允许把一个富文本文档仅通过切换按钮变成 Markdown 文档。格式转换是可能丢失表达能力的独立操作，不属于首版。

### 内容真源唯一

- 富文本文档的唯一真源是 `.cditor.json`。
- Markdown 文档的唯一内容真源是 `.md`。
- Markdown 文档不生成并排的 `.cditor.json` 镜像。
- Cditor 的 `EditorDocument` 只是 `.md` 的运行时投影。

该规则避免双写文件产生“哪个版本更新”的歧义。

### 首版保证支持语法的语义级 round-trip

首版允许 Markdown 源码被规范化，例如空行、列表编号和强调符号风格可能变化，但要求支持的语法在重新解析后语义等价：

- 标题级别不变。
- 粗体、斜体、删除线、行内代码不丢失。
- 链接文字和地址不丢失。
- 列表类型、层级和任务状态不变。
- 引用、代码块、表格、分隔线内容不丢失。

不支持或无法安全导出的内容必须产生 compatibility diagnostic，并阻止 WYSIWYG 写回 `.md`。系统不得静默降级为纯文本后覆盖原文件。

### 源码级无损不是首版目标

首版不保证保留：

- `*italic*` 与 `_italic_` 的原始选择。
- 原始空行数量和尾随空格。
- 原始有序列表编号。
- ATX 与 Setext 标题风格。
- 引用式链接的原始声明位置。
- 未识别扩展语法的 source range。

如果未来要求字符级无损，需要引入 Markdown AST、source range 和增量 patch 层，不能继续使用简单的 parse/export 模型。

## 用户体验

### 新建文档

新建文档 Dialog 增加格式选择：

```text
┌ 新建文档 ──────────────────────┐
│ 名称  [ 项目计划             ] │
│                                │
│ 格式                           │
│ ● 富文本文档                   │
│ ○ Markdown 文档                │
│                                │
│                 取消   创建    │
└────────────────────────────────┘
```

默认选择上一次成功创建的格式；没有历史状态时默认 `RichText`，保持现有行为兼容。

名称输入不包含扩展名：

- 富文本自动添加 `.cditor.json`。
- Markdown 自动添加 `.md`。
- 用户输入保留后缀时拒绝，避免双扩展名。

### 文件树

文件树同时识别：

```text
欢迎.cditor.json
README.md
```

界面隐藏扩展名，但用图标或轻量 badge 区分：

```text
📘 欢迎       Rich
Ⓜ README      MD
```

若同一目录同时存在 `README.md` 和 `README.cditor.json`，两者都显示，不按 display name 去重。

### Markdown 编辑区

Markdown 文档顶部显示视图切换：

```text
README.md                      已保存
                  [ 所见即所得 | Markdown 源码 ]
```

- WYSIWYG 使用 Cditor。
- Source 使用 `gpui_component::input::InputState::code_editor("markdown")`。
- Source 开启 Markdown syntax highlighting、行号、搜索、软换行和两空格缩进。
- 切换按钮在同步进行中禁用，避免并发切换。

富文本文档不显示 Markdown 视图切换器。

### 兼容性提示

Markdown 打开后计算 compatibility：

```rust
pub enum MarkdownCompatibility {
    Editable,
    EditableWithNormalization(Vec<MarkdownDiagnostic>),
    SourceOnly(Vec<MarkdownDiagnostic>),
}
```

- `Editable`：可直接 WYSIWYG 编辑。
- `EditableWithNormalization`：允许 WYSIWYG，但首次切换前提示源码将被规范化。
- `SourceOnly`：源码可编辑，WYSIWYG 只读预览或不可进入编辑状态。

用户未明确接受规范化前，不覆盖原始 `.md`。

## 本地数据格式

```text
notes/
├── notebook.json
├── state.json
├── documents.json
└── files/
    ├── 欢迎.cditor.json
    ├── README.md
    └── 工作/
        ├── 项目计划.cditor.json
        └── 周报.md
```

### 文档索引

Markdown 文件自身不携带稳定 document ID。为避免重命名后丢失 EditorHandle、光标和撤销历史，新增 `documents.json`：

```rust
pub struct DocumentIndex {
    pub schema_version: u32,
    pub documents: BTreeMap<PathBuf, DocumentRecord>,
    pub pending_operation: Option<PendingDocumentOperation>,
}

pub struct DocumentRecord {
    pub id: Uuid,
    pub format: DocumentFormat,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

规则：

- Markdown 的 ID 以 `documents.json` 为准。
- RichText 的 ID 以 `.cditor.json` 内部 ID 为准，索引记录必须与其一致。
- 首次升级时扫描现有 `.cditor.json`，读取其 ID 并建立索引。
- 扫描到未登记 `.md` 时分配新 UUID。
- 扫描到未登记 `.cditor.json` 时读取内部 ID 后登记。
- 索引中存在但文件已删除的记录在确认无 pending operation 后清理。

### 重命名的崩溃恢复

文件 rename 和索引写入不能构成单文件原子事务。内部重命名使用操作日志：

```rust
pub enum PendingDocumentOperation {
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    Delete {
        path: PathBuf,
    },
}
```

Notes foreground 同一时刻只执行一个文件结构操作，因此索引只需要一个 `pending_operation`，不允许并行 rename/delete。

重命名流程：

1. 原子写入带 pending rename 的 `documents.json`。
2. 执行文件系统 rename。
3. 更新索引 key 并清除 pending operation。

启动恢复：

- `from` 不存在、`to` 存在：完成索引迁移。
- `from` 存在、`to` 不存在：回滚 pending operation。
- 两者都存在或都不存在：进入冲突状态，不自动覆盖或删除。

## 数据模型

```rust
pub struct DocumentDescriptor {
    pub document_id: Uuid,
    pub format: DocumentFormat,
    pub relative_path: PathBuf,
    pub absolute_path: PathBuf,
}

pub enum NodeKind {
    Directory,
    Document(DocumentFormat),
}
```

如果带数据的 `NodeKind` 会让排序和复制逻辑变复杂，也可以保持 `NodeKind::Document`，并在 `FileNode` 上单独增加 `format: Option<DocumentFormat>`。实现时以文件职责和测试可读性为准。

UI 状态增加：

```rust
pub struct NotebookUiState {
    pub selected_document: Option<PathBuf>,
    pub expanded_directories: BTreeSet<PathBuf>,
    pub markdown_view_modes: BTreeMap<String, MarkdownViewMode>,
    pub last_created_format: DocumentFormat,
}
```

视图模式按 document ID 保存，重命名不影响恢复。

## Cditor Markdown contract

### 新 API

Cditor 增加带报告的导入导出接口，保留现有简化 API 兼容：

```rust
pub struct MarkdownImportResult {
    pub document: EditorDocument,
    pub compatibility: MarkdownCompatibility,
}

pub struct MarkdownExportResult {
    pub markdown: String,
    pub diagnostics: Vec<MarkdownDiagnostic>,
    pub fidelity: MarkdownFidelity,
}

pub enum MarkdownFidelity {
    Semantic,
    Normalized,
    Unsupported,
}

impl EditorDocument {
    pub fn from_markdown_with_report(
        document_id: impl Into<String>,
        source: &str,
    ) -> Result<MarkdownImportResult, EditorError>;

    pub fn to_markdown_with_report(
        &self,
    ) -> Result<MarkdownExportResult, EditorError>;
}
```

原有 `from_markdown` 可以返回报告中的 document；原有 `to_markdown` 只允许在 fidelity 不是 `Unsupported` 时返回文本，否则返回明确错误。

### 首版支持语法

必须建立完整 round-trip contract：

- Paragraph。
- Heading 1–6。
- Bold、Italic、Strike、Inline code。
- Link。
- Bulleted list、Numbered list、Nested list。
- Todo。
- Quote。
- GitHub-style callout。
- Fenced code 和语言名。
- Table 和单元格 pipe escaping。
- Divider。
- Raw Markdown block。
- Unicode 和中文文本。

### 不可表达内容策略

以下内容不能静默丢失：

- Underline。
- Text/background color。
- File、Attachment。
- Whiteboard、MindMap、Database。
- Embed 和未知 Custom block。

首版策略：

- 如果来源 `.md` 中是可原样保存的 raw/HTML 片段，标记为 `SourceOnly` 或只读 projection。
- 如果内容由 WYSIWYG 新建且无法导出，保存返回 `Unsupported`，保留 dirty 状态并提示用户删除该格式或改存为富文本文档。
- 不自动插入 Navop 私有 Markdown 语法。

## Markdown session

每个打开的 Markdown 文档缓存一个 session：

```rust
pub struct MarkdownSession {
    pub document_id: Uuid,
    pub relative_path: PathBuf,
    pub mode: MarkdownViewMode,
    pub compatibility: MarkdownCompatibility,
    pub source_editor: Entity<InputState>,
    pub rich_editor: EditorHandle,
    pub shared: Arc<RwLock<MarkdownSharedState>>,
}

pub struct MarkdownSharedState {
    pub source: String,
    pub source_revision: u64,
    pub projected_revision: u64,
    pub persisted_revision: u64,
    pub disk_fingerprint: FileFingerprint,
    pub sync_state: MarkdownSyncState,
}
```

`InputState` 只能在 GPUI foreground 使用，不放入跨线程 shared state。Cditor persistence 后台任务只访问 `MarkdownSharedState` 和文件存储。

### 同步状态

```rust
pub enum MarkdownSyncState {
    Clean,
    SourceDirty,
    WysiwygDirty,
    SavingSource,
    SavingWysiwyg,
    Switching,
    Conflict(ExternalChange),
    Incompatible(Vec<MarkdownDiagnostic>),
    Failed(String),
}
```

任一时刻只允许一个写入方向：

- Source 模式只有 source editor 可修改内容。
- WYSIWYG 模式只有 Cditor 可修改内容。
- `Switching`、`Conflict`、`Failed` 状态禁止另一个方向写入。

## 持久化

### MarkdownFileStore

```rust
pub struct MarkdownFileStore {
    path: Arc<RwLock<PathBuf>>,
}
```

职责：

- UTF-8 读取。
- 同目录临时文件、flush、rename 原子写入。
- 写入前验证路径仍在 Notes root 内。
- 保存并比较文件 fingerprint。
- 重命名后更新共享路径。

### 文件 fingerprint

```rust
pub struct FileFingerprint {
    pub size: u64,
    pub modified_at: Option<SystemTime>,
    pub content_hash: [u8; 32],
}
```

每次覆盖前重新读取 fingerprint：

- 与 session 基线一致：允许写入。
- 不一致且本地 clean：重新加载外部内容。
- 不一致且本地 dirty：进入 Conflict，禁止覆盖。

首版至少在保存前和 Tab 激活时检查；后续可增加 watcher 和轮询。

### MarkdownDocumentPersistence

实现 Cditor `EditorPersistence`：

```rust
pub struct MarkdownDocumentPersistence {
    pub document_id: Uuid,
    pub store: MarkdownFileStore,
    pub shared: Arc<RwLock<MarkdownSharedState>>,
}
```

`load`：

1. 读取 `.md`。
2. 生成 compatibility report。
3. 解析为 `EditorDocument`。
4. 更新 shared source 和 fingerprint。

`save`：

1. 调用 `to_markdown_with_report`。
2. fidelity 为 `Unsupported` 时返回 persistence error，不写文件。
3. 检查外部 fingerprint。
4. 原子写入 `.md`。
5. 更新 shared source、revision 和 fingerprint。

Cditor 的 `EditorEvent::Saved` 用于通知 foreground，把最新 shared source 同步到源码编辑器，但仅当源码编辑器没有本地未提交修改时执行。

## 模式切换状态机

### Source 到 WYSIWYG

1. 设置 `sync_state = Switching`，禁用切换按钮。
2. 从 `InputState` 读取完整 source snapshot。
3. 保存原始 source 到 `.md`；即使 WYSIWYG 不兼容，也不丢失源码修改。
4. 调用 `from_markdown_with_report`。
5. 如果 `SourceOnly`，保持 Source 模式并显示 diagnostics。
6. 如果可编辑，调用 `EditorHandle::set_document`。
7. 更新 `projected_revision = source_revision`。
8. 切换到 WYSIWYG 并请求 Cditor focus。

### WYSIWYG 到 Source

1. 设置 `sync_state = Switching`。
2. 如果 Cditor dirty，触发 save 并等待 `Clean` 或 `SaveFailed`。
3. 保存失败或 exporter 返回 `Unsupported` 时保持 WYSIWYG，不覆盖源码编辑器。
4. 从 shared state 获取刚写入的 Markdown。
5. 更新 source editor；更新前确认 source editor 没有独立 dirty revision。
6. 切换到 Source 并聚焦源码输入框。

### 切换期间取消和关闭

- 切换操作绑定 generation ID，旧任务完成后不得覆盖新状态。
- Tab 关闭时等待当前切换结束。
- SourceDirty 先原子保存 `.md`。
- WysiwygDirty 走 Cditor save。
- Conflict、Unsupported 或 SaveFailed 时拒绝关闭并显示原因。

## 自动保存

### Source 模式

- 监听 `InputEvent::Change`。
- 500–1000ms debounce 后保存 source snapshot。
- 新输入增加 generation，旧 debounce task 自动作废。
- Source 模式保存不触发 Cditor parse。

### WYSIWYG 模式

- 继续使用 Cditor 一秒 autosave。
- persistence 负责导出 Markdown、检查兼容性和原子写入。
- 保存成功后更新 shared source。

避免在每次 source keystroke 后重建 Cditor；只在进入 WYSIWYG 时 parse。

## 外部修改和冲突

`.md` 是用户可从外部编辑器修改的开放格式，必须避免最后写入者静默覆盖。

冲突 UI：

```text
文件已被其他程序修改

[重新加载磁盘版本] [另存为] [保留当前内容]
```

首版行为：

- 重新加载：放弃当前内存 dirty 内容前二次确认。
- 另存为：创建新的 `.md` 和 document ID。
- 保留当前内容：不立即覆盖；用户必须查看差异并显式确认后才能强制保存。

不提供自动 merge，避免错误合并 Markdown 结构。

## 文件操作

### 新建

- RichText：创建空 `EditorDocument` 和 `.cditor.json`。
- Markdown：创建 UTF-8 `.md`，初始内容为空字符串或欢迎模板，并写入 document index。

### 重命名

- 扩展名由 format 固定，用户只编辑 stem。
- 更新文件、document index、UI state、editor cache 和 persistence path。
- Markdown session、source editor 和 rich editor handle 保持复用。

### 删除

- RichText dirty：先保存，失败则阻止。
- Markdown SourceDirty：先保存，失败则阻止。
- Markdown WysiwygDirty：先走 Cditor persistence，失败则阻止。
- Conflict 状态禁止删除，除非用户显式选择丢弃本地修改。

### 外部文件发现

- 扫描 `.md` 和 `.cditor.json`。
- 其他扩展名忽略。
- 符号链接继续忽略。
- 非 UTF-8 `.md` 进入加载失败状态，不自动转码或覆盖。

## Crate 和文件边界

Notes 新增或调整：

```text
crates/notes/src/
├── model.rs                       DocumentFormat、descriptor、UI state
├── document_index.rs              documents.json、reconcile、操作恢复
├── storage.rs                     格式分流和目录操作
├── rich_document_persistence.rs   现有 JSON persistence 重命名后迁入
├── markdown_file_store.rs         .md 原子 I/O、fingerprint、外部冲突
├── markdown_persistence.rs        Cditor Markdown persistence
├── markdown_session.rs            双视图同步状态机
├── markdown_source_editor.rs      InputState 创建、事件和 debounce
├── editor_cache.rs                RichText/Markdown session 缓存
├── notes_view.rs                  顶层 entity 和选择路由
├── notes_actions.rs               新建、重命名、删除、切换动作
└── notes_render.rs                工具栏、格式 badge、视图切换
```

Cditor 调整：

```text
crates/core/src/rich_text/markdown/
├── export.rs                      inline/block serializer
├── compatibility.rs               diagnostics 和 fidelity
└── tests.rs                       round-trip matrix

crates/app/src/integration/
├── document.rs                    with_report API
└── markdown.rs                    公共 result/diagnostic 类型
```

文件继续满足 Navop 的 300 行上限；复杂状态转换用独立 reducer/contract 文件，避免堆入 `NotesView`。

## 迁移

升级现有 Notes 时：

1. 检测 `documents.json` 不存在。
2. 扫描所有 `.cditor.json`，读取内部 ID 并建立索引。
3. 扫描已有 `.md`，分配 UUID 并登记；此前被忽略的 Markdown 文件自动出现在树中。
4. 保留 `notebook.json` 和 `state.json`。
5. 不转换现有 `.cditor.json`。
6. 默认视图仍打开上次选择的文档。

迁移先写临时 index，成功扫描全部文件后 rename；任一损坏富文本文档不阻止其他记录建立，但该文件登记为 damaged，并在 UI 中显示错误。

## 错误处理

- Markdown exporter 不兼容：保留 dirty，禁止覆盖 `.md`。
- Markdown parser diagnostics：保留源码，按 compatibility 决定是否允许 WYSIWYG。
- 外部文件改变：进入 Conflict，禁止自动覆盖。
- Source 保存失败：保留 SourceDirty 和编辑器内容。
- WYSIWYG 保存失败：保留 Cditor Dirty。
- 模式切换失败：留在原模式，不更新另一编辑器。
- index 损坏：备份原文件并从磁盘重建，但重建前不执行写操作。
- 文件和 index 更新中断：通过 pending operation 恢复。

## 测试策略

### Cditor round-trip

- 每种支持 block 的 parse → export → parse 语义等价。
- Inline marks 和嵌套 marks。
- Link escaping。
- Nested list 和 todo。
- Table pipe escaping。
- Code fence 内容中包含反引号。
- Unicode、中文和 CRLF。
- Unsupported block 返回 diagnostic，不输出静默降级结果。

### Notes 存储

- 同目录创建 RichText 和 Markdown。
- 扫描两种扩展名并隐藏扩展名。
- 同 stem 不互相覆盖。
- index 首次建立、外部文件发现和损坏恢复。
- rename pending operation 的完成、回滚和冲突。
- Markdown stable ID 在内部 rename 后不变。
- 路径越界、符号链接和非 UTF-8。

### Markdown session contract

- SourceDirty 自动保存成功和失败。
- WysiwygDirty exporter 成功、Unsupported 和保存失败。
- Source → WYSIWYG 成功、SourceOnly 和 parse diagnostics。
- WYSIWYG → Source 等待 save。
- 迟到 generation 不覆盖新状态。
- 外部 fingerprint 冲突不覆盖磁盘。
- close/delete 在 dirty、switching、conflict 状态下的门禁。

### GPUI 定向测试

- Markdown source editor 获得焦点并能输入 Enter。
- 视图切换后焦点交给目标编辑器。
- 切换期间按钮禁用。
- 格式 badge 和切换器只在正确文档上出现。
- 已打开 session 切换文档后保留 source undo 和 Cditor undo。

## 验收标准

1. 可以在同一笔记本创建 `.cditor.json` 和 `.md` 文档。
2. RichText 文档行为与当前版本一致。
3. Markdown 文档可以在 WYSIWYG 和 Source 之间切换。
4. Source 编辑内容原子保存到真实 `.md`。
5. WYSIWYG 编辑支持语法后，`.md` 保存并保持语义等价。
6. 不支持的富文本内容不会静默覆盖 `.md`。
7. Markdown 内部 rename 后 document ID、session 和 undo 状态保持。
8. 外部修改不会被自动覆盖。
9. 重启后恢复所选文档、目录展开和 Markdown 视图模式。
10. 现有富文本笔记无需迁移内容即可继续使用。

完成验证至少包括：

```bash
rtk cargo test -p cditor-core markdown
rtk cargo test -p cditor-app integration
rtk cargo test -p notes
rtk cargo check -p notes
rtk cargo test -p main notes
rtk cargo check -p main
rtk cargo fmt --all -- --check
rtk cargo clippy -p notes -p main --all-targets -- -D warnings
```

还需手工验证新建两种格式、Markdown 双视图切换、输入焦点、Enter、自动保存、外部文件冲突、重命名、关闭保护和重启恢复。

## 实施顺序

1. Cditor Markdown compatibility 和语义级 exporter。
2. Cditor round-trip 测试矩阵。
3. Notes `DocumentFormat` 和 `documents.json`。
4. 双扩展名扫描、创建、重命名和删除。
5. MarkdownFileStore、fingerprint 和外部冲突门禁。
6. MarkdownDocumentPersistence。
7. Markdown source editor。
8. MarkdownSession 状态机和模式切换。
9. UI 格式选择、badge、切换器和 diagnostics。
10. 迁移、恢复、关闭保护和完整验收。

该顺序先解决数据保真和持久化，再接 UI，避免先做出可切换界面却在保存时丢失 Markdown 内容。

## 2026-07-15 实现状态

### Notes 侧已完成

- 已加入 `DocumentFormat::{RichText, Markdown}`，文件树同时识别 `.cditor.json` 和 `.md`。
- 已加入 `documents.json`，Markdown 使用索引 UUID，RichText 从 Cditor JSON 读取内部 ID。
- create、scan、rename、delete 和 pending operation 恢复已支持两种格式。
- 已加入 `MarkdownFileStore`：UTF-8 读取、原子写入、SHA-256 fingerprint 和外部修改冲突门禁。
- 已加入 Markdown source editor：`code_editor("markdown")`、行号、多行、搜索、软换行。
- Markdown source editor 打开或从预览切回时使用 `Window::defer` 聚焦；Enter 由多行 `InputState` 处理。
- 已加入 generation debounce 自动保存；迟到任务不会清理或覆盖更新一代的修改状态。
- Markdown 默认打开 WYSIWYG；工具栏只保留一个“源码”切换按钮。按钮选中时显示 Source，再次点击取消选中并返回 WYSIWYG。
- 已接入 Cditor strict import/apply/export contract。
- `Editable` Markdown 可直接在 WYSIWYG 编辑；`EditableWithNormalization` 必须点击“允许规范化并编辑”；`SourceOnly` 只提供只读 projection。
- WYSIWYG 使用 `MarkdownDocumentPersistence` 和 Cditor autosave，通过 Strict export 写回同一个 `.md`；unsupported 内容和外部文件冲突都会阻止覆盖。
- Markdown view mode 已按稳定 document ID 保存到 `state.json`，内部 rename 后保持。
- RichText 和 Markdown session 都已接入 rename、delete 和 tab close 门禁。
- 工具栏当前使用“富文本 / Markdown / 目录”三个明确入口，避免在 Cditor contract 未完成前扩大 Dialog 状态；后续可无数据迁移地合并为单个带格式选择的 Dialog。
- 文件树用“富 / MD”轻量 badge 区分格式。

当前 Notes 侧模块边界以实际代码为准：

```text
crates/notes/src/
├── document_index.rs          documents.json 与稳定 ID
├── storage.rs                NotesStorage 公共操作
├── storage_support.rs        扫描、恢复和原子文件辅助
├── markdown_file_store.rs    .md I/O、fingerprint 与冲突检测
├── markdown_persistence.rs   Cditor Strict export persistence
├── markdown_adapter.rs       唯一 Cditor Markdown 接缝
├── markdown_session.rs       纯同步状态 contract
├── markdown_source.rs        InputState 创建和 change subscription
├── markdown_view.rs          session 生命周期、保存和切换
├── markdown_render.rs        Markdown 双模式 UI
├── notes_close.rs            RichText/Markdown 关闭门禁
├── notes_actions.rs          创建、重命名和删除联动
└── notes_render.rs           主布局、工具栏和文件树
```

### Cditor strict 集成已完成

- `markdown_adapter.rs` 已使用 `from_markdown_with_report`、`apply_markdown(ReadOnlyPreview)` 和 `export_markdown(Strict)`。
- Source projection replace 会重置 Cditor baseline，不产生 dirty/autosave 回声。
- WYSIWYG autosave 通过 `MarkdownDocumentPersistence` 写回 `.md`，并与 Source editor 共用串行化 fingerprint 状态。
- WYSIWYG → Source 使用 Strict export 更新源码 InputState，再进入 generation debounce 保存。
- close/delete 同时检查 Source session 状态和 Cditor preview dirty/save state。
- normalization acceptance、SourceOnly read-only 和 diagnostics 数量已显示在 Markdown toolbar。

Notes 其他模块不得直接调用当前有损的 `EditorHandle::get_markdown()` 或 `EditorDocument::to_markdown()` 保存 `.md`。Cditor 公共 API 变化只应修改 `markdown_adapter.rs` 和必要的状态转换，不应扩散到 storage、file store 或文档索引。

### 当前自动验证

已执行：

```bash
rtk cargo test -p notes
rtk cargo check -p notes
rtk cargo clippy -p notes --all-targets --no-deps -- -D warnings
rtk cargo fmt --all
rtk git diff --check
```

最新结果：Notes 20 个测试通过，包含 Source/WYSIWYG 共用文件 store、Cditor Strict persistence 和外部冲突回归测试。Notes 编译和自身 Clippy 通过。工作区依赖仍会报告既有 future-incompatibility 提示；不属于本次 Notes 改动。按用户要求，本阶段未启动 GUI，焦点、Enter、切换和实际交互由用户手工验证。
