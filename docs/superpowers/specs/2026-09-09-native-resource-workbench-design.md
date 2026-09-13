# Navop 原生统一资源工作台与 GPUI Shell 覆盖设计

日期：2026-09-09。状态：**建议定稿，尚未实现**。范围：`navop` 与相邻的 `navop-extensions`。

配套声明契约：`2026-09-09-native-resource-workbench-contract.md`。本文中的新增类型、模块和 API 均是设计，不代表现有接口已经可用。

## 1. 决策摘要

**一个宿主会话所有者，一个操作执行入口，两种可组合的渲染方式。**

- 通用 Tree / Table / JSON / Query / Events / Tasks 由 Navop 的 Rust/GPUI 实现一次。
- 扩展提供协议实现、连接表单、操作定义、资源数据及受限的语义页面声明；不是每个扩展复制一套通用 JS 页面。
- 默认使用原生页面；支持 Shell 自定义页面、同一页面的 Shell 增强覆盖，以及显式启用的工作台主体覆盖。
- 无论哪种渲染器，都借用同一个 ResourceSession；Shell 不能关闭连接主资源、释放连接 activation 或绕过宿主权限。
- 不把 ES、Docker、Kafka 等协议分支写进通用 renderer；不要求第三方插件编译 Rust 动态 UI 库。
- 不重建 Resource / Job / Event / Blob IPC；也不把 provider 返回 UI 树、任意表达式或远程布局重新引入协议。
- 第一版不新增 `connections[].workbenchId`：使用工作台声明中的 `connectionIds` 精确绑定已有连接贡献，兼顾旧 manifest 解析器。

## 2. 现状与本次方案纠偏

以下路径相对 `navop/`；扩展路径以 `../navop-extensions/` 开头。行号是本次核查时的定位。

| 已核实的现状 | 证据 | 设计后果 |
| --- | --- | --- |
| 扩展连接入口目前依赖 `shell-plugins`，未启用时直接提示不支持 | `main/src/home/home_strategy.rs:71–158` | 原生工作台不是只添加一个 crate；必须解除入口、连接表单、公共服务的 Shell feature 绑定 |
| 公共服务、headless tab、Shell host 都被同一个 feature 包裹 | `crates/universal-plugins/src/lib.rs:10–33` | feature 只能控制 Shell 实现，不能控制通用资源连接 |
| 已有 managed client 和 generation、job/event/blob 管理 | `crates/extension-plugin-adapter/src/activation.rs:254–337` | 复用现有管理器，新增的是 session 所有权与 UI 适配，不是第二套进程管理 |
| 当前 Shell 打开连接会自行 open resource，再交给 mount session 管理 | `crates/universal-plugins/src/shell_plugin_host.rs:121–205`；`shell_plugin_host/connection.rs:170–195` | 新嵌入接口必须接受借用会话，不能调用旧 `open_connection` |
| Shell 卸载清理包含 resources | `crates/universal-plugins/src/shell_plugin_host/session/cleanup.rs:80–149` | 不可把连接主资源作为普通 owned resource 注册进页面 mount |
| LoadedShellView 能取得 ScriptView Entity，但接口目前是 crate-private | `crates/universal-plugins/src/shell_plugin_host/policy.rs:13–35` | 有可复用的渲染接缝，但没有可直接使用的完整嵌入 API |
| connection 使用 `deny_unknown_fields`，贡献容器则没有 | `crates/extension-runtime/src/extension/manifest/contributes/connection.rs:3–20`；`contributes.rs:15–63` | 保留 `shellViewId` 同时新增 `workbenchId` 仍会让旧宿主拒绝整个 manifest |
| 连接注册已经采用 extension namespace | `crates/extension-runtime/src/registration.rs:366–395` | 工作台也必须用 namespace key，不能全局仅按 resourceType 查找 |
| 已有通用表格、文本/JSON 编辑基础 | `crates/one_ui/src/lib.rs:18–32` | 优先复用 EditTable、EditTableDelegate、LargeTextEditor、ContentState |
| DbTreeView 与 DataGrid 有数据库类型和 SQL 场景依赖 | `crates/db_view/src/db_tree_view.rs:1–30`；`table_data/data_grid.rs` imports | 不直接导入数据库树和结果网格作为资源工作台模型 |

