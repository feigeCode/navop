# 全局 Agent 规则

本文件用于约束自动化代理在本机工作区中的默认工作方式，强调最短路径、风险分级、TDD 思想与可验证交付。

## 指令优先级

- 指令优先级从高到低：
  1. 当前会话中用户的明确要求
  2. 仓库自身的规则、文档与约定
  3. 本 `AGENTS.md` 的任务分流与流程规则
- 本文件是自包含的工程工作流，不依赖特定外部流程体系。
- 本文件明确标记为“个人硬门禁”的规则始终需要满足。
- 对于仅涉及审查、分析、解释或方案讨论而不修改仓库文件的任务，可不进入完整实现流程，但仍应保持推理清晰、结论可追溯。
- 如果用户明确要求 `continue nonstop`，则默认持续推进，直到满足验收标准或出现真实阻塞。

## 默认工作流原则

### 最短路径原则

- 默认采用“**满足质量要求的最短路径**”。
- 能直接完成并验证的，不升级为更重流程。
- 能使用轻量版 planning 完成的小任务，不升级为重文档流程。
- 流程的目标是降低返工与风险，而不是增加形式成本。

### 自包含工作流原则

- 只读任务可直接分析并给出有依据的结论。
- 实现类任务应先明确目标、边界、约束、验收标准和验证方式，再按风险决定计划与测试深度。
- Debug 先收集证据、定位根因，再修改；Review 独立检查规格、正确性、回归与可维护性；完成前必须运行与风险相称的验证。
- TDD 是新增或改变可观察行为、处理高回归风险逻辑时的工程方法，而不是外部工具或流程依赖。

## 任务分流模型

### 只读任务

以下任务可直接处理，不强制进入实现流程：

- 分析
- 解释
- 架构说明
- 代码阅读
- 纯信息型问答
- 不修改文件的只读审查

若任务属于真实问题排查，但尚未进入修改，应先收集日志、错误、复现条件和调用链证据，再形成可验证的根因假设。

### 实现类任务

以下任务原则上必须先完成需求澄清与计划拆分：

- 新功能
- bug 修复
- 行为变更
- 重构
- 页面 / 组件 / API / 脚本实现
- 数据处理逻辑改动

默认流程：

1. 澄清目标、边界、约束、验收标准与验证方式
2. 按任务规模形成轻量任务列表或详细实现计划
3. 再进入具体实现
4. 根据行为风险选择直接验证、回归测试或 TDD

#### 轻量版 planning

- 对小任务，允许采用**轻量版 planning**
- 可在当前对话内完成，不强制产出长文档
- 最小集合至少应明确：
  - 目标
  - 边界
  - 风险
  - 验证方式
- 只有当任务规模、风险或不确定性明显上升时，才升级为更重的 planning 流程

### Debug 类任务

适用：

- traceback
- error
- exception
- 数据异常
- 运行时异常
- 协议异常
- UI 显示与数据不一致
- 根因不明的问题

说明：

- 对真实 bug / 异常排查，不直接猜测式修补
- 先稳定复现或收集足够证据，再建立假设并逐项验证
- 确认根因后再决定修复方案，并补最贴近故障表面的回归保护

### Review 类任务

适用：

- review
- code review
- reviewer output
- spec compliance
- 合并前审查
- 阶段性交付审查

审查要求：

- Review 应检查规格符合性、正确性、边界条件、错误路径、回归风险、测试充分性和可维护性。
- 处理 review 反馈时，应先验证反馈是否成立，再修改；不要未经验证机械接受，也不要因表达方式而忽略有效问题。

### 完成前验证

在声称以下状态前，必须运行与改动直接相关的验证并检查实际输出：

- 完成
- 已修复
- 可提交
- 可合并
- 测试通过
- 验证通过

不得用过去的结果、推测或“理论上应该通过”代替当前验证证据。

### 前端任务

适用：

- UI
- UX
- 页面布局
- 组件视觉
- 图表呈现
- 交互设计

处理方式：

- 明确布局、视觉层级、状态反馈、键盘操作、可访问性与不同窗口尺寸下的行为。
- 修改后优先做结构测试、相关 UI 测试和必要的手工视觉验证。

### 流程升级规则

若在执行过程中发现以下任一情况，应升级到更重流程：

- 影响文件、模块或系统边界超出初始判断
- 出现公共 API、schema、持久化、并发或共享逻辑风险
- 用户真实需求仍不清晰
- 当前验证手段不足以覆盖风险
- 任务已从局部修复演变为中大型实现或重构

### 流程降级规则

若任务满足以下条件，可降级到更轻流程：

- 改动局部且边界清晰
- 不涉及共享核心逻辑
- 验证手段简单直接
- 补长计划或补测试的成本显著高于风险收益
- 问题已收敛为单点修复或局部调整

## 推进与验证

### Step by Step Reasoning Workflow

- 如果需求模糊，应先澄清目标、约束、验收标准与边界条件。
- 为跟踪进度，请维护一个可见的任务列表。
  - 在其中列出先前任务的状态和待办事项，以及项目所需的计划行动（对于简单问答可以跳过）。
  - 对多步骤任务，任一时刻仅保留一个 `in_progress` 步骤。
  - 开始新步骤前及时标记已完成步骤，并避免重复输出冗长计划。
- 回答时优先给出最相关结论，再补背景、依据与权衡。
- 在任务推进过程中，遇到新信息时应主动修正先前判断中的错误或不一致。
- 多步任务优先使用 `update_plan` 或同等方式维护高层进度。

### Environment

- 环境初始化优先遵循仓库文档与项目级 AGENTS。
- 若无明确要求，则按当前任务所需执行最小准备，不做额外环境工程。
- macOS 上 `reqwest` 默认系统代理探测可能在测试进程里触发
  `system-configuration` 的 NULL object panic；应用内需要“无应用代理”的
  HTTP client 时，优先使用 `ReqwestClient::user_agent("onetcli")` 这类显式
  direct client 构造路径，并用相关 `setting_tab`/CLI 测试验证。

