# Notes Cditor Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Navop Home 侧边栏提供唯一 Notes Tab，以本地多级目录树管理 Cditor 原生文档并在右侧完成富文本编辑和自动保存。

**Architecture:** 新 `notes` crate 唯一拥有笔记本元数据、路径策略、文件树状态、本地持久化和 Cditor `EditorHandle`；`main` 只注册 Notes 侧边栏入口与稳定 Tab。Cditor 通过固定 Git revision 直接依赖，Navop Cargo patch 负责统一传递依赖中的 GPUI revision。

**Tech Stack:** Rust 2024、GPUI、gpui-component、Cditor、serde/serde_json、chrono、uuid、dirs、tempfile。

## Global Constraints

- Cditor Git revision 固定为 `170ab124c14284a8d4bc6571141590f2d2e02650`。
- GPUI revision 必须唯一解析为 `76c93968da5b8b8809bdd72e4ad9e7d0e946bad0`。
- 文档只保存 Cditor 原生 JSON，不生成 Markdown，不引入数据库。
- 所有文件操作限制在 `dirs::data_local_dir()/navop/notes` 内，禁止跟随符号链接或接受绝对路径、`.`、`..` 和路径分隔符名称。
- 函数不超过 50 行、文件不超过 300 行、嵌套不超过 3 层、位置参数不超过 3 个。
- 新行为按 TDD 执行；测试先失败，再写最小实现。
- 不修改用户主工作树中的 cloud sync 文件。

---

### Task 1: 依赖统一与 notes crate 骨架

**Files:**
- Modify: `Cargo.toml`
- Modify: `main/Cargo.toml`
- Create: `crates/notes/Cargo.toml`
- Create: `crates/notes/src/lib.rs`
- Create: `crates/notes/locales/notes.yml`

**Interfaces:**
- Consumes: Cditor `Editor`、`EditorHandle`、`EditorPersistence`；workspace GPUI。
- Produces: workspace crate `notes`；`notes::NotesView` 在后续任务实现。

- [ ] **Step 1: 添加依赖解析 contract**

  在 `crates/notes/src/lib.rs` 添加只编译公开 API 的测试：

  ```rust
  #[cfg(test)]
  mod tests {
      use cditor_app::{Editor, EditorDocument, EditorHandle, EditorPersistence};

      #[test]
      fn cditor_public_integration_api_is_available() {
          fn assert_persistence<T: EditorPersistence>() {}
          let _ = Editor::builder;
          let _ = EditorDocument::from_json;
          let _ = std::mem::size_of::<EditorHandle>();
          let _ = assert_persistence::<CompileOnlyPersistence>;
      }
  }
  ```

  `CompileOnlyPersistence` 在同一测试模块实现 `load` 和 `save`，均返回成功。

- [ ] **Step 2: 运行测试并确认 crate 尚不存在**

  运行：`rtk cargo test -p notes cditor_public_integration_api_is_available`

  预期：FAIL，Cargo 报告 workspace 中不存在 `notes` package。

- [ ] **Step 3: 创建 crate 并固定 Cditor revision**

  根 `Cargo.toml` 增加 workspace member 和依赖：

  ```toml
  cditor-app = { git = "https://github.com/feigeCode/Cditor.git", rev = "170ab124c14284a8d4bc6571141590f2d2e02650" }
  notes = { path = "crates/notes" }
  ```

  `crates/notes/Cargo.toml` 至少包含：

  ```toml
  [package]
  name = "notes"
  version = "0.1.0"
  publish.workspace = true
  edition.workspace = true

  [dependencies]
  anyhow.workspace = true
  cditor-app.workspace = true
  chrono.workspace = true
  dirs.workspace = true
  gpui.workspace = true
  gpui-component.workspace = true
  one-core.workspace = true
  rust-i18n.workspace = true
  serde.workspace = true
  serde_json.workspace = true
  tracing.workspace = true
  uuid.workspace = true

  [dev-dependencies]
  gpui = { workspace = true, features = ["test-support"] }
  tempfile = "3"

  [lints]
  workspace = true
  ```

- [ ] **Step 4: 验证只有一套 GPUI**

  运行：

  ```bash
  rtk cargo test -p notes cditor_public_integration_api_is_available
  rtk cargo tree -p notes -i gpui
  ```

  预期：测试 PASS；tree 中所有 `gpui` 均指向 `76c93968…`。如果出现两个 revision，只调整根 Cargo patch，不修改 Cditor Rust 源码。