现有 `docs/extension-resource-plugins/README.md` 和 `architecture.md` 指向“后续 UI 统一由 gpui-shell 承载”。本方案**变更这个 UI 默认方向**，但保留其 headless provider 协议边界。实施时应同步这些文档，不能把 manifest 页面模板误写成恢复已移除的 ViewSpec/UiNode/provider UI RPC。

注意：当前 `extension-runtime/Cargo.toml` 已依赖 GPUI、db_view 等。本文说的“无 UI session/model”是新增模块的代码和 API 边界，不宣称现有 adapter/runtime 的整个传递依赖图已经 UI-free。彻底拆分 runtime catalog 属于另一个重构，不应阻塞首个纵向切片。

## 3. 运行架构与 crate 边界

```text
StoredConnection + Installed Manifest
                   │
        ConnectionPresentationResolver
                   │
             ResourceConnectionTab       ← 连接生命周期、状态、关闭守卫
                   │
          Native ResourceWorkbench        ← Header / Navigation / Tasks / Errors
                   │
        ┌──────────┴───────────┐
        │                      │
 Native Template Renderer   Shell Mount    ← 页面覆盖或工作台主体覆盖
        └──────────┬───────────┘
             Operation Dispatcher         ← 参数、权限、确认、取消、结果、审计
                   │
          ResourceSession / Scopes        ← 一个主资源，多页面借用
                   │
      ManagedUniversalPluginClient
                   │
       Resource / Job / Event / Blob
                   │
     ES / Docker / MQ / Nacos Provider
```

建议代码归属：

| 所在模块 | 新增/调整责任 |
| --- | --- |
| `extension-runtime` | Workbench manifest DTO、校验、注册、安装版本与 namespace 绑定；不依赖 resource_view |
| `extension-plugin-adapter` | 无 GPUI API 的 ResourceSession、owner/scope、操作执行和结果读取；复用 ActivationManager 与现有 job/event/blob 管理器 |
| `resource_view`（新增） | WorkbenchModel、ResourceConnectionTab、导航、模板 renderer、页面状态、CustomPageHost 扩展接口 |
| `universal-plugins` | 保留应用级唯一 UniversalPluginService；接入新 session API；仅 Shell 相关模块继续 feature-gated；实现 CustomPageHost |
| `main` | 服务初始化、连接打开组合、注入可选 Shell host、连接状态与现有 TabContainer 对接 |
| `navop-extensions` | provider、manifest、可选 Shell 页面；不依赖 resource_view 编译 |

依赖方向：`resource_view → extension-plugin-adapter / extension-runtime / one-core / one-ui / gpui`；`universal-plugins → resource_view`；`main → 两者`。**禁止 `resource_view → universal-plugins` 与其实现反向形成环。**

UniversalPluginService 目前还持有 catalog、凭证、monitor 和 GPUI 初始化桥。首版不要复制一个新服务：解除通用部分的 feature 绑定，通过 adapter 定义的窄 `ResourceSessionBackend` 接口委托 activation/client/release；由现有应用服务实现该接口。存储和 GPUI 全局不下沉到 adapter。

`resource_view/src/` 首版按 `resource_tab`、`workbench`、`model`、`navigation`、`pages`、`custom_page_host` 分组即可；不要先为每个按钮创建文件，也不要创建 `es_view.rs`、`docker_view.rs`。通用树先由 resource_view 拥有，出现第二个真实消费者后再考虑下沉 one_ui。

## 4. 所有权：session 不是一个公开字段的 GPUI Entity

建议分为以下三个角色，字段保持私有：