### Command Verification Rules

- 不得虚构已运行的命令、退出码或验证结果。
- 如果一个关键验证命令无法执行，必须明确说明原因。
- 在缺少验证证据时，不得声称“通过”“完成”“可提交”“可合并”。
- 如果用户或仓库要求特定验证命令，应优先执行该命令。
- 若关键验证被阻塞，应如实报告当前状态，并根据阻塞程度决定是否继续实现或先与用户确认。

### Change Delivery Gate

在声明完成、准备 `commit`、准备 `push`、准备发起 PR 之前，应满足以下要求：

1. 已完成与本次改动直接相关的验证，并如实报告结果
2. 已按任务类型完成对应质量门禁：
  - 需要 review 的已 review
  - 需要 completion verification 的已验证
  - 需要测试保护的已按测试策略执行
3. 若仓库要求更重验证，例如构建、集成测试、冒烟测试或特定脚本，应优先遵循仓库规则
4. 若关键验证无法执行，必须明确说明原因，并降低完成度表述

### 测试策略判定（TDD 不是默认全量强制）

- TDD 是一种实现思想：先用测试定义可观察行为，再以最小实现使测试通过，随后在测试保护下重构。
- 是否采用严格 TDD，不按任务大小机械决定，而按“行为影响、共享范围、回归风险、测试价值”显式判定。
- 如果采用 TDD，必须先观察到测试因预期原因失败；先写完实现再补一个立即通过的测试，不算 TDD。

#### A. 直接修改 + 定向验证

适用于：

- 文案、样式、布局微调
- 显然局部、低风险的小修复
- 不涉及公共 API、数据库 schema、共享核心逻辑、复杂状态机或并发语义
- 改动本身明显小于补测试成本

处理方式：

- 可直接修改
- 修改后必须执行与本次改动直接相关的定向验证
- 若已有相关测试，则优先运行相关测试；没有则不强制新增测试

#### B. 修复后补回归测试

适用于：

- 中小 bug 修复
- 有局部行为变化，但范围有限
- 容易补一个贴近问题表面的回归测试

处理方式：

- 可先修复再补测试
- 不强制严格 TDD
- 但应尽量补最贴近问题表面的回归测试

#### C. 必须采用 TDD

适用于：

- 新功能开发
- 明确行为变更
- 公共 API / contract 变更
- 跨模块共享逻辑修改
- 数据库 / 持久化 / 并发 / 状态机相关改动
- 高风险或高回归风险任务

处理方式：

- **Red**：先写一个最小、可读、能表达目标行为的测试，并运行确认它因缺少该行为而失败
- **Green**：只写让测试通过所需的最小实现，避免顺手扩张范围
- **Refactor**：在测试保持通过的前提下整理命名、结构与重复逻辑
- **Verify**：重新运行定向测试，并按影响范围补充相关 crate、集成或端到端验证
- 若受外部系统、硬件或现有架构限制，无法合理先写自动化测试，应说明原因，并先建立最接近行为边界的可执行 contract、fake、回归脚本或明确的手工验证步骤

### 质量门禁分层

#### Level 0：定向验证

- 适用于局部、低风险、小改动
- 直接修改后执行定向验证

#### Level 1：回归测试

- 适用于中小修复或局部行为变化
- 修复后补最贴近问题表面的回归测试

#### Level 2：TDD

- 适用于新功能、明确行为变更、共享逻辑或高风险改动
- 按 Red → Green → Refactor → Verify 循环推进

#### Level 3：Code Review

- 适用于阶段收尾、中高风险改动、合并前审查
- 对照需求、代码差异和验证证据进行独立审查

#### Level 4：Completion Verification

- 适用于所有准备声称完成、已修复、可提交、可合并的任务
- 运行最终验证命令，阅读实际结果，并如实报告通过项、未执行项与阻塞项

## 工程实践

### 快速上手

1. 阅读仓库上下文
  - 查看相关文件、文档、最近提交
  - 优先理解当前任务涉及的模块边界
2. 如用户提供 `plan2go=<path>`：
  - 将该文件视为当前执行来源
  - 执行过程中保持计划状态与进度同步
3. 若需要理解代码架构、调用链、数据流、入口与依赖关系：
  - 优先使用 `ace-tool` 的 `mcp__ace-tool__search_context`
  - `rg` / `grep` 只用于已知字符串的精确定位
  - 若用户明确要求“找出所有出现位置”，可以先用 `ace-tool` 缩小范围，再用 `rg` 做枚举；但架构结论必须以 `ace-tool` 结果为准

### 文档维护

- 每当计划、目标、约束 / 假设、关键决策、经验教训、步骤或进度状态发生变化时，应同步更新相关计划文档。
- 复杂任务在开始实现前，应先评估工作量并拆分为范围受限、因果有序的子任务。
- 对项目中在开发、review、debug、验证过程中反复证明有价值的经验，应及时沉淀到**项目级 `AGENTS.md`** 或等效项目规则文档中，而不是只停留在当前对话。
- 经验沉淀的对象包括但不限于：
  - 常见根因与排查顺序
  - 特定模块的实现 / review 注意点
  - 特定验证入口、命令、超时或环境约束
  - 易误判、易回归、易重复犯错的问题
- 原则是让项目规则随着真实问题不断演进：**一次有效经验，尽量避免团队或后续 agent 再次付出同样试错成本。**
- 若项目已有经验模板，应尽量按统一模板沉淀，避免写成只在当前语境下才能理解的零散描述。
- 一个推荐的最小经验模板包括：
  - **标题**：一句话概括问题 / 经验
  - **触发信号**：什么现象说明又遇到了同类问题
  - **根因 / 约束**：为什么会发生
  - **正确做法**：以后应优先怎么处理
  - **验证方式**：如何确认这次处理是对的
  - **适用范围**：影响哪些模块、页面、链路或命令

