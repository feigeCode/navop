# OnetCli Agent Tool Runtime, CLI, Function Calling And MCP Design

## 背景

OnetCli 目前的 `onetcli` 二进制默认启动 GPUI 桌面应用。数据库、SSH、
SFTP、端口转发、数据库驱动和扩展运行时已经存在，但这些能力主要服务于
桌面 UI。为了让 Codex 等 agent 通过 skill、function calling、MCP 或 CLI
稳定调用本机能力，`onetcli` 需要提供主包内置的 headless tool runtime。

这个能力层不是简单的应用启动器。它应成为一层稳定的本地 tool 接口：

- agent 通过 CLI、function calling 或 MCP 发现连接、查询数据库、执行 SSH
  命令、读取远端文件。
- skill、function calling adapter 和 MCP server 只依赖 tool contract，不
  直接依赖 Rust 内部 API、数据库驱动细节或 SSH 密钥路径。
- Tool runtime、CLI host 和核心高权限工具随主包发布，第三方和高级能力通过 OnetCli
  扩展安装，避免每个新工具都修改主程序。

## 目标

1. 提供 agent-friendly 的 tool contract：稳定 schema、稳定 JSON、稳定
   exit code、超时、非交互模式和结构化错误。
2. 支持 Codex 这类具备 PTY 能力的 agent 使用交互式 SSH shell。
3. 复用现有连接存储、数据库执行、SSH、SFTP、端口转发和扩展运行时。
4. 将 Tool Runtime、CLI host 和核心工具放入主包，让 CLI、function calling
   和 MCP server 复用同一套能力实现。
5. 将高权限能力纳入权限、审计和 allowlist 管理。
6. 保持 `onetcli` 无参数启动桌面应用的现有行为。
7. 允许扩展包向 runtime 贡献增量 tools，再选择暴露到 CLI、MCP 或 function
   calling，而不修改主程序命令解析代码。

## 非目标

1. 第一阶段不要求完整产品化 MCP server。Tool Runtime 和 CLI 先作为 skill
   的稳定底座，已有 `public_mcp` 可以作为 MCP adapter 原型继续演进。
2. 不让第三方扩展直接读取本地密钥、密码或连接数据库文件。
3. 不让 agent 依赖 UI 自动化来完成数据库、SSH 或文件操作。
4. 不在第一阶段实现所有数据库管理功能；优先支持查询、schema、连接测试。

## 设计原则

1. **核心能力内核化**：数据库、SSH、SFTP、连接读取和审计属于 host 能力。
   这些能力直接关系到凭据和本地安全，不能交给任意扩展进程自由实现。
2. **主包提供 Tool Runtime**：tool registry、tool descriptor、schema、
   policy、approval、audit、核心数据库/SSH/SFTP 工具随 `onetcli` 主包发布。
3. **Adapter 不拥有业务能力**：CLI、function calling、MCP server 只负责协议
   和格式转换，最终都调用同一个 `ToolRegistry`。
4. **扩展贡献增量工具**：扩展可以声明 tool、参数 schema、权限、adapter 暴露
   方式和 runtime。Host 负责安装、发现、权限校验、审计和调用。
5. **默认结构化输出**：agent mode 默认 JSON；human mode 可使用 table/text。
6. **交互与非交互分离**：`ssh exec` 用于确定性调用，`ssh shell` 用于 PTY
   会话。
7. **可组合 contract**：tool 输出能被 skill、脚本、CI、function calling 和
   MCP server 复用。

## 总体架构

```text
Codex / Skill / Human
        |
        v
    onetcli binary
        |
        +-- app mode: no args -> launch GPUI app
        |
        +-- tool runtime
              |
              +-- builtin tool registry
              |     +-- connection.list / connection.show / connection.test
              |     +-- db.query / db.schema / db.exec
              |     +-- ssh.exec / ssh.shell / ssh.tunnel / ssh.socks
              |     +-- sftp.list / sftp.read
              |     +-- agent.policy / audit
              |
              +-- extension tool registry
              |     +-- installed composite extensions
              |     +-- contributes.tools
              |     +-- runtime.ipc / runtime.wasm
              |
              +-- adapters
                    +-- CLI adapter
                    +-- function calling adapter
                    +-- MCP adapter / public_mcp
```