- `ResourceSessionOwner`：连接 Tab 的唯一控制端，负责 open/reconnect/close 和 activation lease；显式异步释放。
- `ResourceSessionHandle`：可克隆的受限访问端，读取连接快照、创建 scope、执行已注册操作，不暴露关闭主资源权限。
- `ResourceScope`：页面或任务拥有的子作用域，跟踪自己的请求、job、event、blob、额外子资源；页面卸载只释放本 scope。

`Entity<WorkbenchModel>` 保存导航、选择、输入草稿、加载状态、renderer preference 等 UI 状态，并持有 session handle。I/O 与生命周期在 session/service 中，不要求其成为 GPUI Entity。ManagedUniversalPluginClient 和原始 resource_id 不作为页面可任意修改的 public 字段。

层次为：Application owns service → ConnectionTab owns primary session → PageScope borrows primary resource。Provider 进程仍由 ActivationManager 统一监督；clone client 不等于获取/释放 activation lease。

session 标识包含 extension id、qualified runtime id、activation identity、runtime generation、宿主 session epoch。generation 取自已有运行时，不能在 UI 自己递增冒充它；epoch 在连接重开时改变。异步 UI 结果再带 page/mount id 与 request revision，避免旧页面、旧筛选、旧连接的迟到结果覆盖新状态。

状态至少区分 Opening / Ready / Reconnecting / Disconnected / Failed / Closing / Closed；对接现有 TabContent 时映射为其已有三种连接状态。重连是 ResourceSessionController 的操作，不假设现有 TabContent 有 reconnect trait 方法。

连接 Tab 同时保留现有 `ActiveConnectionLease` 的产品状态职责，不能把它和 provider activation lease 混为一个东西。普通 native/Shell renderer 切换不释放任何连接级 lease。legacy 整页 Shell 在迁移前维持原有独立 owner 路径，不把旧 ownership 规则混用到新 borrowed mount。

## 5. 完整打开、重连与关闭通路

1. 用保存的 extension id + connection contribution id 精确找到安装贡献；缺失或禁用时展示可恢复错误，不猜同名其他插件。
2. 在同一个 catalog/install revision 快照中解析 runtime、workbench binding、shell view、资产路径和 feature/API 兼容性。先校验声明，后执行 provider I/O。
3. 用现有 TabContainer 的稳定连接 tab key 去重；普通点击激活已有 Tab，不再打开第二个 resource。
4. 创建 Pending Tab 和打开取消令牌；通过唯一 service 获取 activation lease，按现有凭证机制构造资源 config，调用 resource open。
5. 如果用户在等待时关闭 Tab，晚到的 open 成功结果立即交给清理路径；不能挂上已关闭页面，也不能遗留 activation。
6. 保存 ResourceOpenResult，完成 capability 协商，建立主 session owner，然后挂载工作台与默认页面。
7. 导航、选择、运行操作走同一个 dispatcher；Shell 页面通过借用 scope 使用同一个 session，不再执行主连接 open。
8. 显式重连先隔离旧 epoch、暂停输入和流，释放旧作用域及资源，再重新 open；新能力集重新校验。输入草稿可保留，旧 provider 句柄不可复用。
9. 关闭先处理所有 dirty editor 的统一守卫；取消关闭则完全保留会话。确认关闭后，禁止新调用、失效 mount/epoch、有界取消并清理子请求/job/event/blob/子资源、关闭主资源、释放 activation，最后完成 Tab 关闭。

关闭幂等；清理失败记录诊断，不无限阻塞 UI。`Drop` 只是 best-effort 兜底，不是唯一清理机制。旧 generation 的 id 不能拿去关闭新进程里的同名对象。

任务默认 page-owned；显式“后台继续”的任务可以转交 session/task scope，不能靠忘记 drop 保活。首版关闭连接会取消这些任务；跨连接关闭继续运行需要应用任务系统显式接管 session owner，留到后续实现。

只读加载可做有限退避重试；写入、发布消息、删除、终端输入不得因重连或换 renderer 自动重放。超时/取消不等于远端副作用没有发生，必须显示“结果未知”并提供查询状态，而不是默认再发一次。

