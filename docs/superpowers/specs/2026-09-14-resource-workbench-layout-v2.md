# Resource Workbench v2：区域化布局与全层级 Shell 覆盖

日期：2026-09-14。状态：**定稿，随本次实现落地**。范围：`navop/crates/extension-runtime`、`navop/crates/resource_view`、`navop/crates/universal-plugins`、`navop-extensions/extensions/composite/*`。

前置阅读：`2026-09-09-native-resource-workbench-design.md`（生命周期、session 所有权、dispatcher 原则不变，本文不重复）；`2026-09-09-native-resource-workbench-contract.md` §4 的 `bodyRenderer` 设想由本文实现并取代。

背景：扩展尚未发布，无兼容包袱。v1 的 `navigation`（扁平列表，唯一渲染来源）、`tree`（声明并校验但从未渲染）、per-page `tabs`（Docker 每页复制完整 8 项列表）、`statusBar`（宿主硬编码 Docker 字段）全部收敛进一个新的 `layout` 声明。

## 1. 决策摘要

**布局 = 固定区域集合；每个区域的内容源要么是 native 声明式模板，要么是 JS Shell 视图；含整个工作台主体。**

- 四个区域槽：`left` / `center` / `right` / `bottom`。不做任意 split 树。需要自由分屏时把区域模型映射成 dock 节点，升级路径不封死。
- 每区域 `source.kind`：`native` 系（`list` / `tree` / `pages` / `status`）或 `shell`（JS 视图，复用 `CustomPageHost::mount` 与 `navop.workbench` host module）。
- 逃生舱：`layout.renderer = {kind: "shell", viewId}` → 整个工作台主体（四区域）归 JS。宿主永不可覆盖部分：连接 Tab/Header、session 所有权、权限校验、任务入口、关闭守卫。
- `operations` / `pages` 声明在任何布局下都强制有效——它们是权限与降级的单一事实来源，纯 Shell 工作台也必须声明。

## 2. v2 Manifest

### 2.1 顶层变化

```text
ResourceWorkbenchContrib:
  schemaVersion: 2                    # v1 拒绝安装
  + layout: Option<ResourceWorkbenchLayout>
  - navigation                        # 删除,由 layout.left(list) 取代
  - tree                              # 删除,由 layout.left(tree).roots 取代
  - statusBar                         # 删除,由 layout.bottom(status) 取代
  pages[].tabs                        # 删除,由 layout.center.tabGroups 取代
  pages[] 其余字段不变
```

`layout` 缺省时的等价默认（简单插件零负担）：

```json
{ "left": {"source": {"kind": "list"}}, "center": {"source": {"kind": "pages"}} }
```

### 2.2 Layout DTO

```rust
ResourceWorkbenchLayout {
    renderer: Option<ResourceWorkbenchRenderer>,   // root shell 覆盖;存在时不得再声明任何区域
    left:    Option<ResourceWorkbenchLeftRegion>,  // width/resizable + source
    center:  Option<ResourceWorkbenchCenterRegion>,
    right:   Option<ResourceWorkbenchSideRegion>,  // 同 left
    bottom:  Option<ResourceWorkbenchBottomRegion>,
}
```

**left/right source**：

- `{"kind": "list"}` — 现有 `navigation` 扁平列表的等价物。
- `{"kind": "tree", "roots": [...]}` — 树。根节点 `TreeRoot { id, title, pageId, children: Option<TreeChildren> }`；`TreeChildren { operation, itemsPath, keyPaths, labelPath, open, children: Option<TreeChildren> }` 递归嵌套形成多级 lazy 树，lazy 拉取走现有 dispatcher；绑定源新增 `parent`（父节点行数据）。**新增** `children.open: Option<ResourceWorkbenchOpen>` — 子节点点击跳转声明（`pageId` + selection 绑定 route，复用 v1 collection `open` 类型）；缺省回退根节点 `pageId` + 行数据作 route。
- `{"kind": "shell", "viewId", "fallback"}` — JS 视图占满该侧区域。
- `{"kind": "none"}` — 不渲染该区域。

**center source**：