Headless adapter 应尽量运行在不初始化 GPUI 的路径上。Tool Runtime 与核心
工具由主包内置，复用 `one-core`、`db`、`ssh`、`sftp` 等 crate。扩展工具
通过扩展运行时调用，但扩展拿到的是受限 host API，而不是裸露的本地 secret。

## Tool Runtime 统一内核

核心抽象应先于 CLI/MCP/function calling 存在。建议新增内部 crate：

```text
crates/tool_runtime
```

或命名为：

```text
crates/capabilities
```

本文后续用 `tool_runtime` 表示这层。

### Tool Descriptor

每个内置或扩展工具提供统一描述：

```rust
pub struct ToolDescriptor {
    pub id: String,
    pub title: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub permissions: Vec<ToolPermission>,
    pub mode: ToolMode,
    pub adapters: Vec<ToolAdapter>,
    pub cli: Option<CliToolMetadata>,
}
```

工具模式用于 adapter 能力过滤：

```rust
pub enum ToolMode {
    Deterministic,
    Interactive,
    LongRunning,
    Streaming,
}
```

Adapter 声明用于决定同一工具暴露到哪里：

```rust
pub enum ToolAdapter {
    Cli,
    FunctionCalling,
    Mcp,
    Gui,
}
```

例如：

```text
db.query:
  mode = Deterministic
  adapters = Cli, FunctionCalling, Mcp

ssh.exec:
  mode = Deterministic
  adapters = Cli, FunctionCalling, Mcp

ssh.shell:
  mode = Interactive
  adapters = Cli

ssh.session.create/read/write/close:
  mode = Streaming or LongRunning
  adapters = Mcp, FunctionCalling
```

### Tool Handler

业务能力通过 handler 实现：

```rust
#[async_trait]
pub trait ToolHandler: Send + Sync {
    fn descriptor(&self) -> ToolDescriptor;

    async fn call(
        &self,
        ctx: ToolContext,
        input: serde_json::Value,
    ) -> Result<ToolResult, ToolError>;
}
```

`ToolContext` 统一携带权限、审批、审计、调用方和取消信息：

```rust
pub struct ToolContext {
    pub actor: ToolActor,
    pub adapter: ToolAdapter,
    pub permission_mode: PermissionMode,
    pub approver: ApprovalManager,
    pub audit: AuditSink,
    pub cancel: CancellationToken,
}
```

### Tool Registry

`ToolRegistry` 是所有入口的唯一分发点：

```rust
pub struct ToolRegistry {
    handlers: BTreeMap<String, Arc<dyn ToolHandler>>,
}
```

要求：

1. 启动时拒绝重复 tool id。
2. `list(adapter)` 根据 adapter 和 policy 过滤工具。
3. `call(id, input, ctx)` 做 schema 校验、permission、approval、audit，再调用
   handler。
4. 内置工具和扩展工具使用同一个 registry。
5. CLI、function calling、MCP adapter 不直接访问数据库、SSH 或连接存储。

### Adapter 映射

CLI：

```bash
onetcli db query prod --sql "select 1" --readonly --format json
```

内部映射为：

```json
{
  "tool": "db.query",
  "input": {
    "connection": "prod",
    "sql": "select 1",
    "readonly": true
  }
}
```

通用调试入口：

```bash
onetcli tool list --format json
onetcli tool schema db.query --format json
onetcli tool call db.query --input '{"connection":"prod","sql":"select 1","readonly":true}'
```

Function calling：

```text
ToolDescriptor -> JSON schema function definition
function call arguments -> ToolRegistry.call()
ToolResult -> function call result
```

MCP：

```text
ToolDescriptor -> rmcp Tool
MCP list_tools -> ToolRegistry.list(Mcp)
MCP call_tool -> ToolRegistry.call()
ToolResult -> CallToolResult::structured
```

不要让 MCP 或 function calling 通过 shell 执行 `onetcli` 子进程。它们应直接复用
`tool_runtime`，避免进程开销、错误结构丢失、权限上下文分裂、取消/超时和审计
重复实现。

## public_mcp 参考实现