## 6. 声明式页面：限制在语义模板，不做第二套前端语言

两个维度必须独立：

- `template`：overview / collection / detail / query / json / events / tasks；将来按多个真实插件需求扩展 editor、diff、metrics、topology、terminal。
- `renderer`：native / shell。同一业务页面可保留 native template 并选择 Shell 增强实现；只有 Shell 的页面没有 native fallback。

数据请求引用命名 operation；operation 定义 method、invoke/job 模式、参数类型、副作用类别、要求的 capabilities、结果投影。tree/page/action 共用 operation，避免方法字符串和安全策略复制。

参数采用有类型的绑定对象：literal / input / route / selection / paging。结果采用受限的字段路径选择、列声明和稳定行键；不支持 JS eval、自由字符串插值、递归 JSONPath 或任意表达式程序。缺失字段、类型错误、重复 row key 必须产生明确数据契约错误。

集合必须声明 items、row key、列类型与分页模式。服务器分页与当前页本地过滤不可混淆；游标作为 provider opaque value 原样传递。树节点 key 应包含 connection/session、资源种类与稳定资源键，不使用显示标签或数组位置。

动态资源由 provider 返回数据；布局结构来自经过安装校验的 manifest。复杂领域投影由 provider/扩展适配层归一化，不把 `if resource_type == "elasticsearch"` 写进 renderer。第一版 JSON 页面可以保留原始数据，专业体验逐步增强。

设计不强制所有中间件都变成 CRUD 表格。查询语言、配置编辑、消息消费、日志/终端是不同交互语义，不能因为都能传 JSON 就假装已经等价。

## 7. 一个 operation dispatcher，两种 renderer

Native toolbar、右键菜单、快捷键、Shell 按钮最终提交相同的 OperationId + typed inputs；统一执行：绑定校验 → capability 检查 → host policy → 危险操作确认 → 调用 → 结果解码 → 状态更新/缓存失效 → 审计。

有效能力不是仅看 manifest：需要满足声明要求、ResourceOpenResult 的实际 capabilities、宿主授权与当前会话状态。manifest 的 `effect: read` 是扩展声明，不是对不可信二进制行为的证明。未知副作用按保守策略处理；provider 仍负责服务端认证与权限。

ResultRef 保留 Inline / Blob / EventStream 区别。Blob 按大小与媒体类型有界读取，不默认完整载入大 JSON；events 有界队列、批量刷新、显示 dropped_count；job 的取消、终态、结果读取、close 都由 session scope 管理。统一任务面板是这些任务的视图，不新增第二个 JobManager。

错误至少区分 unsupported、permission denied、invalid params、provider unavailable、stale session、timeout、result contract violation、renderer failed。错误信息包含 operation/request 标识，但不能泄露密码、连接 secret refs 内容或敏感查询正文。

未来 Public MCP/自动化可以适配同一 operation gateway，但需要单独的工具注册、调用者权限、审批与审计策略；不是把任意 manifest method 自动暴露为工具。查询语句或操作内容敏感时，诊断仅记录脱敏摘要与关联 id。

## 8. Shell 覆盖的三个等级

| 等级 | 覆盖区域 | 不可覆盖部分 | 建议 |
| --- | --- | --- | --- |
| 页面扩展/覆盖 | 一个 page 的内容区；可新增独有页面，也可增强已有原生页面 | 原生导航、连接状态、会话归属、任务入口、安全控制 | 默认扩展方式 |
| 工作台主体覆盖 | sidebar + page content；适合确实需要特殊工作空间的插件 | 外层连接 Tab/Header、关闭守卫、权限、全局任务与“切回原生”入口 | 显式声明 + 用户允许；非默认 |
| 旧版整页 Shell | 当前 shellViewId 路由的原有独立 Tab | 仍遵守旧宿主权限与生命周期 | 仅兼容路径，不是新插件模板 |