- `{"kind": "pages", "tabGroups": [...]}` — 页面区（现状）。`TabGroup { id, tabs: Vec<ResourceWorkbenchTab> }`（tab 项复用 v1 类型：id/title/pageId/route）。page 新增可选 `tabGroupId`；strip 渲染顺序：page.tabGroupId 命中的组 → v1 兼容不需要（扩展未发布，直接只认 tabGroups）→ 无则不渲染 strip。
- `{"kind": "shell", ...}` — JS 视图占满中央区。

**bottom source**：

- `{"kind": "status", "items": [...]}` — `StatusItem { path, label, format }`，`format ∈ {raw|number|bytes|percent|version}`。数据仍由单一 `operation` 拉取（沿用 v1 `statusBar.operation` 语义，operation 字段在 `ResourceWorkbenchBottomRegion` 上）。`engine`/`server_version` 等旧硬编码键全部由 items 声明表达。
- `{"kind": "shell", ...}` / `{"kind": "none"}`。

**width/resizable**：`left/right` 可声明 `width`（默认 208）/ `bottom` 可声明 `height`（默认 28）；`resizable` 首版只接受声明、不做拖拽（预留字段，不宣称可用）。

### 2.3 示例：Docker（tree + tabGroups + status）

```json
{
  "schemaVersion": 2,
  "id": "docker",
  "layout": {
    "left": {
      "width": 260,
      "source": {
        "kind": "tree",
        "roots": [
          { "id": "containers", "title": "Containers", "pageId": "containers",
            "children": { "operation": "listContainers", "itemsPath": "/containers",
                          "keyPaths": ["/id"], "labelPath": "/name",
                          "open": { "pageId": "container-detail",
                                    "route": {"id": {"source": "selection", "path": "/id", "type": "string"}} } } },
          { "id": "overview-root", "title": "Overview", "pageId": "overview" },
          { "id": "images", "title": "Images", "pageId": "images" }
        ]
      }
    },
    "center": {
      "source": { "kind": "pages", "tabGroups": [
        { "id": "container", "tabs": [
            {"id": "inspect", "title": "Inspect", "pageId": "container-detail",
             "route": {"id": {"source": "route", "path": "/id", "type": "string"}}},
            {"id": "logs", "title": "Logs", "pageId": "container-logs",
             "route": {"id": {"source": "route", "path": "/id", "type": "string"}}},
            {"id": "exec", "title": "Exec", "pageId": "container-exec",
             "route": {"id": {"source": "route", "path": "/id", "type": "string"}}}
        ]}
      ]}
    },
    "bottom": {
      "source": { "kind": "status", "operation": "systemUsage", "items": [
        {"path": "/engine", "label": "Engine", "format": "boolean-up"},
        {"path": "/server_version", "label": "Version", "format": "version"},
        {"path": "/containers_running", "label": "Containers", "format": "pair", "otherPath": "/containers_total"},
        {"path": "/disk_used_bytes", "label": "Disk", "format": "bytes"},
        {"path": "/containers_memory_bytes", "label": "RAM", "format": "bytes"},
        {"path": "/containers_cpu_percent", "label": "CPU", "format": "percent"}
      ]}
    }
  }
}
```

### 2.4 示例：纯 JS 工作台

```json
{
  "schemaVersion": 2,
  "layout": { "renderer": {"kind": "shell", "viewId": "es-workspace"} },
  "operations": {...}, "pages": [...]
}
```

激活条件：同扩展 `shellViews` 含该 viewId、声明 `Context` + `Workbench` 模块、通过嵌入兼容性校验（与 v1 页面级 shell 相同规则）。root shell 激活时不挂载任何 native 区域；宿主仍渲染连接 Header、任务入口、关闭守卫。**root 级无 fallback**——viewId 不可用时显示"工作台视图不可用"错误态 + 说明，不伪装成可用 native 工作台（v1 页面级 `fallback: "native"` 语义保留在页面级；区域级 shell fallback 只能 `none`，即显示占位空态）。

### 2.5 删除的 v1 字段（不做兼容）