#### 已沉淀经验

- **标题**：GPUI UI 测试不要直接依赖真实 Tokio worker 的完成时序
- **触发信号**：`#[gpui::test]` 覆盖 UI 加载时，代码路径内部调用 `one_core::gpui_tokio::Tokio::spawn`，测试出现非确定性 background thread / scheduler 活动，或需要等待真实多线程 Tokio worker 才能断言 UI 状态。
- **根因 / 约束**：`Tokio::spawn` 使用全局 Tokio runtime，再通过 GPUI `background_spawn` 回到测试调度器；这会让本应确定性的 UI 测试混入真实多线程调度。
- **正确做法**：优先把异步任务完成后的 UI 状态变更提取为纯状态 contract，并用普通单元测试覆盖成功、失败、防重复加载、手动刷新等行为；网络解析和下载使用 fake HTTP client 覆盖。对纯 HTTP/IO 的 UI 加载，不要在 GPUI view 层额外包一层 `Tokio::spawn`，优先使用 `cx.background_spawn`，这样 `gpui` 的 `test-support` 能用 `TestAppContext`/`condition` 稳定驱动真实 view 测试。
- **验证方式**：运行对应状态 contract 测试、fake HTTP 网络测试、真实 GPUI view 测试，以及相关 crate 的 `cargo check` / `cargo clippy -D warnings` / `cargo test`。
- **适用范围**：`main/src/settings/*`、扩展市场加载、更新检查、数据库驱动安装等 GPUI UI 层异步加载路径。

- **标题**：GPUI `background_spawn` 不得直接轮询依赖 Tokio runtime 的数据库 Future
- **触发信号**：macOS 上执行 SQL 转储、表导入或表导出时，在 `tokio::time::timeout`、数据库连接初始化或 Tokio socket/timer 路径出现“没有 reactor/runtime”类 panic，随后因 panic 穿过 GPUI 的 `extern "C"` 回调边界而触发 `SIGABRT`。
- **根因 / 约束**：GPUI `background_spawn` 使用 GPUI background executor，不会自动进入应用的 Tokio runtime；数据库 Future 即使表面只是 `async`，其连接驱动通常依赖 Tokio timer、reactor 或 socket。上述“纯 HTTP/IO 优先使用 `cx.background_spawn`”的经验不适用于这类 Tokio-bound Future。
- **正确做法**：由拥有数据库操作的状态层统一通过 `one_core::gpui_tokio::Tokio::spawn_result` 创建任务，并向 View 返回 GPUI `Task`；运行时绑定的核心方法保持私有，并在入口使用 `tokio::runtime::Handle::try_current()` 做防御性校验。View 只负责进度 channel、文件写入和 UI 更新，不自行选择数据库 Future 的 executor。
- **验证方式**：覆盖 Tokio runtime 内外的 contract 测试；用结构回归测试保证受影响 View 不出现 `background_spawn` 或危险的 direct/sync API；运行相关 `db`、`db_view` 测试和 `main` 编译检查，并确认崩溃栈不再从 GPUI background executor 进入数据库连接初始化。
- **适用范围**：`crates/db/src/manager.rs`、`crates/db_view/src/import_export/*`，以及任何会调用 Tokio timer、socket、数据库驱动或 Tokio channel 的 GPUI 后台任务。

- **标题**：GPUI foreground Future 创建 Tokio timer 前必须进入应用 Tokio runtime
- **触发信号**：从 `cx.spawn` / `AsyncApp` 启动 ACP、外部进程或其他异步流程时，在 `tokio::time::timeout` / `sleep` 创建处直接出现“there is no reactor running” panic，即使后续实际工作已经通过 Tokio handle spawn。
- **根因 / 约束**：GPUI foreground executor 不是 Tokio runtime；只把子任务 spawn 到 Tokio 不会让包裹该子任务的 GPUI Future 自动拥有 Tokio reactor。Tokio timer 在创建时就要求当前线程已进入 runtime context。
- **正确做法**：优先把 timeout 放入 Tokio handle spawn 的 Future；若必须在 GPUI foreground Future 中等待 Tokio channel，则先用应用持有的 `tokio::runtime::Handle::enter()` 覆盖 timer 的创建与轮询范围。协议集成测试使用纯 Tokio 入口，不用 deterministic GPUI test scheduler 等待真实 Tokio worker。
- **验证方式**：从 GPUI `AsyncApp` 入口运行连接测试，确认不再出现 reactor panic；再用真实 stdio fake agent 覆盖连接 timeout、prompt timeout/cancel、进程退出和连接复用。
- **适用范围**：`crates/ai_chat_view/src/acp/connection/*`，以及任何从 GPUI foreground executor 调用 Tokio timer、socket、process 或 channel 的路径。

- **标题**：GPUI `overflow_y_scrollbar()` 不要直接承担父级 flex 裁剪职责
- **触发信号**：窗口或面板里已经调用 `.overflow_y_scrollbar()`，但列表/卡片区域仍无法上下滚动，尤其是该区域同时需要 `.flex_1()`、`.min_h_0()` 或 `.min_w_0()` 参与父级布局。
- **根因 / 约束**：`gpui_component::scroll::ScrollableElement::overflow_y_scrollbar()` 会生成额外的 `Scrollable` 外层 wrapper；该 wrapper 渲染时主要继承原元素的 `size`，不能假设原元素上的 flex/min 尺寸约束会作为父级布局约束稳定作用到外层滚动盒。
- **正确做法**：用普通外层容器承担父级布局与裁剪，例如 `.flex_1().h_full().min_h_0().min_w_0().overflow_hidden()`；把真正可滚动内容放到内层 `.size_full().overflow_y_scrollbar()` 中。若父级使用 `h_flex()`，注意它默认 `items_center()`，外层滚动边界通常必须显式 `.h_full()` 或其他明确高度，否则内层 `size_full()` 可能塌陷成白屏。参考 `main/src/new_connection/connection_window.rs::render_card_area`。
- **验证方式**：补结构性回归测试，断言外层有 flex/h_full/min/overflow_hidden 边界、内层有 size_full/overflow_y_scrollbar；运行相关 UI 模块的定向 `cargo test`，必要时手工打开窗口验证滚轮。
- **适用范围**：GPUI popup、dialog、tab 面板中需要滚动的列表、卡片网格、表单内容区域。

