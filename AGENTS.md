# 全局 Agent 规则

本文件用于约束自动化代理在本机工作区中的默认工作方式，并将 Superpowers 作为主工作流体系进行选择性激活。

## 指令优先级

- 指令优先级从高到低：
  1. 当前会话中用户的明确要求
  2. 仓库自身的规则、文档与约定
  3. 本 `AGENTS.md` 的任务分流与流程规则
  4. 相关 Superpowers / skill 的流程定义
- 默认以 **Superpowers** 作为主工作流体系。
- 但不默认启用 **full Superpowers**。
- 对不同任务类型，按需激活对应的 Superpowers skills。
- 本文件保留个人硬门禁、环境约束、交付偏好与沟通方式，不再重复展开各 skill 已定义的全部细节。
- 若本文件某项规则被明确视为“个人硬门禁”，则即使使用 skill，也仍需满足。
- 对于仅涉及审查、分析、解释或方案讨论而不修改仓库文件的任务，可不进入完整实现流程，但仍应保持推理清晰、结论可追溯。
- 如果用户明确要求 `continue nonstop`，则默认持续推进，直到满足验收标准或出现真实阻塞。

## 默认工作流原则

### 最短路径原则

- 默认采用“**满足质量要求的最短路径**”。
- 能直接完成并验证的，不升级为更重流程。
- 能使用轻量版 planning 完成的小任务，不升级为重文档流程。
- 能使用单一专项 skill 解决的问题，不扩展为 full Superpowers。
- 流程的目标是降低返工与风险，而不是增加形式成本。

### Superpowers 主体系原则

- Superpowers 是默认主工作流体系，但不是默认全量流程。
- 对实现类任务，原则上必须先完成需求澄清与计划拆分。
- 对只读任务，不强制进入实现流程。
- 对 review、完成前验证、debug、前端设计等环节，使用对应的 Superpowers skills。

## 任务分流模型

### 只读任务

以下任务可直接处理，不强制进入实现流程：

- 分析
- 解释
- 架构说明
- 代码阅读
- 纯信息型问答
- 不修改文件的只读审查

若任务属于真实问题排查，但尚未进入修改，可直接优先使用：

- `systematic-debugging`

### 实现类任务

以下任务原则上必须先完成需求澄清与计划拆分：

- `brainstorming`
- `writing-plans`

适用：

- 新功能
- bug 修复
- 行为变更
- 重构
- 页面 / 组件 / API / 脚本实现
- 数据处理逻辑改动

默认流程：

1. 用 `brainstorming` 澄清目标、边界、约束、验收标准
2. 用 `writing-plans` 形成实现计划
3. 再进入具体实现
4. 根据任务性质按需调用后续技能

#### 轻量版 planning

- 对小任务，允许采用**轻量版** `brainstorming` 与 `writing-plans`
- 可在当前对话内完成，不强制产出长文档
- 最小集合至少应明确：
  - 目标
  - 边界
  - 风险
  - 验证方式
- 只有当任务规模、风险或不确定性明显上升时，才升级为更重的 planning 流程

### Debug 类任务

以下任务优先执行：

- `systematic-debugging`

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
- 先做根因分析，再决定修复方案

### Review 类任务

以下任务必须执行：

- `requesting-code-review`

适用：

- review
- code review
- reviewer output
- spec compliance
- 合并前审查
- 阶段性交付审查

若任务是在处理 review 反馈，则使用：

- `receiving-code-review`

### 完成前验证

在声称以下状态前，必须执行：

- `verification-before-completion`

适用：

- 完成
- 已修复
- 可提交
- 可合并
- 测试通过
- 验证通过

### 前端任务

涉及以下内容时，必须执行：

- `ui-ux-pro-max`

适用：

- UI
- UX
- 页面布局
- 组件视觉
- 图表呈现
- 交互设计

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

- `test-driven-development` 不作为所有实现类任务的默认强制技能。
- TDD 是否启用，不按任务大小机械决定，而按“行为影响、共享范围、回归风险、测试价值”显式判定。

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

#### C. 必须使用 TDD

适用于：

- 新功能开发
- 明确行为变更
- 公共 API / contract 变更
- 跨模块共享逻辑修改
- 数据库 / 持久化 / 并发 / 状态机相关改动
- 高风险或高回归风险任务