`navigation`、`tree`、`statusBar`、`pages[].tabs`。schemaVersion=1 的 manifest 在注册期直接拒绝并报"unsupported schemaVersion; migrate to layout"。四个 composite 扩展随宿主一次性迁移。

## 3. 校验规则（registration.rs，追加在现有校验后）

1. `schemaVersion == 2`。
2. `layout.renderer` 存在 → `left/center/right/bottom` 必须全部为 None（互斥）。
3. `layout.renderer` / 区域 shell source → viewId 必须在同扩展 `shellViews`，且模块声明为 `Context` + `Workbench`、不含 raw 模块（复用 v1 页面级校验，提取为公共函数）。
4. `tabGroups[].tabs[].pageId` 存在；`pages[].tabGroupId` 若声明必须命中某个组 id。
5. `left.kind == "tree"` → `roots` 非空；每个 root 的 `pageId` 存在；`children.operation` 存在；`children.open.pageId` 存在。
6. `left.kind == "list"` → 至少一个 page（渲染空列表无意义，注册期不强制多项，仅此一条存在性兜底）。
7. `bottom.kind == "status"` → `operation` 必须存在且为 invoke 模式；`items` 非空；`format ∈ 枚举`；`pair` 格式要求 `otherPath`。
8. `right`/`bottom` 区域：首版禁止声明（DTO 存在、枚举开放，但注册校验对 composite 之外的组合保守放行——实际按 DTO 全量放开，错误组合靠 deny_unknown_fields + kind 枚举闭合）。

修正：第 8 条删除。四区域对全部 kind 开放，靠 2-7 与类型系统闭合即可，不人为设限。

## 4. 宿主实现（crates/resource_view）

### 4.1 模块拆分

```text
crates/resource_view/src/
├── lib.rs              # NativeResourceWorkbench:状态机、load/dispatch/导航(保留)
├── layout.rs           # 新:ResolvedLayout 解析(Registered 声明 → 渲染用区域枚举)
├── regions/
│   ├── mod.rs
│   ├── nav_list.rs     # 现 render_nav 迁入
│   ├── nav_tree.rs     # 新:树渲染 + lazy children(复用 dispatcher + items_of 投影)
│   ├── page_area.rs    # 现 render_page 主体迁入;strip 查 tabGroups
│   ├── status.rs       # 新:items 驱动状态栏
│   └── shell_region.rs # 新:区域级 shell 挂载(按 region id 缓存 MountHandle)
└── (现有 collection_table/query_page/terminal_host/message_event/route_binding/custom_page_host 不动)
```

### 4.2 布局解析

`ResolvedLayout` 在 `NativeResourceWorkbench::new` 时由 descriptor 一次性解析缓存。`layout` 为 None → 默认 `{left: List, center: Pages}`。渲染入口 `Render::render` 改为：

```text
root shell 声明且可用 → 挂载 root shell(占满,无 native 区域)
否则 h_flex:
  left  → nav_list | nav_tree | shell_region | none
  中列  → v_flex: center(page_area | shell_region) + bottom(status | shell_region | none)
  right → shell_region | native(none) | none
```

terminal 页面保持现状：无视布局直接全屏接管（v1 行为，Docker exec 依赖）。

### 4.3 树渲染（nav_tree.rs）

- 根节点静态渲染；展开时经 dispatcher 执行 `children.operation`（BindingContext: route=Null, selection=Null, paging 默认），结果按 `itemsPath` 投影（复用 `collection_table::items_of` 语义，抽公共函数）。
- 行点击：`children.open` 存在 → `route_binding::build_route(open.route, route, row)` 后 `navigate`；缺省 → `navigate(root.page_id, row)`。
- `TreeChildren.children: Option<Box<TreeChildren>>` 递归声明任意层级 lazy；渲染层按节点键（根 id + 逐层行键）缓存展开状态，折叠时清除后代缓存。每层 operation/open 可用 `parent` 绑定源引用父节点行数据（如 K8s namespace → pods）。
- 加载/失败态内联在节点下方，复用现有 `loading_state`/alert 模式。

### 4.4 状态栏（status.rs）