`mcp-dev` worktree 中的 `crates/public_mcp` 已经实现了一部分可复用原型：

1. `PublicMcpToolProvider`：provider 暴露 `tools()` 和 `call_tool()`。
2. `PublicMcpToolRegistry`：聚合 provider，拒绝重复 tool name，并按 name
   dispatch。
3. `PublicMcpServer`：通过 `rmcp::ServerHandler` 实现 `list_tools` 和
   `call_tool`。
4. `PermissionMode`、`PublicMcpOperationKind`、`ApprovalManager`：将写终端、
   内部函数调用等高风险操作接入 allow/ask/deny。
5. `LoopbackMcpServer` + discovery file + token handshake + stdio bridge：主应用
   在 loopback 上启动 MCP runtime，外部 stdio helper 通过 discovery 文件连接。
6. `InternalFunctionToolProvider`：用 `name + arguments` 暴露应用内注册函数。
7. `ToolAnnotations`：把 read-only/destructive/idempotent/open-world 语义传给 MCP
   客户端。

这些设计可以保留为 MCP adapter 的基础，但最终边界应调整为：

```text
tool_runtime
  -> owns ToolDescriptor / ToolRegistry / ToolHandler / ToolContext / permission / audit

public_mcp
  -> adapts ToolDescriptor to rmcp::Tool
  -> adapts MCP call_tool to ToolRegistry.call()
  -> keeps loopback runtime, discovery, token handshake, stdio bridge

main
  -> builds tool registry from builtin tools + extension tools + active UI session providers
```

也就是说，`public_mcp` 不应长期拥有唯一的 tool registry。它应该从主包
`tool_runtime` 读取 tool 列表，并只处理 MCP 协议、discovery 和 transport。

当前 `public_mcp` 暴露的工具可以迁移成：

```text
public_mcp.list_sessions        -> terminal.session.list
public_mcp.remote_exec          -> remote.exec
public_mcp.remote_command_poll  -> remote.command.poll
public_mcp.remote_command_output -> remote.command.output
public_mcp.remote_command_cancel -> remote.command.cancel
public_mcp.remote_file_write    -> remote.file.write
public_mcp.session_diagnostics  -> session.diagnostics
```

旧版 `terminal_snapshot`/`terminal_write`（终端粘贴/屏幕读取）已被移除，结构化远程执行工具成为唯一执行通道，不再保留兼容 alias。

## 主包内置策略

第一阶段推荐将 Tool Runtime 和 CLI adapter 放入主包，而不是将 CLI 本身做成扩展。
主包包含：

1. `onetcli` 命令入口和 CLI/GUI app 分流。
2. `crates/tool_runtime` 内部 crate。
3. `crates/cli` adapter crate。
4. 连接发现、数据库查询、SSH exec/shell、SFTP 读取等核心工具。
5. 统一 ToolResult、JSON envelope、exit code、审计和 agent policy。
6. 扩展 tool contribution 的发现与调用入口。

主包不包含：

1. 第三方数据库驱动二进制。
2. 第三方运维工具或业务诊断工具。
3. agent skill 的自动安装产物。
4. 每个扩展工具的具体实现。

这些增量能力继续通过 extension marketplace 安装。

### 体积判断

Tool Runtime 和 CLI host 本身不是体积大头。命令解析、JSON 输出、policy、审计和 dispatch
属于普通 Rust 业务代码，预计对 release 二进制体积影响较小。

真正影响体积的是桌面 UI、数据库客户端、加密/SSH、Wasm runtime、native
库和静态 bundled 驱动。当前主程序已经依赖数据库、SSH、扩展运行时和多种
视图能力，所以把 Tool Runtime 和 CLI host 放入主包通常不会重复引入一份大依赖。

体积控制规则：

1. 不为了 CLI 把第三方数据库驱动改成 builtin。
2. 不把扩展 helper、skill 文件和业务诊断脚本静态编入主二进制。
3. `crates/tool_runtime` 只依赖 service 层，不依赖 GPUI view 层。
4. `crates/cli` 只做 adapter，不实现数据库、SSH、SFTP 业务能力。
5. 新增依赖优先选择轻量库；命令解析可优先评估 `pico-args` 或已有依赖，
   只有需要完整 help/schema 生成时再使用更重 parser。