- **标题**：GPUI `TabContainer` 必须在 active view 的直接边界截断 intrinsic size
- **触发信号**：打开 RDP、图片、画布等 tab 后，tab 栏窗口控件、左右侧栏或中心区域被内容“挤压”、自动靠拢；只给 `TabContainer` 根节点或 `tab-content` 增加 `.min_w_0()` 后问题仍会复现。
- **根因 / 约束**：flex shrink 约束必须覆盖从 active view 到窗口 chrome 的每一层直接布局边界。外层已有 `.min_w_0()` / `.min_h_0()` 并不能替代中间 `AnyView` wrapper、sidebar center 和图片根节点自身的约束；任一层保留自动最小尺寸时，RDP frame 的 intrinsic size 都可能继续向上传播。
- **正确做法**：所有 active tab 无论是否启用 sidebar，都先放入统一的 `.size_full().min_w_0().min_h_0().overflow_hidden()` wrapper；sidebar center 同样显式裁剪。图片/远程桌面类 view 的 root、content 和 frame 也应设置零最小尺寸，由父容器 bounds 决定最终大小，不允许 frame 反向参与 `TabContainer` 宽高计算。
- **验证方式**：先用 contract 测试确认 sidebar 与非 sidebar 两条路径都经过同一个 active-view boundary，并覆盖 sidebar center、RDP root/content/frame 的 shrink 约束；再运行 `one-core`、对应 view crate 的测试和 `main` 编译检查，手工切换普通 tab/RDP tab、缩放主窗口及展开侧栏，确认窗口 chrome 和侧栏位置不跳变。
- **适用范围**：`crates/core/src/tab_container.rs`、`crates/remote_desktop_view/src/view/render.rs`，以及任何在 tab 中渲染具有 intrinsic size 的图片、canvas、视频或远程桌面视图。

- **标题**：扩展管理器的 reload、安装和卸载刷新必须按 kind 且保持语言 WASM 惰性加载
- **触发信号**：重新加载、安装或卸载一个静态 composite、数据库驱动或 provider 时，UI 长时间无响应，日志出现大量 `cranelift_codegen`、`wasmtime` 或 Tree-sitter 语言扩展编译记录。
- **根因 / 约束**：统一刷新路径如果丢失扩展 kind，或调用 `load_language_extensions_from_root`，会在 GPUI 线程同步读取并编译全部语言 WASM；非语言扩展实际只需要刷新 runtime catalog 和贡献点，语言扩展也只需要更新 manifest 与文件后缀映射，parser 应在调用方首次请求语言时惰性加载。
- **正确做法**：reload、安装和卸载完成后都必须把具体 `ExtensionKind` 传给 runtime 刷新；只有 `Language` 与 `LanguageBundle` 才调用 `register_language_extension_manifests_from_root`，不得在刷新路径调用 eager 的 `load_language_extensions_from_root`。其他 kind 只调用 `refresh_global_runtime_catalog` 和 `refresh_runtime_contributions`；目录删除等文件 I/O 使用 `cx.background_spawn`，完成后回到前台更新 UI。
- **验证方式**：用 reload-scope contract 覆盖所有 `ExtensionKind`，用结构 contract 约束刷新只注册 manifest、卸载 I/O 使用 background executor；手工卸载静态 composite，确认 UI 不冻结且日志不再出现语言 Cranelift/Wasmtime 编译。
- **适用范围**：`crates/extension-runtime/src/extension_view_host.rs`、`crates/extension_view/src/actions.rs`、扩展管理页的重新加载、安装与卸载刷新路径。

- **标题**：macOS 局域网 `No route to host` 先检查本地网络权限与 App Bundle 签名
- **触发信号**：Navop 在 macOS 上可连接公网或正式服务器，但访问 `10.*`、`172.16-31.*`、`192.168.*` 等局域网地址时返回 `No route to host (os error 65)`，而终端或旧版 OnetCli 可以连接。
- **根因 / 约束**：macOS 本地网络隐私会在 TCP 建连前拒绝未授权应用；若 `Info.plist` 缺少 `NSLocalNetworkUsageDescription`，或只签名 DMG、没有签名 `.app`，系统无法把授权稳定关联到 Bundle。未经 Bundle 签名的程序可能显示基于二进制哈希的临时 Identifier，且 `Info.plist` 不受签名保护。
- **正确做法**：macOS App 声明 `NSLocalNetworkUsageDescription`，在创建 DMG 前先签名并严格验证 `Navop.app`；生产发布优先使用 Developer ID Application，缺少证书时仅将 ad-hoc 签名作为开发/临时回退。对私网 `HostUnreachable` 错误保留原始信息，并提示检查“系统设置 > 隐私与安全性 > 本地网络”、VPN、路由、代理和跳板机。
- **验证方式**：用 `nc -vz <private-ip> 22` 与 `route -n get <private-ip>` 区分系统路由和应用权限；检查 `codesign -dv --verbose=4 Navop.app` 显示预期 Bundle Identifier、已绑定 `Info.plist` 和 sealed resources，并运行 macOS bundle 脚本测试及 SSH 错误提示测试。
- **适用范围**：`resources/macos/Info.plist`、`script/bundle-macos.sh`、macOS Release/DMG 流程，以及 SSH、SFTP、数据库、远程桌面、端口转发等局域网 TCP 连接入口。