`render_status_bar` 改为 items 驱动：`value.pointer(path)` 取值 + format 渲染。`format` 语义：`raw` 字符串直出；`number` 数字；`bytes` 复用 `format_bytes`；`percent` 保留两位；`version` 前缀 `v`；`boolean-up` 绿点+文案（true → theme.success + "Running" 类语义由 label 承载，宿主只渲染状态色）；`pair` `x/y`。数据拉取沿用现有 `load_status_bar`（operation 改从 bottom region 读取）。

### 4.5 区域级 Shell（shell_region.rs）

- 复用 `CustomPageHost::mount`；`ShellPageMountRequest.page_context` 扩展：`{regionId, pageId: null, route, selection, capabilities}`。mount 缓存 key 从 page.id 改为 region 标识（`"left"`/`"right"`/`"bottom"`/`"center"`）。
- JS 侧拿到现有 `navop.workbench` host module（`current()` + `dispatch()`，workbench.rs:26 已实现），region 上下文通过 `current()` 的 `regionId` 区分。
- region shell 失败 → fallback 声明只有 `none`：渲染占位空态（icon + "This panel is provided by an extension view that is unavailable"）。
- root shell 失败 → 错误态 + "切回"提示文字（无 native 降级，见 2.4）。

### 4.6 联动（首版范围）

左树/列表选中变化 → 右侧 shell region 需要感知。首版机制：`navigate()` 时刷新 region shell 的 page_context（dispose + remount 代价大，改为：region mount 的 view 常驻，宿主在 navigate 后向 `MountHandle` 挂的 workbench entity 发 `EventEmitter` 事件；JS 侧后续版本通过 workbench module 的订阅 API 获取——**首版 JS API 只保证 `current()` 返回挂载时快照 + remount on demand**，完整 `onNavigationChanged` 订阅列为后续，不在本次范围)。center native 页面不受影响。

## 5. JS 侧（本次不新增 host module API）

`navop.workbench` 现有 `current()` / `dispatch()` 已满足 region shell 与 root shell 的数据访问。区域布局信息经 `current().regionId` / `current().layout`（page_context 附带 ResolvedLayout 的 JSON 摘要）只读暴露。`navigate` / `selection` 双向 API、dirty 守卫协议 → 后续版本。

## 6. 迁移

| 扩展 | v1 | v2 |
|---|---|---|
| docker | navigation(11) + tree(1,未渲染) + statusBar + tabs×8页 | layout.left(tree, roots: containers+静态根) + center.tabGroups(container 组) + bottom.status(items 9 项) |
| elasticsearch | navigation(7) | layout.left(list) 默认即可,声明可不写 layout |
| mqtt | navigation(6) + tree(1) + tabs×4页 | layout.left(tree, roots: topics) + center.tabGroups(topics 组) |
| rocketmq | navigation(7) + tree(2) | layout.left(tree, roots: topics+groups) |

宿主删代码：v1 `navigation`/`tree`/`statusBar` 渲染路径、`ResourceWorkbenchTab` 在 page 上的字段、Docker 专用状态栏 match（lib.rs:1488-1594）。

## 7. 验收

1. 无 `layout` 声明的 ES manifest → 行为与 v1 等价（左列表 + 页面区）。
2. Docker：左树可展开（listContainers）、点容器跳 container-detail；tab 组 8 页共享一份声明；底部状态栏 9 项全由 items 驱动，无 Docker 专用代码。
3. `layout.renderer: shell` 的测试 manifest → 整区 JS 视图，无 native 区域；viewId 无效时错误态。
4. `right: {source: {kind: shell}}` → JS 视图常驻右侧；navigate 后 `current()` 快照语义不变（首版无订阅）。
5. schemaVersion=1 → 注册失败,错误信息指向迁移。
6. 既有单测（resolve_renderer 四例、parser tests）全绿;新增 layout parser/校验单测。

## 8. 明确不做（本次）

- 区域拖拽调宽、split pane、区域显隐持久化。
- JS `navigate()`/`selection()` 双向 API、`onNavigationChanged` 订阅。
- 树右键菜单。
- bottom 的非 status native 模板（tasks/terminal 已是页面模板,不重复做区域版）。