6. 发布前用 release binary size 做基线比较，Tool Runtime + CLI host 增量应作为验收指标。

建议的体积验收：

```text
1. 记录加入 CLI 前后的 release `onetcli` 大小。
2. 记录压缩安装包大小。
3. 若仅加入 Tool Runtime、CLI host 和核心 dispatch，增量明显异常时检查新增依赖树。
4. 第三方驱动和扩展工具不计入主包体积，应在各自扩展包中统计。
```

## 命令模型

### Core Commands

核心工具由主程序内置，并通过稳定 CLI path 暴露给 skill：

```bash
onetcli connection list --format json
onetcli connection show <connection> --format json
onetcli connection test <connection> --format json

onetcli db schema <connection> --format json
onetcli db query <connection> --sql "select 1" --readonly --format json
onetcli db exec <connection> --file ./migration.sql --write --format json

onetcli ssh exec <connection> --command "uptime" --format json --timeout 10s
onetcli ssh shell <connection>
onetcli ssh tunnel <connection> --local 15432 --remote 127.0.0.1:5432
onetcli ssh socks <connection> --local 1080

onetcli sftp list <connection> /var/log --format json
onetcli sftp read <connection> /var/log/app.log --max-bytes 65536 --format json
```

### Agent Defaults

供 skill 调用时，应默认使用：

```bash
--format json
--no-interactive
--timeout <duration>
```

写操作必须显式声明：

```bash
--write
```

数据库查询默认只读：

```bash
onetcli db query prod --sql "select * from users limit 10" --readonly --format json
```

### Interactive SSH

`ssh shell` 是一等能力，用于人类和 Codex 这类支持交互式 PTY 的 agent：

```bash
onetcli ssh shell prod-web
onetcli ssh shell prod-web --workdir /srv/app
onetcli ssh shell prod-web --init "export TERM=xterm-256color"
```

要求：

1. stdin/stdout 必须是 TTY；否则返回错误并建议使用 `ssh exec`。
2. 分配远端 PTY。
3. 本地终端进入 raw mode，退出时必须恢复。
4. 支持窗口 resize 同步。
5. 默认不输出额外 banner，避免干扰 agent 读取终端内容。
6. 可选 transcript/audit：

```bash
onetcli ssh shell prod-web --transcript ~/.onetcli/audit/sessions/session.log
```

## 输出 Contract

所有 agent-friendly 命令必须支持统一响应 envelope。

成功：

```json
{
  "ok": true,
  "data": {},
  "meta": {
    "tool": "db.query",
    "adapter": "cli",
    "elapsed_ms": 18,
    "connection": "prod",
    "format_version": "1"
  }
}
```

失败：

```json
{
  "ok": false,
  "error": {
    "code": "DB_CONNECTION_FAILED",
    "message": "failed to connect to database",
    "hint": "check host, port, credentials, ssh tunnel, or driver installation"
  },
  "meta": {
    "tool": "db.query",
    "adapter": "cli",
    "elapsed_ms": 1024,
    "connection": "prod",
    "format_version": "1"
  }
}
```

Exit code：

```text
0  success
1  generic error
2  invalid arguments
3  permission denied
4  connection not found
5  timeout
6  remote command failed
7  partial success
```

对于 `ssh exec`，远端命令退出码放在 `data.exit_code` 中。当 SSH 连接成功但
远端命令返回非零时，`onetcli` 应返回 exit code `6`，并在 JSON 中保留
stdout/stderr：

```json
{
  "ok": false,
  "error": {
    "code": "REMOTE_COMMAND_FAILED",
    "message": "remote command exited with code 1"
  },
  "data": {
    "exit_code": 1,
    "stdout": "",
    "stderr": "service not found"
  }
}
```

## 扩展增量安装模型

Tool Runtime、CLI host 和核心工具放在主包。扩展安装只用于增加第三方或高级工具。

推荐方案：在现有 composite extension 上增加 `tools` contribution，而不是新增
独立 `cli_extensions` kind，也不是把 Tool Runtime 或 CLI host 本身做成扩展。

理由：

1. 现有扩展系统已经支持 `extension.json`、权限、runtime、marketplace、
   安装目录和卸载流程。
