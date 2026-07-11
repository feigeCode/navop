# ACP Runtime Reliability Design

## 背景

OnetCli 已通过 `agent-client-protocol` 接入外部 ACP Agent，并可从 ACP Agent 扩展目录加载 Claude、Codex、OpenCode 等 stdio 启动器。现有链路包含扩展发现、子进程启动、ACP initialize、鉴权、新建 session、prompt、`SessionUpdate` 翻译和聊天 UI 渲染，但“协议请求返回成功”目前被近似等同于“用户已经得到有效回答”。

本机日志证明协议基础链路可以运行，同时也暴露出以下可用性缺口：

- OpenCode 可以完成 initialize、创建 session，并对 prompt 返回 `end_turn`，但没有发送文本、推理、工具或计划事件。当前客户端仍发送 `TurnCompleted`，UI 清除等待状态后只剩空白。
- Codex 公布 `api-key` 和 `chat-gpt` 鉴权方式，但客户端通过 auth method id 猜环境变量。`api-key` 无法可靠映射到 `OPENAI_API_KEY` 或 `CODEX_API_KEY`。
- 鉴权失败或超时后，客户端仍继续创建 session。这会把“未鉴权”伪装成“连接成功”，直到首轮 prompt 才暴露 401 等错误。
- ACP 配置来源已从设置和环境变量切换为扩展 manifest，缺少稳定的用户级覆盖层。用户无法在不修改已安装扩展的情况下指定 auth method、环境变量引用或启动参数。
- 外部 provider 返回的无效 token、模型下线、HTTP 状态和嵌套 JSON-RPC 错误缺少统一提取，UI 经常只显示 `Internal error`。
- `connection.rs` 同时负责传输、鉴权、文件系统请求、生命周期和单轮完成判定，已经不适合继续承载新的可靠性逻辑。

本设计改造公共 ACP 客户端内核，而不是为 Claude、Codex、OpenCode 分别增加临时特判。

## 目标

1. Claude、Codex、OpenCode 共用一套可诊断的 ACP 生命周期。
2. ACP 连接只有在满足所选鉴权策略并成功创建 session 后才进入 Ready。
3. prompt 只有产生有效输出后才可以被标记为成功；无输出的 `end_turn` 必须成为明确错误。
4. 用户能在不修改扩展安装目录、不明文保存密钥的前提下覆盖 ACP Agent 的鉴权和运行配置。
5. UI 能区分启动、初始化、等待鉴权、创建会话、就绪、运行和失败状态，并给出可执行的恢复建议。
6. 保持扩展 manifest 向后兼容，继续复用 `RuntimeEvent`、`AgentTranscript`、工具卡片和 ACP 权限审批。
7. 用单元测试和协议级集成测试保护鉴权选择、错误提取、空响应、取消、超时和进程退出行为。

## 非目标

- 不重写一套 ACP 专属聊天转录和卡片 UI。
- 不在本轮实现密钥持久化；真实凭证继续来自进程环境或外部 CLI 的安全登录状态。
- 不修复第三方 provider 的无效 token、下线模型或服务中断；OnetCli 负责准确识别并解释这些错误。
- 不自动修改用户的 `~/.codex`、Claude Code 或 OpenCode 配置。
- 不把 ACP Agent 的厂商业务逻辑写入 `ai_chat_view`。
- 不在本轮实现终端能力；客户端继续只公布实际已经注册的文件系统能力。

## 方案选择

### 方案 A：只增加错误提示和空响应检测

该方案改动最少，但只能把静默失败变成可见失败，不能解决配置入口、鉴权选择和连接假成功问题。

### 方案 B：统一 ACP 会话内核、配置覆盖和结构化诊断

该方案保留现有扩展体系和 UI 复用，通过配置模型、生命周期状态机、结构化错误和单轮追踪修复公共根因。它能支持当前三个 Agent，也能为后续 ACP 扩展提供稳定 contract。

### 方案 C：独立重写 ACP runtime 和聊天 UI

该方案会复制 `agent_runtime`、转录、权限和工具卡片能力，范围大且背离统一工具运行时方向。

采用方案 B。

## 总体架构