- **标题**：安装器文件关联变更必须覆盖只替换二进制的应用内更新
- **触发信号**：新版 MSI/DEB/RPM/`.app` 已声明新的文件类型，但旧用户通过应用内更新后，系统“打开方式”中仍没有 Navop，或双击文件仍无法交给新版本。
- **根因 / 约束**：Windows/Linux 应用内更新只替换可执行文件，不会重新运行 MSI 注册表组件、复制 desktop/MIME 资源或刷新桌面缓存；macOS 虽替换整个 `.app`，同路径覆盖后 LaunchServices 也可能尚未重新扫描。现代系统还会保护用户已有的默认应用选择，不能依赖静默覆盖 `UserChoice`。
- **正确做法**：把关联定义同时用于安装器和应用内启动迁移。新版本首次启动时在后台幂等执行：Windows 写入当前用户 `Software\Classes` 的 ProgID、`OpenWithProgids` 和绝对打开命令并发送 `SHCNE_ASSOCCHANGED`；Linux 将嵌入的 desktop/MIME 模板写入 `XDG_DATA_HOME`、刷新缓存，并只在当前 MIME 没有默认应用时设置 Navop；macOS 对当前 `.app` 执行 `lsregister -f`。用包含 schema 与 executable path 的 stamp 避免重复迁移，应用移动后应自动重跑。
- **验证方式**：用纯 contract 覆盖三种扩展、绝对路径/desktop `Exec` 转义、不写 `UserChoice`、已有默认应用不覆盖、stamp 幂等和 `.app` 推导；运行 main 测试与 check，并在 Windows CI 验证条件编译、在 Linux 包环境验证用户级 desktop/MIME 文件和缓存刷新。
- **适用范围**：`main/src/file_association.rs`、`main/src/main.rs`、`main/src/update/*`、`resources/{macos,linux}`、Windows WiX 和所有新增文件关联/URL scheme 的发布迁移。

- **标题**：macOS GUI 编辑器不要直接依赖 Bundle 内部 executable 处理重复打开
- **触发信号**：第一次能打开外部编辑器，但编辑器已运行时再次打开远程文件没有反应、产生第二个无效进程，或编辑器随 OnetCli 生命周期收到 `SIGHUP`。
- **根因 / 约束**：部分 macOS 应用（例如 Notepad--）依赖 LaunchServices 的 `QEvent::FileOpen` 向已有实例交付文件；直接执行 `.app/Contents/MacOS/*` 会绕过该机制。编辑器安装检测仍应检查真实 executable，不能简单把 `/usr/bin/open` 当作可用性候选。
- **正确做法**：manifest 用 `launchMode: macos_open` 声明 LaunchServices 模式，`programCandidates` 继续负责真实 executable 检测与首次确认；Host 从 executable 推导 `.app` Bundle，再以参数数组执行 `/usr/bin/open -a <bundle> <file>`，不经过 shell。Linux/Windows 和未声明该模式的编辑器保持 direct launch。
- **验证方式**：覆盖默认 direct、manifest/runtime mode 传递、`.app` 推导、非 Bundle 拒绝及完整 `open` argv；在编辑器已运行时连续打开两个文件，确认复用同一实例并正确收到文件。
- **适用范围**：`crates/remote_file_editor`、`contributes.remoteFileEditors` manifest/runtime contract，以及所有 macOS `.app` 外部编辑器扩展。

- **标题**：外部编辑器自动上传必须以磁盘写盘为边界，并用轮询补偿 watcher 丢事件
- **触发信号**：编辑器已经保存本地临时文件，但 OnetCli 偶发没有上传；或编辑器采用原子替换、文件系统 watcher 丢事件，导致只依赖事件监听不稳定。
- **根因 / 约束**：OnetCli 无法访问第三方编辑器尚未写盘的内存 buffer，也不应修改编辑器配置或模拟保存快捷键；不同编辑器的文件事件语义不一致。
- **正确做法**：Host 同时使用精确文件事件和定时轮询，先比较本地内容指纹，未变化时禁止远端 I/O；成功上传或远端重载后更新指纹以去重。全局自动上传关闭时，新会话不创建 watcher、poller 或上传 controller。
- **验证方式**：覆盖默认设置、显式关闭、内容指纹变化/不变、远端重载指纹更新；手工验证 Zed 与 Notepad-- 保存后上传，以及关闭开关后远端不变。
- **适用范围**：`crates/remote_file_editor`、远程文件编辑器设置及所有外部编辑器贡献。

- **标题**：远端写操作成功后统一刷新当前可见目录
- **触发信号**：外部编辑器上传或内置编辑器保存已经成功，但 SFTP 侧边栏仍显示旧的大小、时间或目录内容，需要手动刷新。
- **根因 / 约束**：远端编辑器与 SFTP 视图分属不同 crate，不能通过反向依赖直接刷新；同一个内置编辑器窗口还可能承载来自不同 SFTP 面板的 tab。
- **正确做法**：由调用方传入类型擦除且可克隆的远端变更成功回调，每个 tab/外部会话保存自己的回调；只有远端写成功后触发，失败、取消和只读操作不触发。回调刷新调用方当前可见目录，不改变当前路径；同路径 tab 被其他面板重新打开时更新为最新回调。
- **验证方式**：覆盖回调调用 contract；运行 `remote_file_editor`、`sftp_view`、`terminal_view` 测试和 `main` check；手工确认外部上传与内置保存后侧边栏无需手动刷新。
- **适用范围**：`crates/remote_file_editor`、`crates/sftp_view`、`crates/terminal_view/src/sidebar/file_manager_panel.rs`。

