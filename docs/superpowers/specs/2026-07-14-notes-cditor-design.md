# Notes 与 Cditor 集成设计

## 背景

Navop 当前没有本地笔记入口。用户希望从 Home 侧边栏直接打开一个笔记本 Tab，在左侧管理多级目录和文档，在右侧使用 Cditor 富文本编辑器，并把全部内容保存在本地。

Cditor 已公开面向第三方 GPUI 宿主的 `Editor`、`EditorHandle`、`EditorDocument` 和 `EditorPersistence`。兼容性 spike 已把 Cditor 的 Zed revision 从 `1d217ee39d381ac101b7cf49d3d22451ac1093fe` 对齐为 Navop 使用的 `76c93968da5b8b8809bdd72e4ad9e7d0e946bad0`，结果为 `cditor-app` 编译通过且 250 个库测试通过。因此 Notes 不复制编辑器内部实现，只通过公开集成 API 嵌入 Cditor。

## 目标

- Home 侧边栏新增 Notes 入口，点击时创建或激活唯一 Notes Tab。
- 首次打开时创建一个包含名称和描述的本地笔记本。
- Notes Tab 左侧展示可展开的多级目录和文档树，右侧显示当前文档的 Cditor 编辑器。
- 支持新建目录、新建文档、重命名和递归删除。
- 点击文档时切换到对应编辑器，并保留已打开文档的光标、选区、撤销历史和自动保存状态。
- 使用 Cditor 原生 JSON 无损保存文档，不生成 Markdown 镜像，不引入数据库。
- 所有路径保持在 Notes 根目录内，禁止绝对路径、`..` 和符号链接越界。

## 非目标

- 首版不提供多个笔记本、笔记本列表或云同步。
- 首版不提供拖拽移动、剪切粘贴、全文搜索和 Markdown 导入导出。
- 首版不暴露 Cditor 内部 `DocumentRuntime`，也不复制 Cditor 源码到 Navop。
- 首版不把 Notes 注册为 extension contribution。

## 用户体验

### 打开与首次创建

Home 侧边栏底部在 Extensions 之前显示 Notes。点击 Notes 使用稳定 Tab ID `notes` 调用 `activate_or_add_tab_lazy`。如果 Tab 已存在，只激活现有 Tab。

本地不存在 `notebook.json` 时，Tab 显示名称和描述表单。创建成功后写入元数据、创建 `files/` 目录和默认文档“欢迎”，随后直接打开编辑器。后续打开时恢复笔记本、展开目录和最后选中文档。

### 页面布局

```text
┌ 笔记本名称 ─ 描述 ─ 保存状态 ─ 编辑信息 ┐
├─────────────────┬────────────────────────┤
│ 新建文档  新建目录 │                        │
│                 │                        │
│ ▼ 工作          │      Cditor Editor     │
│   项目计划       │                        │
│ ▶ 学习          │                        │
│   欢迎           │                        │
└─────────────────┴────────────────────────┘
```

左侧面板使用明确宽度、`min_h_0` 和 `overflow_hidden` 边界，内部列表独立滚动。空目录仍必须显示为目录，因此 Notes 使用自己的扁平树投影，不依赖 `gpui_component::TreeItem::is_folder()` 的“有子项才是目录”语义。

### 文件操作

- 新建目录：在当前目录或根目录下创建真实文件系统目录。
- 新建文档：创建空的 Cditor `EditorDocument`，文件扩展名为 `.cditor.json`，界面隐藏扩展名并立即打开。
- 重命名：校验空名称、保留名称、路径分隔符和同级冲突后执行文件系统 rename。
- 删除：文档或目录删除前确认；目录显示递归影响数量。活动文档 Dirty 时先保存，保存失败则阻止删除。
- 展开/折叠：目录状态写入 `state.json`；重新扫描后按稳定相对路径恢复。

## 本地数据格式

默认根目录为 `dirs::data_local_dir()/navop/notes`：