ACP 链路拆分为五个责任边界：

```text
ACP extension manifest
        │
        ├── defaults ──────────────┐
        │                          ▼
user acp-agents.json ───────► config resolver
                                   │ resolved AcpAgentConfig
                                   ▼
                           connection lifecycle
                        start → init → auth → session
                                   │
                    ┌──────────────┴──────────────┐
                    ▼                             ▼
               session state                 turn tracker
                    │                             │
                    └──────── SessionUpdate ─────┘
                                   │
                                   ▼
                        RuntimeEvent translation
                                   │
                                   ▼
                         existing AgentTranscript
```

`extension-runtime` 负责声明和校验扩展默认配置，`main` 负责读取用户覆盖并生成最终配置，`ai_chat_view::acp` 负责协议生命周期和事件语义。任何一层都不读取另一层的内部表示。

## 配置模型

### 扩展 manifest

`acp_agent.json` 的每个 agent 增加可选 `auth` 与 `timeouts` 字段。旧 manifest 不包含这些字段时仍能解析。

```json
{
  "id": "codex-acp",
  "name": "Codex",
  "transport": {
    "type": "stdio",
    "command": "bin/codex-acp",
    "args": [],
    "env": {}
  },
  "auth": {
    "preferred_method": "api-key",
    "allow_unauthenticated_fallback": true,
    "methods": [
      {
        "id": "api-key",
        "env_any": ["OPENAI_API_KEY", "CODEX_API_KEY"],
        "env_all": [],
        "interactive": false
      },
      {
        "id": "chat-gpt",
        "env_any": [],
        "env_all": [],
        "interactive": true
      }
    ]
  },
  "timeouts": {
    "connect_seconds": 30,
    "authenticate_seconds": 120,
    "prompt_seconds": 600
  }
}
```

`env_all` 中的变量必须全部存在，`env_any` 非空时至少有一个变量存在。两个列表都为空表示该 method 不依赖环境凭证。`allow_unauthenticated_fallback` 表示 Agent 可以使用自身已有的本地登录状态。只有该字段为 `true` 时，客户端才允许在没有可调用 auth method 的情况下继续创建 session。旧 manifest 的兼容默认值为 `true`，避免现有本地登录型 Agent 在升级后全部失效；新 manifest 应显式声明该行为。

扩展 manifest 只允许为 stdio transport 引用包内相对 command。现有路径穿越、文件存在和 Unix 可执行位校验继续保留。

### 用户覆盖

用户覆盖文件固定为：

```text
~/.config/one-hub/acp-agents.json
```

第一阶段支持以下字段：

```json
{
  "version": 1,
  "agents": {
    "codex-acp.codex-acp": {
      "auth_method": "api-key",
      "env": {
        "OPENAI_API_KEY": "${env:OPENAI_API_KEY}"
      },
      "args": []
    },
    "opencode-acp.opencode-acp": {
      "auth_method": "opencode-login"
    }
  }
}
```

约束如下：

- 环境变量值只接受 `${env:NAME}` 引用或非敏感字面量。
- 名称以 `KEY`、`TOKEN`、`SECRET`、`PASSWORD`、`CREDENTIAL` 结尾的变量不允许使用字面量，避免凭证明文落盘。
- 引用的宿主环境变量不存在或为空时，配置解析返回结构化的 missing credential 信息，而不是悄悄删除该变量。
- 已安装扩展的用户覆盖可以修改 args、env、auth method 和 timeout，但不能覆盖 command，也不能放宽扩展包路径校验。
- 后续若支持完全自定义 Agent，应使用独立的 `custom_agents` 配置节点并明确允许绝对 command；该能力不属于本轮范围。

最终合并顺序为：

```text
extension manifest defaults
    < user ACP override
    < per-view transient skill environment
```

同名环境变量由后一层覆盖。skill context 只附加 OnetCli 自有变量，不得覆盖用户凭证变量。

## 运行时配置类型

`AcpAgentConfig` 表示已经完成合并和环境引用解析的运行时配置：