- **标题**：可见终端执行不能用 EOF 绑定 Agent 取消与命令完成
- **触发信号**：`terminal.exec` 执行 `command &`、`npm run dev &` 或 `nohup command &` 后一直 pending；点击 Agent 的 × 后对话仍显示运行中；或取消 Agent 时误向终端发送 Ctrl+C、终止仍在运行的命令。
- **根因 / 约束**：后台进程会继承 PTY/stdout/stderr，shell leader 退出不代表 reader 能收到 EOF。Agent turn、tool waiter 与终端命令若共用同一个 future，进程或 FD 清理就会反向阻塞对话终态。可见终端命令由用户终端拥有，Agent 取消无权终止它。
- **正确做法**：用 OSC 133 supervisor 独立管理 readiness、safe-replace、命令 epoch、observer 与 timeout。fresh `InputStart` 后将提示符标记为空；空 `Ready` 提示符直接提交 Agent 命令，只有 supervisor 已观测到用户或 insert-only 调用留下的未提交输入时，才发送一次 ETX 清理并等待新的 `InputStart`，避免每次执行都机械触发 Ctrl+C。提交动作必须立即把 readiness 悲观切到 `SubmissionPending`，即使 `wait_for_output=false`。命令完成以 `CommandFinished` 或新 prompt epoch 为边界，不依赖 EOF。取消前未开始的调用零写入；提交后的取消只 detach waiter，后台 supervisor 继续有界清理，并停止缓存无人消费的输出。Agent turn 同时立即发出 `TurnCancelled`，旧 turn 的迟到写入按 turn id 丢弃。诊断用户手动执行或超时后留在可见终端中的现场时，使用有界只读的 `terminal.read(lines=N)` 读取 live PTY/scrollback 尾部，不要为了重新拿输出而重复执行命令。
- **验证方式**：覆盖空提示符直接提交且零 ETX、半行输入与 insert-only 输入才清理、busy/unknown 零写入、fresh prompt 握手、`wait_for_output=false` 立即 busy、预取消不排队、取消后不发送控制字符、detached output 不增长、background/nohup 不等 EOF、`terminal.read` 行数/字符上限与滚屏 tail、旧 turn 不清理或污染新 turn；再运行 terminal/tool-runtime/agent-runtime/UI 的定向测试、check 与 clippy。
- **适用范围**：`crates/terminal/src/exec_supervisor/*`、SSH terminal actor、`terminal.exec` Public MCP/Agent adapter、`agent_runtime` turn cancellation 与 `ai_chat_view` 终态处理。

- **标题**：显式终端 Ctrl+C 必须走 supervisor control，不能复用 Agent 取消或任意输入接口
- **触发信号**：AI 需要停止当前可见终端的前台任务；有人考虑把 Agent 的 × 映射成 Ctrl+C、把 `"\\u0003"` 当作 `terminal.exec.command`，或直接向 Agent 暴露任意 PTY 字节写入。
- **根因 / 约束**：Agent turn 取消只表达“停止当前对话等待”，不拥有终端进程；`terminal.exec` 的 safe-replace 只允许在可信 `Ready` prompt 上清理半行并提交命令，而真正需要 Ctrl+C 时通常处于 `SubmissionPending` / `CommandRunning`。任意字节输入会绕过 readiness、审批和自动化 lease，产生竞态或误中断。
- **正确做法**：使用独立高风险 `terminal.control(action=interrupt)`。由 terminal actor 内的 supervisor 原子检查 readiness，仅在明确的前台运行状态写入一次 ETX (`0x03`)；其他状态全部 fail closed、零写入。control 不移除 exec observer、不伪造 exit code，真实完成仍由 OSC `CommandFinished` / prompt epoch 收口。
- **验证方式**：覆盖 running/submission-pending 允许、ready/awaiting-prompt not-running、busy/unknown/disconnected 零写入、预取消零入队、control 后 observer 仍由真实终态完成；验证 Agent prompt 区分 `terminal_exec`、`terminal_control` 与取消按钮。
- **适用范围**：`crates/terminal/src/exec_supervisor/*`、`crates/terminal/src/ssh_backend.rs`、`crates/terminal_view/src/public_mcp.rs`、Public MCP terminal control 工具与 Agent prompt。

- **标题**：Agent Auto 模式不进行工具审批，High/Critical 也直接执行
- **触发信号**：Auto 模式下出现 `NeedUserInput` 工具确认卡，或 Agent→tool_runtime 映射仍把 high-risk policy 设为 `Ask`。
- **根因 / 约束**：`ToolExecutionMode::Auto` 表达用户已授权 Agent 自主执行当前暴露工具；风险等级仍用于展示、审计和 Manual 模式审批，但不能在 Auto 模式再次暂停。`ReadOnly` 仍通过工具暴露过滤保证只读，不能用 Auto 的放行规则扩大其工具集合。
- **正确做法**：`requires_tool_approval` 对 Auto 始终返回 false；Agent runtime adapter 保持 `PermissionProfile::Auto` 标识，同时将 `high_risk_policy` 覆盖为 `Allow`。Manual 继续确认所有非 Read 业务工具，ReadOnly 只暴露 Read 工具。
- **验证方式**：覆盖 Auto 的 High、Critical、同轮多个 High 直接执行且无 `NeedUserInput`；覆盖 Manual 非 Read 仍审批、ReadOnly 仍过滤写工具；验证 Agent Auto permission policy 的 `mode=Auto` 且 `high_risk_policy=Allow`。
- **适用范围**：`crates/agent_runtime/src/tasks/agent.rs`、`crates/agent_runtime/src/tools/runtime_adapter.rs`、Agent 工具模式 UI 与相关审批测试。