2. `extension.json` 已有 `contributes.commands`，可以演进为更通用的
   `contributes.tools`，再通过 tool metadata 选择暴露到 CLI、MCP 或
   function calling。
3. 第三方扩展通常不只提供 CLI，也可能提供菜单、UI action、Wasm action 和
   agent 能力描述。Composite extension 更适合作为聚合包。
4. 避免同时维护两套扩展安装、权限、签名和 marketplace 逻辑。

### 安装目录

继续使用现有目录：

```text
<config-dir>/extensions/composite/<extension-id>/
  extension.json
  bin/
    helper
  wasm/
    tool.wasm
  skills/
    onetcli-db/SKILL.md
```

扩展包通过现有 extension marketplace 安装。CLI mode、MCP mode 和 GUI mode 都从
`ExtensionRegistry` 读取 composite extensions。未安装任何扩展时，主包内置
的 `connection`、`db`、`ssh`、`sftp` 工具及 CLI path 仍然可用。

### Manifest 扩展示例

```json
{
  "schema_version": 1,
  "id": "com.example.onetcli-tools",
  "name": "Example OnetCli Tools",
  "version": "0.1.0",
  "engines": {
    "onetcli": ">=0.7.0"
  },
  "permissions": [
    "tools:contribute",
    "cli:tools:expose",
    "mcp:tools:expose",
    "function_calling:tools:expose",
    "db:connections:list",
    "db:query:readonly",
    "ssh:exec"
  ],
  "runtime": {
    "ipc": [
      {
        "id": "tools",
        "entry": {
          "command": "./bin/helper",
          "args": []
        },
        "transport": {
          "kind": "local_socket",
          "connect_timeout_ms": 5000
        }
      }
    ]
  },
  "contributes": {
    "tools": [
      {
        "id": "example.inspect",
        "title": "Inspect",
        "description": "Inspect a saved connection with extension logic.",
        "adapters": ["cli", "mcp", "function_calling"],
        "cli": {
          "path": ["example", "inspect"]
        },
        "handler": {
          "kind": "ipc",
          "runtime_id": "tools",
          "method": "tool/call"
        },
        "input_schema": {
          "type": "object",
          "properties": {
            "connection": { "type": "string" },
            "format": { "type": "string", "enum": ["json", "text"] }
          },
          "required": ["connection"]
        },
        "output_schema": {
          "type": "object"
        },
        "permissions": ["connection:read"]
      }
    ],
    "skills": [
      {
        "id": "onetcli-example",
        "path": "skills/onetcli-example/SKILL.md",
        "description": "Use onetcli example commands from agents."
      }
    ]
  }
}
```

当前 `ContributesManifest` 中没有 `tools` 和 `skills` 字段。实现时需要新增
结构化字段，并保持未知字段向后兼容。

### Adapter 暴露路径

扩展 tool 被挂载到完整 CLI 路径：

```bash
onetcli ext <extension-id> <command>
```

同时可以声明短路径：

```bash
onetcli example inspect prod --format json
```

短路径有冲突风险，因此规则如下：

1. 内置 CLI 命令优先级最高。
2. 扩展短路径不能覆盖内置命令或内置 tool 的 CLI path。
3. 多个扩展声明同一短路径时，默认禁用该短路径，只允许完整路径：

```bash
onetcli ext com.example.onetcli-tools inspect prod
```

4. 用户可以在本地 policy 中显式指定短路径归属。

Function calling 和 MCP 暴露规则：

1. 只有声明对应 adapter 的 tool 才能出现在 function calling 或 MCP tool list。
2. `ToolMode::Interactive` 默认不能暴露到 function calling。
3. `ToolMode::LongRunning` 或 `Streaming` 暴露到 function calling 时必须使用
   session 化工具，例如 `ssh.session.create/read/write/close`。
4. MCP adapter 可以暴露 session 化工具，但不直接暴露裸 UI 内部对象。

## 扩展运行时调用

扩展 tool 不直接继承用户 shell 权限。Host 调用扩展 runtime，并传入结构化
request：