默认 native；只有明确声明 shell renderer 且用户允许时才覆盖。用户可固定“始终使用原生”，有 native template 的页面必须可切回。工作台主体覆盖需要已有原生工作台作为退路；纯 Shell 插件继续走显式 legacy/custom 路径，不伪造可用的 native 工作台。

新版挂载接口建议称 `CustomPageHost::mount`，由 resource_view 定义接口、universal-plugins 实现。输入是经过解析的 view contribution、borrowed PageScope、PageContext；输出是可嵌入 AnyView/ScriptView Entity 的 mount handle 和生命周期接口。该接口**不调用**旧 `ShellPluginHost::open_connection`。

PageContext 包含 page id、route、selection、只读 session snapshot、有效操作集合。新增受限 `navop.workbench` host module 提供 dispatch、navigate、selection、dirty/save 协议；相关 host-module enum、权限校验和版本声明都需实际新增，不能在现有 manifest 上凭空使用。

兼容 `navop.context` 的 resource handle 形状时，背后也必须是 borrowed-primary handle；对它调用 close 返回明确错误。新 workbench mount 默认经 dispatcher 使用声明的操作，不默认暴露任意 resource.invoke、runtime 控制、secrets 或任意新连接。特殊子资源须显式声明允许的操作与 scope，仍受宿主策略约束。

新 mount lifecycle：prepare → mount → activate/deactivate → dispose。dispose 只收回本 mount 的订阅和子资源；不 dispose session owner。Shell 不能无限阻止关闭；保存守卫有超时，失败后由用户决定保留、丢弃或取消关闭。

首版同一 page 同时只保留一个活跃 renderer。切换前检查草稿；可移交的 typed input/route/selection 由宿主管理，不可序列化的 Shell 编辑状态先保存或确认丢弃。不能以 fallback 为名静默丢稿。

## 9. 路由与 fallback 不是一个简单的 if/else

| 条件 | 行为 |
| --- | --- |
| 有合法工作台绑定，native 可用 | 开原生工作台；页面按显式 renderer preference 决定是否使用 Shell |
| 页面选择 Shell，但此构建没有 shell-plugins | 有 native template 则回到原生并说明；Shell-only 页面显示不可用，其他页面仍可用 |
| Shell 装载失败 | 只做展示降级，不重新 open 主资源、不自动重发 operation；dirty 状态先走守卫 |
| 没有工作台绑定，有旧 shellViewId | 使用原有 Shell 路由；无 Shell 构建明确提示缺少功能 |
| 两种 UI 声明都没有 | 可保留 headless/连接诊断页，不根据 resourceType 猜测某个业务 UI |
| 绑定歧义、跨扩展引用、非法 descriptor、授权失败 | 明确失败；不可通过 legacy fallback 隐藏配置错误或绕过安全检查 |

旧 explorer.js 是完整页面，不会因放进一个容器就自动成为合格嵌入页面。页面级复用需要拆出对应内容和新的 lifecycle/borrowed-session 协议。

## 10. 注册、身份与向后兼容

新增 `RegisteredResourceWorkbenchContribution`，注册 key 为 `(extension_id, workbench_id)`。新增按 `(extension_id, connection_contribution_id)` 查询绑定的方法；resourceType 只用于类型一致性校验与搜索过滤，不用于全局挑选 renderer。

第一版 canonical binding 是 `resourceWorkbenches[].connectionIds`，全部指向同一扩展的连接贡献；每条连接最多绑定一个工作台，并校验 runtimeId/resourceType 一致。注册时可将 resolved workbench key 填入宿主内部连接记录，但不需要更改旧连接对象或存储字段。

旧宿主会忽略新的 resourceWorkbenches 顶层贡献，继续使用未改变的 shellViewId。该兼容结论仅适用于其他旧字段和 API 版本也保持兼容的包；不能声称整个新版插件自然都兼容。