- **标题**：ACP 安全确认保留 Dialog，并在权限卡解释二次审批
- **触发信号**：ACP 权限卡允许后又弹出 Public MCP 安全确认 Dialog，用户误以为发生了无意义的重复审批；或者自动执行模式仍显示二次审批提示、High/Critical 工具仍继续请求确认。
- **根因 / 约束**：ACP `request_permission` 与实际 Public MCP `tools/call` 是两个独立安全边界。安全确认模式需要用强制可见的 Dialog 承担最终审批，但如果消息卡不解释模式来源，用户会把它理解为异常重复；Auto 模式则不应保留确认语义。
- **正确做法**：在“手动确认/安全确认”模式的 ACP 权限卡中明确说明：允许 ACP 后，实际工具执行还会弹出一次安全确认窗口；如不需要二次审批，可将 MCP 权限模式切换为“自动执行”。Public MCP Ask 统一进入全局 Dialog，不再通过 ACP 一次性路由替换成第二张可操作消息卡。自动执行模式不显示上述提示，且 High/Critical 一并直接执行。
- **验证方式**：覆盖手动确认卡片包含“安全确认、二次审批、自动执行”说明，自动执行卡片不含误导提示，ACP 后续 Public MCP 请求进入 Dialog 队列并展示完整脱敏参数，以及 Auto High/Critical 直接执行。
- **适用范围**：`crates/ai_chat_view/src/acp/*`、`agent_transcript.rs`、`agent_view.rs`、`crates/public_mcp/src/permissions.rs`、`main/src/public_mcp_approval*`。

- **标题**：macOS 自定义标题栏中的可拖元素必须由应用显式接管标题栏拖动
- **触发信号**：透明标题栏或 tab 栏中，按钮、输入框或 tab 的拖动被解释为窗口移动；为了规避问题出现 `allow_tab_drag = !is_macos` 一类平台禁用逻辑。
- **根因 / 约束**：GPUI 的 `stop_propagation()` 和 `prevent_default()` 只影响 GPUI 事件传播，不能阻止 AppKit 把透明标题栏视为系统 window-move region。把 `NSWindow.isMovable` 设为 false 虽能规避抢事件，但会禁用 macOS Window 菜单的平铺与窗口管理快捷键。
- **正确做法**：使用包含 Zed/GPUI #60620 的提交或更新版本，主窗口保持 `is_movable: true` 并设置 `app_owns_titlebar_drag: true`；空白标题栏通过 `Window::start_window_move()` 显式拖窗，可交互子元素在 GPUI 层阻止冒泡并保留自身 `on_drag`。不要用 macOS 条件整体禁用 tab drag。
- **验证方式**：结构测试保证不存在 `allow_tab_drag = !is_macos` 且窗口启用 `app_owns_titlebar_drag`；macOS 手工验证空白区拖窗、tab 排序、终端 tab 拖动分屏、标题栏按钮点击和 Window 菜单 tiling 均可用。
- **适用范围**：`main/src/main.rs`、`crates/core/src/tab_container.rs`，以及任何放在 macOS 透明自定义标题栏内的可点击、可选择或可拖拽 GPUI 元素。

- **标题**：GPUI 拖放解析源 Entity 时必须在 `read` 前排除当前更新 Entity
- **触发信号**：拖动 tab、pane 或其他实体到自身内容区时出现 `cannot read <Entity> while it is already being updated`，macOS 上随后因 panic 穿过原生回调边界触发 `SIGABRT`。
- **根因 / 约束**：`Context<T>` 回调已经持有当前 `Entity<T>` 的可变更新权限；若源解析先 downcast 得到同一个 entity，再调用 `read(cx)` 检查其状态，就会形成 update 中读取自身的重入。事后比较源/目标是否相同已经太晚。
- **正确做法**：源解析函数显式接收目标 entity handle；downcast 后先只比较 `Entity`/`EntityId`，相同立即返回，确认是外部 entity 后才允许 `read(cx)`。不要通过捕获 panic 或延迟通知掩盖重入。
- **验证方式**：补 contract 测试保证同实体 guard 在首次 `read(cx)` 之前；手工把当前 tab 拖回自己的内容区，确认无 drop overlay、无 panic，再验证拖到另一个 workspace 仍可正常转移。
- **适用范围**：`crates/terminal_view/src/workspace/tab_drag.rs`，以及所有从 GPUI drag payload、AnyView downcast 或 registry handle 解析实体并读取状态的事件回调。

- **标题**：Redis String 读取链路必须保留原始字节，不能默认按 UTF-8 解码
- **触发信号**：查看 Java 序列化、Protobuf、MessagePack、压缩内容等 Redis String 值时出现 `Cannot convert from UTF-8`，或为了消除错误而考虑使用 `String::from_utf8_lossy`。
- **根因 / 约束**：Redis 的 String/Bulk String 是二进制安全字节串，不保证 UTF-8；`query_async::<String>` / `Option<String>` 会在 redis-rs 转换层提前失败，而 lossy 转换会丢失原始字节并可能在保存时破坏数据。
- **正确做法**：值详情读取使用 `Vec<u8>` 保留原始内容；合法 UTF-8 继续按文本展示和编辑，非法 UTF-8 使用转义 Raw、Hex 或 Binary 展示，并在没有字节安全编辑/写回 contract 时保持只读。原始命令结果同样应映射为显式 Binary 类型。
- **验证方式**：用 Java 序列化头 `AC ED 00 05` 覆盖连接层原始字节保留、Raw/Hex/Binary 格式化和非 UTF-8 只读保护；同时验证普通中文/JSON 文本仍可正常显示编辑。
- **适用范围**：`crates/redis_view/src/connection.rs`、`key_value_view.rs`、`types.rs`，以及 Redis String、集合成员、Hash/Stream 字段等所有可能承载二进制 bulk string 的读取链路。

- **标题**：过滤树的自动展开必须与用户显式折叠分开建模
- **触发信号**：输入搜索词后匹配路径会自动展开，但点击展开箭头无法收起，或收起后下一次重建扁平列表又立即展开。
- **根因 / 约束**：搜索态通常需要派生“自动展开匹配路径”的可见性；若遍历逻辑在搜索时无条件忽略普通展开集合，就无法区分“尚未手动操作”和“用户明确要求折叠”。单一 `expanded_nodes` 集合不足以表达这两个来源。
- **正确做法**：保留普通 `expanded_nodes` 作为搜索结束后的持久状态，并增加仅在当前搜索词下有效的显式折叠 override；渲染箭头、点击切换和子树遍历统一读取同一个 effective expansion contract。搜索词变化时清空 override，用户在搜索中展开/折叠时同步更新普通状态以决定退出搜索后的结果。
- **验证方式**：纯 contract 覆盖搜索自动展开、显式折叠优先、非搜索态遵循普通展开状态；真实树测试确认搜索中箭头可反复收起/展开，修改搜索词后重新自动展示匹配路径，清空搜索后保留用户最后的普通展开选择。
- **适用范围**：`crates/redis_view/src/redis_tree_view.rs`，以及数据库树、文件树、资源树等同时支持过滤和层级展开的 GPUI 树视图。