```json
{
  "tool_id": "example.inspect",
  "input": {
    "connection": "prod",
    "format": "json"
  },
  "stdin": null,
  "agent": {
    "mode": true,
    "interactive": false
  },
  "context": {
    "cwd": "/Users/me/project",
    "env_allowlist": ["TERM", "LANG"]
  }
}
```

扩展返回统一 envelope：

```json
{
  "ok": true,
  "data": {},
  "meta": {
    "extension_id": "com.example.onetcli-tools",
    "tool_id": "example.inspect"
  }
}
```

扩展若需要访问数据库、SSH、SFTP，应通过 host API 请求，不直接读取连接存储：

```text
extension runtime
  -> host permission checker
  -> host db/ssh/sftp gateway
  -> existing db/ssh/sftp crates
```

## 权限模型

新增权限建议：

```text
tools:contribute
cli:tools:expose
mcp:tools:expose
function_calling:tools:expose
process:spawn

connection:list
connection:read

db:query:readonly
db:exec:write
db:schema:read
db:export

ssh:exec
ssh:shell
ssh:tunnel
ssh:sftp:read
ssh:sftp:write

agent:skill:contribute
```

权限分两层校验：

1. **扩展安装时**：高风险权限需要用户批准。
2. **Tool 调用时**：检查扩展权限、agent policy、连接 allowlist 和 tool 参数。

默认策略：

1. Agent mode 下默认禁止写操作。
2. Agent mode 下默认禁止未 allowlist 的连接。
3. `ssh shell`、`ssh tunnel`、`db exec --write` 属于高风险能力，需要显式策略。
4. 密码、私钥、token 不进入 JSON 输出、日志或扩展 request。

## Agent Policy

为 agent 调用设计本地 policy：

```bash
onetcli agent policy show --format json
onetcli agent allow connection prod-readonly
onetcli agent deny connection payroll
onetcli agent allow tool db.query
onetcli agent allow tool ssh.exec --connection prod-web
```

策略文件建议放在：

```text
<config-dir>/agent-policy.json
```

示例：

```json
{
  "format_version": 1,
  "connections": {
    "prod-readonly": {
      "agent_enabled": true,
      "allowed_tools": ["db.schema", "db.query"],
      "readonly": true
    },
    "prod-web": {
      "agent_enabled": true,
      "allowed_tools": ["ssh.exec", "ssh.shell"],
      "interactive": true
    }
  },
  "extensions": {
    "com.example.onetcli-tools": {
      "enabled": true,
      "allowed_tools": ["example.inspect"]
    }
  }
}
```

第一阶段也可以先使用连接级 `agent_enabled` 标记，后续再演进到完整 policy。

## Skill 分发模型

扩展可以携带 skill 文件，但不应自动安装到所有 agent。Host 只负责导出或展示
skill 元数据：

```bash
onetcli skill list --format json
onetcli skill export onetcli-db --target codex
```

Skill 内容应依赖稳定 tool contract，并通过 CLI adapter 调用：

```text
1. 调用 `onetcli connection list --type database --format json` 发现连接。
2. 调用 `onetcli db schema <connection> --format json` 获取结构。
3. 只读问题调用 `onetcli db query <connection> --sql ... --readonly --format json`。
4. 需要连续远端操作时调用 `onetcli ssh shell <connection>`。
5. 写操作必须先向用户确认，再使用带 `--write` 的命令。
```

这样 extension marketplace 可以分发能力和 skill，但 agent 端仍依赖统一
tool contract；Codex skill 可以优先用 CLI adapter，其他 agent 可通过 MCP 或
function calling adapter 使用同一工具。

## 审计

所有 agent mode tool 调用写入审计日志：

```text
timestamp
actor: human | agent
tool: db.query | ssh.exec | extension tool id
adapter: cli | mcp | function_calling | gui
connection id/name
readonly/write
interactive: true/false
input summary
exit code
elapsed_ms
```

审计日志不记录：

1. 密码、私钥、token。
2. 完整连接参数。
3. 默认不记录完整 SQL 结果集。

对于 SQL 和 shell command，记录摘要和 hash：

```json
{
  "input_summary": "select * from users limit 10",
  "input_sha256": "..."
}
```

用户可以显式开启 transcript：

```bash
onetcli ssh shell prod-web --transcript <path>
```