- [ ] **Step 5: 提交依赖骨架**

  ```bash
  rtk git add Cargo.toml Cargo.lock crates/notes
  rtk git commit -m "feat(notes): add Cditor integration crate"
  ```

### Task 2: 本地模型、路径策略与笔记本创建

**Files:**
- Create: `crates/notes/src/model.rs`
- Create: `crates/notes/src/path_policy.rs`
- Create: `crates/notes/src/storage.rs`
- Create: `crates/notes/src/storage_tests.rs`
- Modify: `crates/notes/src/lib.rs`

**Interfaces:**
- Produces: `NotebookMetadata`、`NotebookUiState`、`FileNode`、`NodeKind`、`NotesStorage`。
- `NotesStorage::open(root: PathBuf) -> Result<Self>`
- `NotesStorage::create_notebook(&self, name: &str, description: &str) -> Result<NotebookMetadata>`
- `NotesStorage::load_notebook(&self) -> Result<Option<NotebookMetadata>>`
- `NotesStorage::scan_tree(&self) -> Result<Vec<FileNode>>`

- [ ] **Step 1: 写失败测试**

  在 `storage_tests.rs` 覆盖：首次创建生成 `notebook.json`/`state.json`/`files/欢迎.cditor.json`；重新加载元数据相等；空目录出现在扫描结果；拒绝 `""`、`".."`、`"a/b"` 和 `.cditor.json` 后缀。

  核心断言：

  ```rust
  assert_eq!("My Notes", loaded.name);
  assert!(root.join("files/欢迎.cditor.json").is_file());
  assert!(validate_node_name("../escape").is_err());
  ```

- [ ] **Step 2: 运行失败测试**

  运行：`rtk cargo test -p notes storage_tests`

  预期：FAIL，模型与 `NotesStorage` 尚不存在。

- [ ] **Step 3: 实现模型与路径校验**

  `FileNode` 使用稳定相对路径：

  ```rust
  pub struct FileNode {
      pub relative_path: PathBuf,
      pub display_name: String,
      pub kind: NodeKind,
      pub children: Vec<FileNode>,
  }

  pub enum NodeKind {
      Directory,
      Document,
  }
  ```

  `validate_node_name` trim 后拒绝空白、`.`、`..`、`/`、`\\`、NUL 和保留后缀。

- [ ] **Step 4: 实现原子 JSON 与首次创建**

  `write_json_atomic(path, value)` 在同目录创建 `.<name>.tmp`，写入、flush 后 rename。首次文档由：

  ```rust
  EditorDocument::from_markdown(document_id.to_string(), "# 欢迎\n\n开始记录。")
  ```

  生成，并保存原生 JSON。

- [ ] **Step 5: 运行测试并提交**

  ```bash
  rtk cargo test -p notes storage_tests
  rtk git add crates/notes/src
  rtk git commit -m "feat(notes): add local notebook storage"
  ```

### Task 3: 目录和文档操作契约

**Files:**
- Modify: `crates/notes/src/storage.rs`
- Modify: `crates/notes/src/storage_tests.rs`
- Modify: `crates/notes/src/path_policy.rs`

**Interfaces:**
- `create_directory(&self, parent: &Path, name: &str) -> Result<PathBuf>`
- `create_document(&self, parent: &Path, name: &str) -> Result<DocumentDescriptor>`
- `rename_node(&self, relative_path: &Path, new_name: &str) -> Result<PathBuf>`
- `delete_node(&self, relative_path: &Path) -> Result<DeleteSummary>`

- [ ] **Step 1: 写失败测试**

  覆盖多级目录、同级重名、空目录重命名、文档扩展名隐藏、递归删除计数、绝对路径和 `..` 越界拒绝、符号链接忽略。

  ```rust
  let work = storage.create_directory(Path::new(""), "工作")?;
  let document = storage.create_document(&work, "项目计划")?;
  assert_eq!(Path::new("工作/项目计划.cditor.json"), document.relative_path);
  ```

- [ ] **Step 2: 运行测试确认失败**

  运行：`rtk cargo test -p notes storage_tests::node_operations`