```rust
pub struct AcpAgentConfig {
    pub id: SharedString,
    pub name: SharedString,
    pub transport: AcpTransport,
    pub auth: AcpAuthConfig,
    pub timeouts: AcpTimeoutConfig,
}

pub struct AcpAuthConfig {
    pub requested_method: Option<String>,
    pub preferred_method: Option<String>,
    pub allow_unauthenticated_fallback: bool,
    pub methods: Vec<AcpAuthMethodConfig>,
}

pub struct AcpAuthMethodConfig {
    pub id: String,
    pub env_any: Vec<String>,
    pub env_all: Vec<String>,
    pub interactive: bool,
}
```

timeout 采用有界默认值：连接 30 秒、非交互鉴权 30 秒、交互鉴权 120 秒、单轮 prompt 600 秒、drop 后强制 abort 宽限 2 秒。用户覆盖必须限制在 1 到 3600 秒之间。

## 生命周期状态机

连接状态由显式枚举表示：

```rust
pub enum AcpConnectionPhase {
    Starting,
    Initializing,
    AuthenticationRequired { methods: Vec<AcpAuthMethodSummary> },
    Authenticating { method_id: String },
    CreatingSession,
    Ready,
    RunningTurn { turn_id: TurnId },
    Failed { error: AcpError },
    Closed,
}
```

合法转换如下：

```text
Starting → Initializing
Initializing → Authenticating | AuthenticationRequired | CreatingSession | Failed
AuthenticationRequired → Authenticating | Closed
Authenticating → CreatingSession | AuthenticationRequired | Failed
CreatingSession → Ready | Failed
Ready → RunningTurn | Closed
RunningTurn → Ready | Failed | Closed
Failed → Closed
```

连接入口返回显式 outcome：

```rust
pub enum AcpConnectOutcome {
    Ready(AcpConnection),
    AuthenticationRequired(AcpPendingConnection),
}
```

`AcpConnection` 只有到达 Ready 后才可以构造。交互式鉴权需要用户动作时返回 `AcpPendingConnection`，其中保留尚未关闭的协议句柄、可选 methods 和 lifecycle；UI 调用其 `authenticate` 后继续创建 session。它不是普通连接失败，也不能被自动回退到本地 Agent。

切换 Agent、切回内置 Agent、关闭 view 或应用退出时，生命周期先发送 shutdown，等待短暂宽限后 abort，保证 stdio 子进程不会长期残留。

## 鉴权决策

鉴权选择只使用显式配置，不再通过 method id 猜环境变量名。

决策顺序：

1. Agent 未公布 auth methods：直接创建 session。
2. `requested_method`（来自用户 `auth_method` 覆盖）存在时：该 method 必须同时被 Agent 公布并存在于最终 auth 配置，否则返回 `UnsupportedAuthMethod`。
3. 未指定时优先选择 manifest 的 `preferred_method`，前提是 Agent 公布了该 method。
4. 否则依次选择第一个满足 `env_all` 且满足 `env_any` 的非交互 method。
5. 否则选择第一个交互 method并进入 `AuthenticationRequired`。
6. 没有可用 method 且 `allow_unauthenticated_fallback` 为 `true`：记录本地登录回退并创建 session。
7. 其余情况返回 `MissingCredentials`。

非交互 authenticate 失败或超时后停止连接，不继续创建 session。交互 authenticate 失败后回到 `AuthenticationRequired`，允许用户重试或取消。

日志可以记录 method id 和缺少的变量名，但绝不记录环境变量值。

## 单轮完成语义

每次 prompt 创建独立 `AcpTurnTracker`：

```rust
struct AcpTurnTracker {
    turn_id: TurnId,
    received_assistant_content: bool,
    received_reasoning: bool,
    received_tool_activity: bool,
    received_plan: bool,
    cancelled: bool,
}
```

以下 `SessionUpdate` 视为有效输出：

- 非空 `AgentMessageChunk`
- 非空 `AgentThoughtChunk`
- `ToolCall`
- `ToolCallUpdate`
- `Plan`

metadata update、空文本块和 user message echo 不计为 Agent 输出。

prompt RPC 完成后的判定：