## 与现有模块的关系

```text
main
  -> app/tool adapter entry split, GUI launcher

one-core
  -> storage, connection repository, key storage, agent policy

tool_runtime
  -> ToolDescriptor, ToolRegistry, ToolHandler, ToolContext, ToolResult, policy, audit

cli
  -> CLI parser, CLI path mapping, table/json rendering, ToolRegistry adapter

public_mcp
  -> MCP protocol adapter, rmcp transport, loopback runtime, discovery, stdio bridge

function_calling
  -> function schema export, function call adapter, ToolRegistry adapter

db
  -> DbManager, DbConnection, SqlResult, IPC driver integration

ssh
  -> RusshClient, auth, shell, exec, port forward, socks

sftp
  -> remote file operations

extension-runtime
  -> extension registry, marketplace, composite manifest, tool contributions

extension-host / extension-protocol
  -> process runtime transport for extension tool handlers
```

需要新增或调整的内部 crate：

```text
crates/tool_runtime
crates/cli
crates/public_mcp
```

`crates/tool_runtime` 负责：

1. tool descriptor / handler / registry。
2. unified ToolResult / ToolError。
3. tool permission、approval、audit。
4. builtin tool registration。
5. extension tool registration。

`crates/cli` 负责：

1. command parser。
2. CLI path 到 tool id 的映射。
3. table/json 输出。
4. exit code 映射。
5. `onetcli tool list/schema/call` 调试入口。

`crates/public_mcp` 参考 `mcp-dev` worktree 继续演进，但长期只作为 MCP adapter：

1. 将 `ToolDescriptor` 转换为 `rmcp::Tool`。
2. 将 `call_tool` 转发到 `ToolRegistry.call()`。
3. 保留 loopback runtime、discovery file、token handshake 和 stdio bridge。
4. 保留 approval UI 通道，但审批请求应使用统一 `ToolContext`。

`crates/tool_runtime`、`crates/cli` 和 `crates/public_mcp` 随主包编译进 `onetcli`。
`main/src/main.rs` 只做最薄分流：

```text
if cli::should_handle_cli_args(args) {
    cli::run(args).await;
    return;
}

if mcp::should_handle_mcp_args(args) {
    public_mcp::run_stdio_or_server(args).await;
    return;
}

launch_gpui_app();
```

## Windows 二进制策略

当前主程序使用 `windows_subsystem = "windows"` 隐藏 release 控制台。这和 CLI，
尤其是 `ssh shell` 冲突。

推荐演进：

1. 第一阶段保持单主包、单入口设计，在 macOS/Linux 上实现并验证 CLI。
2. Windows 上优先评估能否保留同一主包但调整入口/subsystem 策略。
3. 只有当 Windows console 与 GUI 体验无法兼容时，再拆分二进制：

```text
onetcli      -> console subsystem, CLI first, no args may launch GUI
onetcli-gui  -> windows subsystem, GUI launcher
```

4. 即使拆分二进制，也不改变主包发布策略：安装包中必须保留 `onetcli`
   命令供 agent 调用。

## MVP

第一阶段实现：

```bash
onetcli tool list --format json
onetcli tool schema db.query --format json
onetcli tool call db.query --input '{"connection":"local","sql":"select 1","readonly":true}'

onetcli connection list --format json
onetcli connection show <connection> --format json
onetcli db schema <connection> --format json
onetcli db query <connection> --sql "select 1" --readonly --format json
onetcli ssh exec <connection> --command "uptime" --format json --timeout 10s
onetcli ssh shell <connection>
```

同时定义但可延后实现：

```bash
onetcli mcp serve
onetcli extension tool list --format json
onetcli agent policy show --format json
```

MVP 验收标准：

1. `onetcli` 无参数仍启动桌面应用。
2. CLI tool 调用不初始化 GPUI。
3. `onetcli tool call db.query ...` 和 `onetcli db query ...` 调用同一个 handler。
4. 数据库和 SSH tool 可以复用已保存连接。
5. JSON 输出、ToolResult 和错误 envelope 稳定。
6. `ssh shell` 可在真实 TTY 下交互，并在退出后恢复终端状态。
7. agent mode 默认不执行写操作。
8. Tool Runtime 和 CLI host 随主包发布，未安装扩展时核心工具仍可用。
9. 记录 release 二进制和安装包体积基线，确认 Tool Runtime + CLI host 增量可接受。
10. `public_mcp` 的 registry 迁移路径明确：MCP adapter 能从统一 registry 暴露
    至少一个只读 tool。