处理方式：

- 必须调用 `test-driven-development`

### 质量门禁分层

#### Level 0：定向验证

- 适用于局部、低风险、小改动
- 直接修改后执行定向验证

#### Level 1：回归测试

- 适用于中小修复或局部行为变化
- 修复后补最贴近问题表面的回归测试

#### Level 2：TDD

- 适用于新功能、明确行为变更、共享逻辑或高风险改动
- 使用 `test-driven-development`

#### Level 3：Code Review

- 适用于阶段收尾、中高风险改动、合并前审查
- 使用 `requesting-code-review` / `receiving-code-review`

#### Level 4：Completion Verification

- 适用于所有准备声称完成、已修复、可提交、可合并的任务
- 使用 `verification-before-completion`

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

- **标题**：可见终端执行不能用 EOF 绑定 Agent 取消与命令完成
- **触发信号**：`terminal.exec` 执行 `command &`、`npm run dev &` 或 `nohup command &` 后一直 pending；点击 Agent 的 × 后对话仍显示运行中；或取消 Agent 时误向终端发送 Ctrl+C、终止仍在运行的命令。
- **根因 / 约束**：后台进程会继承 PTY/stdout/stderr，shell leader 退出不代表 reader 能收到 EOF。Agent turn、tool waiter 与终端命令若共用同一个 future，进程或 FD 清理就会反向阻塞对话终态。可见终端命令由用户终端拥有，Agent 取消无权终止它。
- **正确做法**：用 OSC 133 supervisor 独立管理 readiness、safe-replace、命令 epoch、observer 与 timeout。只有 `Ready` 才能发送 ETX，收到 fresh `InputStart` 后再提交命令；提交动作必须立即把 readiness 悲观切到 `SubmissionPending`，即使 `wait_for_output=false`。命令完成以 `CommandFinished` 或新 prompt epoch 为边界，不依赖 EOF。取消前未开始的调用零写入；提交后的取消只 detach waiter，后台 supervisor 继续有界清理，并停止缓存无人消费的输出。Agent turn 同时立即发出 `TurnCancelled`，旧 turn 的迟到写入按 turn id 丢弃。
- **验证方式**：覆盖 busy/unknown 零写入、半行输入清理、fresh prompt 握手、`wait_for_output=false` 立即 busy、预取消不排队、取消后不发送控制字符、detached output 不增长、background/nohup 不等 EOF、旧 turn 不清理或污染新 turn；再运行 terminal/tool-runtime/agent-runtime/UI 的定向测试、check 与 clippy。
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

- 真实 bug 默认优先使用 `systematic-debugging`
- 不直接猜测式修补，先确认根因，再决定修复策略
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
- 重构完成后，仍需回到 review 与 completion verification。

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

- 任何 `spawn_agent` 调用都必须显式设置 `model` 与 `reasoning_effort`。
- 除非存在更高优先级指令的强制覆盖，子代理模型仅允许使用 `gpt-5.4` 与 `gpt-5.3-codex`。
- 子代理默认使用 `gpt-5.4`。
- 仅当任务以代码实现、测试修复、局部重构、单模块阅读与分析为主，且不需要复杂跨模块推理时，才可使用 `gpt-5.3-codex`。
- `reasoning_effort` 仅允许使用 `high` 或 `xhigh`。
- 若任务复杂度存在明显歧义，一律上调为 `xhigh`。
- 派发前应先判断是否确有委派价值；一旦决定派发，必须在回复中简要说明本次选择该模型与推理等级的原因。

### 技能（Skills）

- 技能存放位置：`~/.codex/skills/`（个人）与 `.codex/skills/`（项目共享，可选）。
- 开始任务前，应优先判断是否存在匹配的 superpower 或 skill。
- 若任务命中 skill，阅读其 `SKILL.md` 并按流程执行。
- 本文件默认采用以下主干整合方式：
  - 实现前：`brainstorming -> writing-plans`
  - debug：`systematic-debugging`
  - review：`requesting-code-review` / `receiving-code-review`
  - 完成前：`verification-before-completion`
  - 高风险行为变更：`test-driven-development`
  - 前端设计：`ui-ux-pro-max`
- 在回复中声明本次使用了哪些技能。

@RTK.md