- tracker 有有效输出且 RPC 成功：发送 `TurnCompleted`，phase 回到 Ready。
- tracker 没有有效输出且 stop reason 为取消：发送取消结果，phase 回到 Ready。
- tracker 没有有效输出且 RPC 成功：发送 `TurnFailed(AcpError::EmptyResponse)`，phase 回到 Ready；连接仍可重试。
- RPC 返回错误：提取结构化错误并发送 `TurnFailed`；可恢复错误回到 Ready，连接级错误进入 Failed。
- 超时：发送 `session/cancel`，等待短暂取消宽限，然后发送 `PromptTimeout` 并回到 Ready。
- stdio/HTTP 连接关闭：当前轮发送 `ConnectionClosed`，连接进入 Failed。

同一连接任一时刻只允许一个 active turn。重复 prompt 返回 `TurnAlreadyRunning`，不覆盖现有 turn id。

## 结构化错误

`AcpError` 至少包含：

```rust
pub enum AcpErrorKind {
    CommandNotFound,
    CommandNotExecutable,
    ProcessExited,
    ConnectTimeout,
    InitializeFailed,
    ProtocolVersionMismatch,
    UnsupportedAuthMethod,
    MissingCredentials,
    AuthenticationFailed,
    AuthenticationTimeout,
    SessionCreationFailed,
    TurnAlreadyRunning,
    PromptFailed,
    PromptTimeout,
    EmptyResponse,
    ConnectionClosed,
    InvalidUserConfig,
}
```

错误值携带：

- `kind`
- agent id 和展示名
- 当前 phase
- 用户可读 summary
- 不含秘密的 technical detail
- recovery action

JSON-RPC error 提取优先级：

1. `data.message`
2. `data.additionalDetails`
3. 顶层 `message`
4. 序列化后的脱敏 `data`

若 data 中存在 `httpStatusCode`，一并显示。错误格式化必须去除 ANSI 控制序列，并对疑似 token、authorization header 和 key-value secret 做脱敏。

典型用户提示：

```text
Codex ACP 鉴权失败

需要 OPENAI_API_KEY、CODEX_API_KEY，或完成 ChatGPT 登录。
Agent 已启动并完成协议初始化，但当前没有可用凭证。

技术详情：ACP auth methods = [api-key, chat-gpt]
```

## UI 集成

现有 Agent 选择器保持不变。连接过程不再通过临时 system message 模拟状态，而是把 `AcpConnectionPhase` 转成 composer 状态和一条可替换的状态卡：

- Starting：正在启动 Agent
- Initializing：正在协商 ACP 协议
- AuthenticationRequired：需要登录或配置凭证，提供“登录”和“取消”动作
- Authenticating：正在完成鉴权
- CreatingSession：正在创建工作区会话
- Ready：输入框可用
- RunningTurn：显示现有“ACP 正在响应…”状态
- Failed：显示摘要、技术详情和恢复建议

状态卡在 phase 变化时原位更新，不能累积成聊天噪音。正常 assistant 输出开始后，运行状态卡移除；失败时由错误卡替换。

第一阶段不增加完整设置页面。错误卡提供“打开 ACP 配置文件”动作时，使用现有外部编辑器能力打开 `acp-agents.json`；若文件不存在，创建只包含 schema 版本和空 agents 对象的安全模板。该动作属于用户明确触发，不在应用启动时自动创建文件。

## 模块拆分

为满足每文件不超过 300 行、每函数不超过 50 行的项目硬门禁，ACP 代码按职责拆分：

```text
crates/ai_chat_view/src/acp/
├── auth.rs          鉴权决策与 authenticate 流程
├── client.rs        initialize、session RPC 和协议 handlers
├── config.rs        最终运行时配置与 transport 构造
├── connection.rs    对外连接句柄和生命周期编排
├── error.rs         AcpError、JSON-RPC 错误提取与脱敏
├── permission.rs    现有权限请求处理
├── state.rs         Agent 元数据和连接 phase 快照
├── translate.rs     SessionUpdate → RuntimeEvent
└── turn.rs          active turn、有效输出、超时和完成判定
```

扩展默认配置解析继续位于：

```text
crates/extension-runtime/src/extension/acp_agent_provider.rs
```

用户覆盖解析和最终配置合并位于：