```text
notes/
├── notebook.json
├── state.json
└── files/
    ├── 欢迎.cditor.json
    └── 工作/
        └── 项目计划.cditor.json
```

`notebook.json`：

```rust
pub struct NotebookMetadata {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

`state.json`：

```rust
pub struct NotebookUiState {
    pub selected_document: Option<PathBuf>,
    pub expanded_directories: BTreeSet<PathBuf>,
}
```

目录树直接映射 `files/` 下的真实目录。每个 `.cditor.json` 文件保存一个带稳定 `document_id` 的 `EditorDocument`。重命名路径不修改文档 ID。

JSON 写入采用同目录临时文件、flush 和 rename 的原子替换流程。解析失败时不覆盖原文件，并把错误暴露给界面。

## Crate 边界

新增 `crates/notes`：

```text
src/
├── lib.rs                  公共导出与 i18n
├── model.rs                元数据、节点和 UI 状态
├── path_policy.rs          名称与根目录边界校验
├── storage.rs              笔记本、目录树和原子文件操作
├── document_persistence.rs Cditor EditorPersistence 适配
├── tree_state.rs           展开、选择和扁平投影
├── tree_view.rs            左侧文件树渲染与操作菜单
├── notes_view.rs           Entity 状态、异步加载和编辑器缓存
└── notes_render.rs         页面布局、创建表单和错误状态
```

`main` 只依赖 `notes::NotesView`，负责 Home 入口和唯一 Tab 注册。`notes` 依赖 `cditor-app`、GPUI、gpui-component、one-core、serde、chrono、uuid 和 dirs。

## 编辑器生命周期

`NotesView` 以文档相对路径为 key 缓存 `EditorHandle`。首次选择文档时使用其稳定 `document_id`、路径绑定的 `EditorPersistence` 和一秒 autosave 创建编辑器。再次选择时复用 handle，避免丢失撤销历史。

Cditor 的 persistence trait 在自身后台任务中调用同步文件 I/O；目录扫描和非 Cditor 文件操作使用 GPUI background task，结果回到 foreground 更新 Entity。渲染和输入路径不直接读取目录或写文件。

关闭 Notes Tab 时，如果任何缓存编辑器 Dirty，则逐个调用 save 并等待保存状态结束；失败时显示错误并拒绝关闭。

## 错误处理与安全

- 创建根目录失败时显示完整路径和错误，不进入可编辑状态。
- 非 UTF-8 文件名保持可见的 lossy label，但内部始终使用 `PathBuf`。
- 名称拒绝空白、`.`、`..`、路径分隔符和 `.cditor.json` 保留后缀。
- 所有操作先 canonicalize 已存在父目录并验证位于 canonical Notes 根目录内。
- 扫描时不跟随符号链接；发现符号链接时忽略并记录 warning。
- 损坏文档只进入加载失败视图，原文件保持不变。
- 删除活动目录前统计其中打开的文档，保存失败时整次删除取消。

## 测试与验收

纯存储测试使用 `tempfile::TempDir`，覆盖首次创建、元数据 round-trip、多级目录、空目录、文档创建、重命名冲突、递归删除、路径越界、符号链接和损坏 JSON。

Cditor 适配测试使用 fake/临时文件 persistence，覆盖原生文档 load/save round-trip、稳定 document ID、原子替换和保存失败不破坏原文件。

树状态测试覆盖展开、折叠、空目录投影、选择恢复、删除后回退选择。Main contract 测试覆盖 Notes 入口存在、稳定 Tab ID 和重复点击复用。

完成验证包括：

```bash
rtk cargo tree -p notes -i gpui
rtk cargo test -p notes
rtk cargo check -p notes
rtk cargo test -p main notes
rtk cargo check -p main
rtk cargo fmt --all -- --check
rtk cargo clippy -p notes -p main --all-targets -- -D warnings
```

验收时还需启动 Navop，确认首次创建、目录展开、新建/重命名/删除、文件切换、输入后自动保存和重启恢复。