保留完整 legacy explorer 的过渡 ES 包可以走这个 additive 路径。去掉 legacy 页面或新增只在新宿主存在的 Shell module 时，必须提高真实 `engines.onetcli` 最低版本，或分别发布 legacy/native 包；最低版本由实际发布确定，不在设计中虚构。当前引擎兼容检查在反序列化之后，所以它救不了旧 connection 对象新增未知字段造成的解析失败。

Workbench descriptor 自有 `schemaVersion: 1`；未知版本/模板严格诊断，不能忽略后猜测布局。建议从 Rust DTO 生成 JSON Schema，供 SDK/CI 使用，避免文档和运行时各维护一份冲突定义。

安装/升级需要对 connection/workbench/shell/runtime 一起做交叉引用验证并原子注册。活动 session 固定 descriptor 与安装 revision；升级按统一 retire 流程关闭旧 session，或保持旧版本资产直到引用释放，不能让旧句柄配上新页面定义。

扩展 SDK/打包 CI 至少验证 manifest 与示例结果、方法/能力引用、各目标平台 provider 产物、Shell asset 引用与 API 要求；提供 fake resource backend 和 descriptor 预览测试。没有这些配套，仅有 schema 很难成为可维护的第三方生态。

## 11. ES 首个纵向切片

当前扩展是 `com.navop.elasticsearch`，runtime `main`，connection `elasticsearch9`，resourceType `elasticsearch`，legacy shell `explorer`。协议核查见 `../navop-extensions/extensions/composite/elasticsearch/src/client.rs:66–94,129–153,249–278` 与 `src/state/job.rs`。

| 页面 | 现有 provider 能力 | 首版处理 |
| --- | --- | --- |
| Overview | cluster/info、cluster/health | 通用字段概览 + 原始 JSON；不先硬编码 ES 指标卡 |
| Indices | index/list → `{indices:[{name,health,docs,size_bytes}]}` | items `/indices`、row key `/name`；docs/size_bytes 先按实际值展示，不假设已经是数值 |
| Index detail / Mapping | index/get、index/mapping，参数均为 `{name}` | route.name 绑定；先通用 JSON/detail，不臆造已经归一化的字段树 |
| Search | search/async，参数支持 indices、body 或 query | 原生 Query + job；当前异步是 provider 本地任务，不是 ES 服务端 `_async_search` |
| Search result | `{raw: ...}`，以 Inline 或 Blob 返回 | 首版 JSON；hits 表格需投影 `/raw/hits/hits` 与复合键，不能直接假设顶层有 rows |
| Tasks / Events | 已有 job / event 协议与 ES 事件能力 | Tasks 展示宿主 scope 内任务；Events 另做具体事件开流/绑定契约测试后启用 |

Provider 可不改即可完成连接、索引列表、详情 JSON 与基础搜索的首个闭环，前提是宿主实现了对应 descriptor 消费端。专业 Mapping 树、稳定搜索行标识、字段补全、错误定位等仍需补数据契约或 provider 适配，不应承诺“只加 manifest 就有完整 Kibana 式体验”。

ES DSL 编辑器、Nacos 配置编辑器、MQ 消息发布表单不天然必须是 Shell。先用通用 Query/Editor/Form；只有模板覆盖不了的专业交互才用 Shell。Docker terminal、Kafka topology 等也应在出现可复用需求时演进宿主通用能力，而非永远固化为插件重复代码。

## 12. “万物可连”的边界

统一的是连接入口、会话与权限、导航/操作模型、任务/流/结果、基础展示，不是所有系统的领域协议。

- Docker 的本地 socket/远程连接、交互终端等，需要分别验证 endpoint 授权与双向输入/resize/退出语义；普通 EventStream 不能自动代表完整终端。
- MQ 管理、消息生产、消费确认/offset 是不同操作契约；事件面板的 dropped_count 不是消息队列 ack，不可承诺可靠消费。
- Nacos 配置发布需要权限、版本/冲突检查和危险写操作确认；HTTP JSON 能通不代表编辑闭环完整。
- 后续 Kubernetes 的 watch/exec/配置冲突需要专门域契约；不靠增加几个 manifest page 就宣称支持。