```text
main/src/ai_chat_acp.rs
main/src/ai_chat_acp/user_config.rs
```

若拆分后 `main/src/ai_chat_acp.rs` 仍低于 300 行，可将 `user_config.rs` 作为 `main/src/ai_chat_acp/user_config.rs` 子模块；否则继续按解析、合并职责拆分。实现计划必须以实际行数验证为准。

## 数据流

### 应用启动与 Agent 刷新

1. ExtensionRegistry 初始化并发现 `acp_agents/*/acp_agent.json`。
2. provider 解析和校验扩展默认配置。
3. `main::ai_chat_acp` 读取用户覆盖；文件不存在等价于空覆盖。
4. resolver 按 agent id 合并配置并解析环境引用。
5. 无效的单个用户覆盖只禁用对应 Agent，并把诊断附加到该 Agent；不能导致全部 ACP Agent 消失。
6. AgentChatView 获得可运行配置与不可运行诊断，选择器可以展示禁用原因。

配置 provider 因此返回 entry，而不是只返回可运行 config：

```rust
pub struct AcpAgentEntry {
    pub id: SharedString,
    pub name: SharedString,
    pub config: Option<AcpAgentConfig>,
    pub diagnostic: Option<AcpConfigDiagnostic>,
}
```

`config` 与 `diagnostic` 必须恰有一个存在。无效 entry 在选择器中保留但禁用，避免“配置错误后 Agent 无声消失”。

### 连接

1. 用户选择 ACP Agent。
2. lifecycle 启动 transport 并进入 Starting。
3. initialize 成功后保存 capabilities 和 auth methods。
4. auth resolver 选择非交互、交互或本地登录回退。
5. 鉴权完成后创建 workspace session。
6. 成功后发布 Ready，并启用输入框。

### prompt

1. UI 创建 turn id，显示 RunningTurn 状态。
2. turn tracker 在发送请求前注册为 active，避免早到通知丢失。
3. SessionUpdate 同时更新 session metadata、turn tracker，并翻译为 RuntimeEvent。
4. prompt response 与 tracker 一起决定 completed、empty、cancelled 或 failed。
5. 终态后清除 active turn；连接可继续下一轮。

## 兼容与迁移

- 现有 `acp_agent.json` 无需立即修改；缺失 auth/timeouts 使用兼容默认值。
- 现有扩展 command、args、env 行为保持不变。
- `ONETCLI_SKILLS` 和 `ONETCLI_SELECTED_SKILLS` 继续传给 stdio Agent。
- 每轮选中 skill 的文本上下文继续包裹用户 prompt，但实现应确保空 skill context 不产生额外前缀。
- 用户配置文件不存在时，所有现有 ACP Agent 仍按扩展默认值运行。
- HTTP transport 继续由当前 `agent-client-protocol` transport 实现负责；本设计不改变其协议语义。
- 历史 ACP session API、mode 和 config option API 继续保留。连接拆分不得删除已实现的 list/load/resume/close/delete/logout 等能力。

## 安全约束

- 日志、错误卡和测试快照不得包含真实 token、API key、authorization header 或密码。
- 用户覆盖不能改变已安装扩展 command，避免借配置绕过包内可执行文件约束。
- 文件读写 handler 继续要求绝对路径且规范化后位于 workspace root 内。
- 环境变量仅传给所选 ACP 子进程，不写入聊天记录。
- interactive authentication 只能由用户动作触发，不在应用启动或 Agent 列表刷新时自动打开浏览器。
- permission request 继续走现有 ACP approval provider，不因 auth 或 turn 重构而默认放行。

## 测试策略

该任务改变共享 ACP contract、鉴权和状态机，使用 Level 2 TDD，并在完成前执行 Level 3 review 和 Level 4 verification。

### 扩展配置测试

- 旧 manifest 无 auth/timeouts 时使用兼容默认值。
- 新 manifest 正确解析 preferred method、`env_any`、`env_all`、interactive 和 fallback。
- timeout 越界被拒绝。
- command 路径穿越、缺失和不可执行继续被拒绝。

### 用户配置测试

