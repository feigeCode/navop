# OnetCli CLI / Automation Runtime 架构

> 最后更新: 2026-06-24 | commit: `<当前 HEAD>`

## 概述

`onetcli` 当前命令体系由三层组成：

1. **命令解析层** — `crates/onetcli_cli`
2. **headless runtime** — `crates/onetcli_runtime`
3. **GUI 主程序** — `main`

CLI、MCP、function calling 都复用同一个 `tool_runtime::ToolRegistry`，不分别实现业务逻辑。

## 架构图

```
onetcli (main binary)
├── 无参数 → GPUI 桌面应用
├── update 命令 → 更新处理
└── 其他命令 → crates/onetcli_runtime (headless, 不初始化 GPUI)
    ├── crates/onetcli_cli::parse_from()  ── 解析命令行
    ├── cli_host::handle_command()        ── 分发到 domain adapter
    │   ├── tool list/schema/call         ── ToolAdapter::FunctionCalling
    │   ├── connection list/show          ── onetcli.connections.*
    │   ├── db schema/query/exec          ── db.* (handler 待实现)
    │   ├── ssh exec/shell/tunnel/socks   ── ssh.* (handler 待实现)
    │   └── sftp list/read               ── sftp.* (handler 待实现)
    └── connections::connection_tool_registry()
        ├── onetcli.connections.list
        ├── onetcli.connections.show
        ├── onetcli.connections.list_kinds
        ├── onetcli.connections.get_schema
        ├── onetcli.connections.validate
        └── connections.save

MCP runtime (main 内)
└── 复用同一个 ToolRegistry
    └── 通过 ToolRuntimeMcpProvider 暴露给外部 MCP 客户端
```

## Crate 职责

### crates/onetcli_cli
纯解析层，只依赖 `clap`。

对外类型：
- `OnetCliCommand` — Tool / Connection / Db / Ssh / Sftp
- `ToolCommand` — List / Schema / Call
- `DbCommand` — Schema / Query / Exec
- `SshCommand` — Exec / Shell / Tunnel / Socks
- `SftpCommand` — List / Read
- `ConnectionCommand` — List / Show
- `OutputFormat` — Json (后续可扩展 Text)
- `parse_from()` — 无命令时返回 `None`（走 GUI）
- `print_error()` — clap 错误打印

### crates/onetcli_runtime
headless runtime，不依赖 GPUI。

依赖：`tool_runtime`, `onetcli_cli`, `one_core`, `serde_json`, `anyhow`, `futures`

公共 API：
- `cli_host::handle_command(make_registry)` — CLI 入口，接收 registry 构造闭包
- `cli_host::run_tool_command(command, registry)` — tool list/schema/call
- `cli_host::domain::run_connection_command(...)` — connection 到 tool 适配
- `cli_host::domain::run_db_command(...)` — db 到 tool 适配
- `cli_host::domain::run_ssh_command(...)` — ssh 到 tool 适配
- `cli_host::domain::run_sftp_command(...)` — sftp 到 tool 适配
- `connections::connection_tool_registry(repo)` — 连接管理 tool registry
- `builtin_tool_registry_with_version(version)` — 内置 tool（app_info 等）
- `tool_registry(repo)` — 合并内置 + 连接管理 registry

## Tool 命名空间

对外 tool id 统一使用产品前缀：

| Tool ID | 说明 | 状态 |
|---|---|---|
| `onetcli.app_info` | 应用元数据 | ✅ 已实现 |
| `onetcli.connections.list` | 列出已保存连接 | ✅ 已实现 |
| `onetcli.connections.show` | 查看单个连接 | ✅ 已实现 |
| `onetcli.connections.list_kinds` | 列出可创建的连接类型 | ✅ 已实现 |
| `onetcli.connections.get_schema` | 获取连接字段 schema | ✅ 已实现 |
| `onetcli.connections.validate` | 验证连接配置 | ✅ 已实现 |
| `connections.save` | 创建或更新连接 | ✅ 已实现 |
| `db.schema` | 数据库 schema | ⏳ handler 待实现 |
| `db.query` | 数据库只读查询 | ⏳ handler 待实现 |
| `db.exec` | 数据库写操作 | ⏳ handler 待实现 |
| `ssh.exec` | SSH 远程命令 | ⏳ handler 待实现 |
| `ssh.shell` | SSH 交互式 shell | ⏳ handler 待实现 |
| `ssh.tunnel` | SSH 端口转发 | ⏳ handler 待实现 |
| `ssh.socks` | SSH SOCKS 代理 | ⏳ handler 待实现 |
| `sftp.list` | SFTP 目录列表 | ⏳ handler 待实现 |
| `sftp.read` | SFTP 文件读取 | ⏳ handler 待实现 |