- [ ] **Step 3: 实现根目录边界和文件操作**

  所有入口先调用 `resolve_relative_path`；已存在父目录 canonicalize 后必须 `starts_with(canonical_files_root)`。扫描使用 `symlink_metadata`，符号链接直接跳过。

- [ ] **Step 4: 运行存储测试并提交**

  ```bash
  rtk cargo test -p notes storage_tests
  rtk git add crates/notes/src/storage.rs crates/notes/src/storage_tests.rs crates/notes/src/path_policy.rs
  rtk git commit -m "feat(notes): manage local folders and documents"
  ```

### Task 4: Cditor 文件持久化与编辑器缓存

**Files:**
- Create: `crates/notes/src/document_persistence.rs`
- Create: `crates/notes/src/document_persistence_tests.rs`
- Create: `crates/notes/src/editor_cache.rs`
- Modify: `crates/notes/src/lib.rs`

**Interfaces:**
- Produces: `FileDocumentPersistence::new(path: PathBuf)`；实现 `EditorPersistence`。
- Produces: `EditorCache::open(descriptor, window, cx) -> Result<EditorHandle>`。

- [ ] **Step 1: 写 persistence 失败测试**

  使用 TempDir 覆盖 load 不存在返回 None、save/load 原生 JSON round-trip、document ID 不因路径 rename 改变、保存失败保留原文件。

  ```rust
  let saved = persistence.load("stable-id")?.expect("saved document");
  assert_eq!("stable-id", saved.document_id);
  ```

- [ ] **Step 2: 运行失败测试**

  运行：`rtk cargo test -p notes document_persistence_tests`

- [ ] **Step 3: 实现 persistence**

  `load` 读取绑定路径并用 `EditorDocument::from_json`；`save` 校验 request document ID 后调用共享原子写入。错误统一映射到 `EditorPersistenceError::new`。

- [ ] **Step 4: 实现 EditorCache**

  cache 以稳定 document ID 为 key：

  ```rust
  let handle = Editor::builder()
      .document_id(descriptor.document_id.clone())
      .persistence(FileDocumentPersistence::new(descriptor.absolute_path.clone()))
      .autosave(Duration::from_secs(1))
      .build(cx)?;
  ```

  再次打开返回已有 handle。

- [ ] **Step 5: 运行测试并提交**

  ```bash
  rtk cargo test -p notes document_persistence_tests
  rtk cargo check -p notes
  rtk git add crates/notes/src
  rtk git commit -m "feat(notes): persist Cditor documents locally"
  ```

### Task 5: 文件树状态与 Notes Tab UI

**Files:**
- Create: `crates/notes/src/tree_state.rs`
- Create: `crates/notes/src/tree_state_tests.rs`
- Create: `crates/notes/src/tree_view.rs`
- Create: `crates/notes/src/notes_view.rs`
- Create: `crates/notes/src/notes_render.rs`
- Modify: `crates/notes/src/lib.rs`
- Modify: `crates/notes/locales/notes.yml`

**Interfaces:**
- Produces: `NotesView::new(window, cx)`。
- `TreeState::project(nodes, expanded) -> Vec<TreeRow>` 保留空目录。
- `NotesView` 实现 `Render`、`Focusable`、`EventEmitter<TabContentEvent>`、`TabContent`。

- [ ] **Step 1: 写树状态失败测试**

  覆盖空目录显示、展开后深度、折叠隐藏后代、删除活动文档后选择相邻文档、state.json 恢复。

  ```rust
  assert_eq!(NodeKind::Directory, rows[0].kind);
  assert_eq!(0, rows[0].depth);
  assert_eq!(1, rows[1].depth);
  ```

- [ ] **Step 2: 运行失败测试并实现纯状态**

  ```bash
  rtk cargo test -p notes tree_state_tests
  ```

  实现后重复运行直到 PASS。

- [ ] **Step 3: 实现 NotesView 加载状态**

  状态明确分为：

  ```rust
  enum NotesLoadState {
      Loading,
      NeedsSetup,
      Ready,
      Failed(String),
  }
  ```

  目录扫描通过 `cx.background_spawn`，完成后回到 foreground 更新 tree rows 和选中文档。

- [ ] **Step 4: 实现创建表单和布局**

  首次创建表单包含名称、描述和 Create。Ready 页面使用外层 `h_flex().size_full().min_h_0().overflow_hidden()`；左侧固定约 260px，内部列表滚动；右侧 `.flex_1().min_w_0().h_full()` 渲染活动 `EditorHandle::entity()`。