- 文件不存在等价于空覆盖。
- env 引用能从宿主环境解析。
- 缺失 env 引用产生对应 Agent 的诊断。
- 敏感变量字面量被拒绝。
- args、auth method 和 timeout 合并优先级正确。
- 用户覆盖不能修改扩展 command。
- 一个 Agent 配置错误不影响其他 Agent 加载。

### 鉴权测试

- 用户指定 method 优先。
- preferred method 次优先。
- 凭证齐备的非交互 method 可自动选择。
- 只有交互 method 时返回 AuthenticationRequired。
- 未公布的用户 method 返回 UnsupportedAuthMethod。
- 缺少凭证且禁止 fallback 时返回 MissingCredentials。
- authenticate 失败和超时不会继续创建 session。
- 允许 fallback 时无需 authenticate 即可创建 session。

### turn 测试

- 非空 assistant chunk 标记有效输出。
- reasoning、tool call、tool update 和 plan 均标记有效输出。
- metadata、user echo 和空文本不标记有效输出。
- 有效输出加成功 response 产生 TurnCompleted。
- 无输出的 `end_turn` 产生 EmptyResponse。
- prompt error 能提取嵌套 401 和 provider message。
- prompt timeout 发送 cancel 并产生 PromptTimeout。
- 同时提交第二轮返回 TurnAlreadyRunning。
- 连接关闭使 active turn 失败并进入 Failed。

### 生命周期测试

- phase 只允许合法转换。
- drop 先发送 shutdown，再在宽限后 abort。
- 交互鉴权成功后继续创建 session。
- 交互鉴权取消后关闭 transport。
- session 创建失败不能进入 Ready。

### UI contract 测试

- phase 状态卡原位更新，不累积重复消息。
- Ready 前输入框不可提交。
- assistant 输出到达后移除 RunningTurn 状态。
- EmptyResponse 和 auth error 显示 recovery action。
- 切回内置 Agent 后 ACP 状态和 active turn 被清理。

## 验收标准

### 通用

1. 选择任一有效 ACP Agent 后，UI 能准确显示连接阶段并最终进入 Ready。
2. 缺少凭证时不会显示“连接成功”；用户能看到缺少什么以及如何恢复。
3. prompt 无任何有效输出时显示 EmptyResponse，不再留下空白成功轮次。
4. prompt、鉴权和连接错误显示可读摘要、脱敏技术详情和恢复建议。
5. 任一失败或取消路径都不会永久保留“ACP 正在响应…”状态。
6. 多轮 prompt、主动取消、切换 Agent 和关闭 view 不遗留 active turn 或 ACP 子进程。

### Claude

- 使用有效本地登录或配置的环境凭证可以创建 session 并返回非空回答。
- provider 报告下线模型时，UI 显示真实模型错误而非笼统 Internal error。

### Codex

- 使用有效 Codex/ChatGPT 本地登录，或显式映射的 API key，可以创建 session 并返回非空回答。
- 401 错误能显示 HTTP 状态和 provider message，且秘密被脱敏。

### OpenCode

- 已完成 OpenCode 登录时可以返回非空回答。
- 未登录或 provider 未配置导致无输出 `end_turn` 时，UI 显示 EmptyResponse 和登录/配置建议，而不是空白。

### 工程门禁

- 所有新增或修改的 Rust 文件不超过 300 行，函数不超过 50 行，嵌套深度不超过 3。
- 相关 crate 的定向测试、`cargo check` 和 `cargo clippy -- -D warnings` 通过。
- 完成代码审查和 completion verification 后，才可以声称 ACP 已恢复可用。

## 实施顺序

1. 扩展 manifest auth/timeouts contract 与兼容解析。
2. 用户配置解析、环境引用和配置合并。
3. `AcpError`、鉴权决策和 turn tracker 的纯逻辑测试与实现。
4. 拆分连接 lifecycle，并接入 phase、超时和结构化错误。
5. AgentChatView 接入连接阶段、交互鉴权和错误恢复。
6. 使用 fake ACP agent 完成协议级集成测试。
7. 对本机 Claude、Codex、OpenCode 执行人工 smoke test，并记录外部凭证或 provider 限制。