目前 endpoint 授权有具体 config 字段与协议识别限制（`extension-plugin-adapter/src/provider_permissions.rs`）。因此“不新增网络机制”应理解为“不重建 provider IPC”，不等于 Unix socket、named pipe、SSH tunnel 等已经全部支持。Native sidecar 权限声明也不等于 OS 沙箱。

## 13. 原生 UI 与性能契约

复用 one_ui::EditTable 的 delegate 模式和 LargeTextEditor 的文本/JSON 基础；保留 db_view 现有 SQL 专业工作流，不以统一资源工作台为由改写数据库编辑器。新增树和模板不引入数据库节点类型。

UI state 用稳定 Entity 保存；render 不做 I/O、不创建连接、不内联调用另一个实体的 render。Tokio-bound RPC、timer、流在应用 Tokio runtime 执行，结果经桥返回 GPUI 前台；使用 WeakEntity 和 revision 检查，不能跨线程操作 Window/Context。

导航和内容各有明确滚动所有者，AnyView/Shell mount 外层有 min_w_0/min_h_0 和裁剪边界；共享窗口 Root、主题 token、焦点/快捷键体系，不由每个嵌入页重新建 Root。隐藏页面停止无意义刷新，树/表格虚拟化，输入/筛选保留稳定状态。

列表条数、单次结果字节数、blob 预览上限、event buffer、并发数、请求超时、后台 job 数量、缓存/LRU 大小都要有宿主上限；插件可以申请更小值，不能无限增大。具体默认值通过首轮基准确定，发布前必须固化并测试。

## 14. 实施顺序与验收

| 阶段 | 范围 | 必须交付的验收 |
| --- | --- | --- |
| P0 生命周期与 feature 解耦 | 唯一服务非 Shell-gated；owner/borrowed scope；连接 form/open 可在普通构建使用 | 无 Shell 构建可打开 fake provider；重复打开一个主资源；晚到 open 能清理；重启旧句柄失效 |
| P1 原生最小闭环 | descriptor parser/catalog；ResourceTab；Tree/Collection/JSON；真实连接入口一起接通 | ES Overview/Indices/Detail 在原生显示；连接树操作到 provider 再到页面全链路可用 |
| P2 查询/任务/事件 | dispatcher；Query/job/blob；Events；只读 retry 与取消策略 | 小/大结果、取消、丢事件、分页、乱序返回、大列表及线程边界测试 |
| P3 Shell 页面嵌入与覆盖 | borrowed mount API；workbench module；dirty guard；renderer preference/fallback | native↔Shell 不二次 open；卸载 Shell 主连接仍可用；Shell 无权 close 主资源；失败不重发写操作 |
| P4 第二种领域验证 | Nacos 只读列表/配置读取或 Docker 只读列表，按 endpoint 条件选择 | 不改 resource_view 业务分支即可新增插件；专业差异通过 descriptor/provider/Shell 表达 |
| P5 扩展增强 | 写操作表单/冲突、工作台主体覆盖、更多 transport/专业组件 | 每种新语义有独立契约、风险策略和跨平台验证后再开放 |

不要先做完全部通用组件，最后才改连接入口。P1 就必须把打开流程接通；否则容易得到脱离实际服务所有权的 fake UI。

测试按层分开：serde/绑定/引用/路径单测；fake backend 的 session 与乱序/关闭/重启测试；renderer 纯状态和 GPUI 焦点/布局测试；真实 provider 的协议 fixture 与端到端冒烟；默认构建和 shell-plugins 构建都纳入 CI，release feature 组合也要覆盖。确定性 GPUI 测试不直接等待不可控的真实 Tokio worker。

首个里程碑不是“画出了 ES 页面”，而是：**无 Shell 构建能打开 ES；原生页共享一个会话；关闭/重连无泄漏；有 Shell 构建能覆盖一个页面且不接管主连接；第二个 provider 不需要给 Navop 添加业务分支。**