- [ ] **Step 5: 实现树操作菜单**

  顶部按钮和行 context menu 调用 storage：新建目录、新建文档、重命名、删除。成功后重新扫描；失败显示 Notification。删除打开中的 Dirty 文档前先保存，失败时取消删除。

- [ ] **Step 6: 实现 TabContent 与关闭保护**

  `content_key()` 返回 `"Notes"`，title 使用笔记本名称，icon 使用 `IconName::BookOpen`。`try_close` 遍历 dirty handles，触发保存并在任一失败时返回 false。

- [ ] **Step 7: 运行定向验证并提交**

  ```bash
  rtk cargo test -p notes
  rtk cargo check -p notes
  rtk git add crates/notes
  rtk git commit -m "feat(notes): add notebook tree and Cditor tab"
  ```

### Task 6: Home 侧边栏入口与唯一 Tab

**Files:**
- Modify: `main/Cargo.toml`
- Modify: `main/src/home_tab.rs`
- Modify: `main/src/home/home_tabs.rs`
- Modify: `main/locales/main.yml`
- Add focused tests near `main/src/home/home_tabs.rs`

**Interfaces:**
- Consumes: `notes::NotesView::new`。
- Produces: `HomePage::add_notes_tab(window, cx)`；稳定 Tab ID `notes`。

- [ ] **Step 1: 写失败 contract**

  测试源码 contract 包含 `Button::new("open_notes")`、`IconName::BookOpen`、`add_notes_tab` 和 `activate_or_add_tab_lazy("notes", ...)`，并断言 Notes 入口位于 Extensions 之前。

- [ ] **Step 2: 运行失败测试**

  运行：`rtk cargo test -p main notes`

- [ ] **Step 3: 实现唯一 Tab 打开入口**

  ```rust
  pub(crate) fn add_notes_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
      let tab_container = self.active_tab_container(cx);
      window.defer(cx, move |window, cx| {
          tab_container.update(cx, |tabs, cx| {
              tabs.activate_or_add_tab_lazy(
                  "notes",
                  |window, cx| TabItem::new("notes", "home", cx.new(|cx| NotesView::new(window, cx))),
                  window,
                  cx,
              );
          });
      });
  }
  ```

- [ ] **Step 4: 增加侧边栏按钮和本地化**

  使用 `BookOpen`，collapsed 显示 tooltip，expanded 显示 `Home.notes`。点击调用 `add_notes_tab`。

- [ ] **Step 5: 运行验证并提交**

  ```bash
  rtk cargo test -p main notes
  rtk cargo check -p main
  rtk git add main Cargo.toml Cargo.lock
  rtk git commit -m "feat(main): add Notes sidebar entry"
  ```

### Task 7: 审查、运行验收与完成验证

**Files:**
- Modify: `AGENTS.md` only if implementation reveals a reusable project rule not already documented.
- Review: all files changed since `12af710e`.

**Interfaces:**
- Produces: 完整验证证据和未解决风险清单。

- [ ] **Step 1: 静态和测试验证**

  ```bash
  rtk cargo tree -p notes -i gpui
  rtk cargo test -p notes
  rtk cargo check -p notes
  rtk cargo test -p main notes
  rtk cargo check -p main
  rtk cargo fmt --all -- --check
  rtk cargo clippy -p notes -p main --all-targets -- -D warnings
  ```

- [ ] **Step 2: 结构指标检查**

  运行 `rtk find crates/notes/src -name '*.rs'` 和 `rtk wc -l`，确认每个文件不超过 300 行；人工检查新增函数不超过 50 行、嵌套不超过 3 层、位置参数不超过 3 个。

- [ ] **Step 3: 运行应用验收**

  启动 Navop，验证 Notes 唯一 Tab、首次创建名称/描述、空目录、新建嵌套目录和文档、重命名、删除确认、文件切换、Cditor 输入、自动保存及重启恢复。

- [ ] **Step 4: 请求代码审查并处理反馈**

  审查基线 `12af710e` 到当前 HEAD，修复全部 Critical/Important 问题，并重新运行受影响验证。

- [ ] **Step 5: completion verification**

  按设计文档逐项核对显式需求和命令证据；只有所有要求均有直接证据时才声明完成。