## CLI 命令面

### 当前可用命令

```bash
# 工具自发现
onetcli tool list --format json                # 列出所有 function-calling tool
onetcli tool schema <tool_id> --format json    # 查看 tool 的 input/output schema
onetcli tool call <tool_id> --input '<json>'   # 直接调用 tool
onetcli tool call <tool_id> '<json>'           # 兼容旧位置参数形式

# 连接管理
onetcli connection list --format json          # 列出已保存连接
onetcli connection show <id-or-name> --format json  # 查看单个连接

# 数据库自动化（handler 待实现）
onetcli db schema <connection> --format json
onetcli db query <connection> --sql "..." --readonly --format json
onetcli db exec <connection> --file ./migration.sql --write --format json

# SSH 自动化（handler 待实现）
onetcli ssh exec <connection> --command "..." --timeout 10s
onetcli ssh shell <connection> --workdir ... --init "..." --transcript ...
onetcli ssh tunnel <connection> --local 15432 --remote 127.0.0.1:5432
onetcli ssh socks <connection> --local 1080

# SFTP 自动化（handler 待实现）
onetcli sftp list <connection> <path> --format json
onetcli sftp read <connection> <path> --max-bytes 65536 --format json
```

## 统一错误格式

所有 runtime/tool 错误都走同一 JSON envelope：

```json
{
  "ok": false,
  "error": {
    "code": "<error_code>",
    "message": "<human-readable message>"
  }
}
```

当前错误码：
- `unknown_tool` — tool id 未注册
- `unsupported_adapter` — tool 不支持当前 adapter
- `tool_failed` — tool 执行失败
- `invalid_json` — 输入 JSON 解析失败
- `write_not_allowed` — mutating tool 未带 `--allow-write`

成功时直接输出 `ToolResult.structured_content`，不包裹 envelope。

## 写操作保护

mutating / destructive tool（`ToolAnnotations::mutating()`）默认拒绝执行。

- 不带 `--allow-write` 时，返回 `write_not_allowed` JSON 错误，exit code 2
- 带 `--allow-write` 时正常执行

## 扩展方式

新增业务 tool（如 `db.query`）的标准流程：

1. 在 `onetcli_runtime` 或新的 domain crate 中实现 `ToolHandler`
2. 注册到 registry 时暴露 adapter: `ToolAdapter::FunctionCalling`
3. CLI 短命令自动可用（domain adapter 已有映射）
4. MCP 和 function calling 也同时可用

不需要修改 `main`、`onetcli_cli` 和 domain adapter。

## 文件结构

```
crates/
├── onetcli_cli/              # 命令行解析 (clap)
│   └── src/
│       ├── lib.rs            # 命令定义、OutputFormat
│       └── tests.rs          # 解析测试
├── onetcli_runtime/          # headless runtime
│   └── src/
│       ├── lib.rs            # builtin_tool_registry, tool_registry
│       ├── cli_host.rs       # CLI host: 分发、输出、错误
│       ├── cli_host/
│       │   ├── domain.rs     # connection/db/ssh/sftp → tool 适配
│       │   └── tests.rs      # CLI host 测试
│       └── connections/      # onetcli.connections.* tools
│           ├── connections.rs
│           ├── build.rs
│           ├── schema.rs
│           ├── validation.rs
│           ├── input.rs
│           ├── extended_build.rs
│           └── tests/
├── tool_runtime/             # 统一 tool registry 抽象
│   └── src/
│       └── lib.rs            # ToolHandler, ToolRegistry, ToolAdapter
└── main/                     # GUI 主程序
    └── src/
        ├── main.rs           # 入口: update → CLI → GPUI
        └── public_mcp_runtime/
            ├── public_mcp_runtime.rs  # 装配 CLI registry
            └── tool_registry.rs      # MCP tool provider 装配
```