## 扩展化阶段

第二阶段实现：

1. `ContributesManifest` 增加 `tools` 和 `skills` 字段。
2. `ExtensionRuntimeCatalog` 加载 installed composite extensions 的 tool
   contributions。
3. `onetcli extension tool list` 展示扩展工具。
4. `onetcli ext <extension-id> <tool>` 调用扩展 tool handler。
5. marketplace 安装包支持携带 tool contribution 和 skill 文件。
6. function calling adapter 从 `ToolDescriptor` 导出函数 schema。

第三阶段实现：

1. agent policy UI 和 CLI。
2. 扩展短路径冲突解析。
3. transcript 和审计查询。
4. MCP server 复用同一套 `crates/tool_runtime` contract。
5. `public_mcp` 旧工具名提供 alias，并逐步迁移到统一 tool id。

## 方案取舍

### 方案 A：所有 CLI、function calling、MCP adapter 和第三方工具都各自实现

优点：局部实现最快。

缺点：数据库、SSH、权限、审计、错误结构和 schema 会重复实现，长期不可维护。

### 方案 B：以 CLI 为核心，MCP/function calling 通过执行 CLI 子进程复用

优点：短期接入成本低，外部 agent 能快速调用。

缺点：多一层进程开销，取消/超时/streaming/approval 难统一，错误结构容易丢失，
内部 function calling 也会被迫走 shell。

### 方案 C：Tool Runtime 放主包，CLI/function calling/MCP 都是 adapter

优点：核心 agent 能力开箱即用；Tool Runtime 体积增量可控；CLI、function
calling、MCP、GUI/workflow 复用同一套能力；扩展可同时贡献 tool、UI、skill、
runtime；第三方能力不需要进入主二进制。

缺点：需要先定义稳定 ToolDescriptor/ToolHandler/ToolResult；`public_mcp`
现有 registry 需要迁移成 adapter。

推荐方案是 C。Tool Runtime、CLI host 和核心数据库、SSH、SFTP 工具随主包发布，
第三方和高级能力通过 composite extension 贡献 tools，并选择暴露到 CLI、
function calling 或 MCP。

## 开放问题

1. 第一版 agent policy 是连接标签还是独立 policy 文件。
2. `ssh shell` 是否默认允许 agent 使用，还是需要用户显式 allow。
3. 扩展携带的 skill 是否由 OnetCli 自动安装到 Codex，还是只导出给用户安装。
4. 扩展 runtime 的 tool handler 优先支持 IPC 还是 Wasm component。
5. Windows 是否需要在后续阶段拆分 `onetcli` 和 `onetcli-gui`，还是通过
   subsystem/launcher 策略保持单主包体验。
6. `public_mcp` 已有工具名是否保留永久 alias，还是设定废弃周期。
7. Function calling adapter 是只给主应用内置 agent 使用，还是也导出给外部 SDK。

## 建议决策

1. Tool Runtime、CLI host 与核心工具放入主包，不把 CLI 本身做成扩展。
2. CLI、function calling、MCP server 都作为 adapter 调用同一个 `ToolRegistry`。
3. `public_mcp` 参考实现继续保留 loopback/discovery/approval/stdio bridge，但
   tool registry 下沉到 `crates/tool_runtime`。
4. 采用 composite extension `tools` contribution，不新增独立扩展 kind。
5. 第一阶段先实现核心内置工具，保证 skill 能马上调用数据库和 SSH。
6. `ssh exec` 作为 skill/function calling/MCP 默认入口，`ssh shell` 作为
   Codex/human 的 CLI PTY 入口。
7. Agent mode 下默认只读，写操作和交互式 shell 由 policy 显式放开。
8. 所有输出先稳定 ToolResult/JSON envelope，再补 CLI table/text。
9. release 前记录体积基线，确保 Tool Runtime + CLI host 增量没有异常依赖引入。