### 执行原则

1. 先澄清，再实现；先缩小边界，再扩展范围。
2. 优先局部修改与最小充分实现，避免无关扩张。
3. 若任务复杂度上升，应及时升级流程，而不是硬撑轻流程。
4. 若任务收敛为局部改动，应及时降级流程，避免形式成本。

### How to Report Bugs

- 清晰描述 bug 的现象、触发条件、预期行为与实际行为。
- 给出尽量稳定可复现的步骤。
- 说明真实影响范围与严重程度。
- 清晰解释真实世界中的后果，并关联影响严重程度和修复优先级。
- 收集有助于诊断 bug 的上下文信息，例如使用模式、错误信息、堆栈跟踪、日志、环境配置与版本。

### Bug Fixing

- 不直接猜测式修补，先确认根因，再决定修复策略
- 优先通过复现、日志、调用链、最小实验或二分缩小问题范围
- 修复后按测试策略与验证门禁完成收口

### Testing Standards

- 测试优先覆盖关键路径、边界情况和错误路径。
- 测试应具体、可读、稳定，避免脆弱测试。
- 对 `assertEqual` 类断言，优先遵循“expected 在前，actual 在后”。
- 是否需要 TDD，按全局测试策略判定，不在本节重复规定。

### How to Write Code 如何编写代码

- 遵循 SOLID、DRY、关注点分离与 YAGNI。
- 命名应清晰、抽象应务实。
- 仅在关键或不直观逻辑处添加简短注释，避免注释噪音。
- 修改行为时，优先移除死代码和明显过时的兼容路径，除非用户明确要求保留。
- 明确处理边界条件，不要隐藏失败。
- 关注时间复杂度和空间复杂度，尤其在高 IO 或高内存路径上。
- 新增文档字符串时，保持简洁，说明目的、关键假设与实现理由。
- 除非没有更合理方案，不主动添加大范围 linter 抑制注释。

#### 代码指标（硬性上限）

- **函数长度**：≤ 50 行（不含空行）
- **文件大小**：≤ 300 行
- **嵌套深度**：≤ 3 层
- **参数数量**：位置参数 ≤ 3
- **圈复杂度**：每函数 ≤ 10
- **禁止魔法数字**：提取为具名常量

### Refactoring Standards

- 默认优先保持行为不变，再提升结构质量。
- 重构应在测试或验证保护下进行，必要时先补测试再重构。
- 如果检测到循环导入，请将共享逻辑提取到新工具模块或现有模块中，以保持依赖图无环。
- 对较大的重构，优先先拆分计划，再按步骤推进。
- 重构完成后，仍需进行独立 review 与完成前验证。

### Safety Rules 安全规则

- 不要运行破坏性命令，例如 `git reset`，除非用户明确要求。
- 不要使用非 Git 工具操作 `.git` 目录。
- 避免危险删除命令，除非其作用范围被明确限制在临时产物。
- 不要将密钥、凭证、API Key 硬编码进源码文件。
- 数据库访问应使用参数化查询。
- 不要通过拼接不可信输入来构造 shell 命令或 SQL。
- 在系统边界校验并清理外部输入。
- 除非用户明确要求，否则不要终止非当前任务启动的进程。

## 沟通与协作

### 沟通风格

#### 语言约定

- 默认使用简体中文回答，可混用英文技术术语。
- 代码标识符使用英文。
- 代码注释优先简体中文，保持简洁清晰。

#### 混合输出模式

根据任务类型选择合适的输出风格：

- 执行类任务：强调进度、当前动作、下一步
- 分析类任务：强调结论、依据、权衡

##### 模式 A：执行进度式

适用场景：代码修改、重构、bug 修复、多步任务、文件操作

推荐结构：

🎯 任务：一句话描述当前任务

📋 执行计划：
- ✅ 已完成
- 🔄 进行中
- ⏸ 待执行

🛠️ 当前进度：
详细描述当前正在做什么，已完成什么

⚠️ 风险/阻塞：
潜在问题、注意点、阻塞因素

📎 参考：`file:line`

##### 模式 B：分析回答式

适用场景：问答、代码解释、方案对比、架构分析、问题诊断

推荐结构：

✅ 结论：1-2 句直接回答核心问题

🧠 关键分析：
1. 核心观点
2. 依据
3. 权衡

🔍 深入剖析：（可选）
📊 方案对比：（可选）
🛠️ 实施建议：（可选）
⚠️ 风险与权衡：（可选）

#### 技术内容规范

- 多行代码、配置、日志，优先使用带语言标识的 Markdown 代码块。
- 示例代码聚焦核心逻辑，省略无关部分。
- 需要强调变更时，可使用 `+ / -` 辅助表达差异。
- 仅在确有必要时使用表格。

#### 输出结尾建议

- 复杂内容后附简短总结，重申核心要点。
- 结尾给出实用建议、行动指南或鼓励进一步提问。

### 子代理派发策略

- 仅在任务可明确拆分、并行收益真实存在或需要独立审查时派发子代理。
- 子代理不限制模型或推理等级，按当前环境可用能力、任务特点与成本选择即可。
- 派发时明确目标、范围、输入、预期输出、验证方式和禁止触碰的区域。
- 避免多个代理同时修改同一文件或同一逻辑边界；若无法避免，由主代理负责协调顺序与合并。
- 主代理必须复核子代理的结论、代码和验证证据，并对最终交付负责。

@RTK.md
