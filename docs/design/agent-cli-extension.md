# OnetCli Agent CLI Extension Design

## 背景

OnetCli 目前的 `onetcli` 二进制默认启动 GPUI 桌面应用。数据库、SSH、
SFTP、端口转发、数据库驱动和扩展运行时已经存在，但这些能力主要服务于
桌面 UI。为了让 Codex 等 agent 通过 skill 稳定调用本机能力，`onetcli`
需要提供 headless CLI。

这个 CLI 不是简单的应用启动器。它应成为一层稳定的本地能力接口：

- agent 通过命令行发现连接、查询数据库、执行 SSH 命令、读取远端文件。
- skill 只依赖 CLI contract，不直接依赖 Rust 内部 API、数据库驱动细节或
  SSH 密钥路径。
- 第三方能力尽量通过 OnetCli 扩展安装，而不是要求修改主程序。

## 目标

1. 提供 agent-friendly 的 CLI contract：稳定 JSON、稳定 exit code、超时、
   非交互模式和结构化错误。
2. 支持 Codex 这类具备 PTY 能力的 agent 使用交互式 SSH shell。
3. 复用现有连接存储、数据库执行、SSH、SFTP、端口转发和扩展运行时。
4. 允许扩展包向 `onetcli` 安装新命令，而不修改主程序命令解析代码。
5. 将高权限能力纳入权限、审计和 allowlist 管理。
6. 保持 `onetcli` 无参数启动桌面应用的现有行为。

## 非目标

1. 第一阶段不实现完整 MCP server。CLI contract 先作为 skill 的稳定底座。
2. 不让第三方扩展直接读取本地密钥、密码或连接数据库文件。
3. 不让 agent 依赖 UI 自动化来完成数据库、SSH 或文件操作。
4. 不在第一阶段实现所有数据库管理功能；优先支持查询、schema、连接测试。

## 设计原则

1. **核心能力内核化**：数据库、SSH、SFTP、连接读取和审计属于 host 能力。
   这些能力直接关系到凭据和本地安全，不能交给任意扩展进程自由实现。
2. **扩展贡献命令**：扩展可以声明 CLI 命令、参数 schema、权限和 runtime。
   Host 负责安装、发现、权限校验、审计和调用。
3. **默认结构化输出**：agent mode 默认 JSON；human mode 可使用 table/text。
4. **交互与非交互分离**：`ssh exec` 用于确定性调用，`ssh shell` 用于 PTY
   会话。
5. **可组合 contract**：命令输出能被 skill、脚本、CI 和后续 MCP server 复用。

## 总体架构

```text
Codex / Skill / Human
        |
        v
    onetcli binary
        |
        +-- app mode: no args -> launch GPUI app
        |
        +-- cli mode
              |
              +-- core command router
              |     +-- connection
              |     +-- db
              |     +-- ssh
              |     +-- sftp
              |     +-- extension
              |     +-- agent policy
              |
              +-- CLI extension registry
                    |
                    +-- installed composite extensions
                    +-- contributes.cli.commands
                    +-- runtime.ipc / runtime.wasm
```

CLI mode 应尽量运行在不初始化 GPUI 的路径上。核心命令复用 `one-core`、
`db`、`ssh`、`sftp` 等 crate。扩展命令通过扩展运行时调用，但扩展拿到的
是受限 host API，而不是裸露的本地 secret。

## 命令模型

### Core Commands

核心命令由主程序内置，作为 skill 的第一批稳定 contract：

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
    "command": "db.query",
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
    "command": "db.query",
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

## 扩展安装模型

推荐方案：在现有 composite extension 上增加 CLI contribution，而不是第一
阶段新增独立 `cli_extensions` kind。

理由：

1. 现有扩展系统已经支持 `extension.json`、权限、runtime、marketplace、
   安装目录和卸载流程。
2. `extension.json` 已有 `contributes.commands`，可以自然扩展为
   `contributes.cli.commands`。
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

扩展包通过现有 extension marketplace 安装。CLI mode 和 GUI mode 都从
`ExtensionRegistry` 读取 composite extensions。

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
    "cli:commands:contribute",
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
    "cli": {
      "commands": [
        {
          "id": "example.inspect",
          "name": "inspect",
          "path": ["example", "inspect"],
          "summary": "Inspect a saved connection with extension logic.",
          "handler": {
            "kind": "ipc",
            "runtime_id": "tools",
            "method": "cli/execute"
          },
          "args_schema": {
            "type": "object",
            "properties": {
              "connection": { "type": "string" },
              "format": { "type": "string", "enum": ["json", "text"] }
            },
            "required": ["connection"]
          }
        }
      ]
    },
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

当前 `ContributesManifest` 中没有 `cli` 和 `skills` 字段。实现时需要新增
结构化字段，并保持未知字段向后兼容。

### CLI Command Path

扩展命令被挂载到：

```bash
onetcli ext <extension-id> <command>
```

同时可以声明短路径：

```bash
onetcli example inspect prod --format json
```

短路径有冲突风险，因此规则如下：

1. 内置命令优先级最高。
2. 扩展短路径不能覆盖内置命令。
3. 多个扩展声明同一短路径时，默认禁用该短路径，只允许完整路径：

```bash
onetcli ext com.example.onetcli-tools inspect prod
```

4. 用户可以在本地 policy 中显式指定短路径归属。

## 扩展运行时调用

扩展 CLI 命令不直接继承用户 shell 权限。Host 调用扩展 runtime，并传入结构化
request：

```json
{
  "command_id": "example.inspect",
  "argv": ["prod"],
  "options": {
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
    "command_id": "example.inspect"
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
cli:commands:contribute
cli:interactive
cli:process:spawn

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
2. **命令执行时**：检查扩展权限、agent policy、连接 allowlist 和命令参数。

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
onetcli agent allow command db.query
onetcli agent allow command ssh.exec --connection prod-web
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
      "allowed_commands": ["db.schema", "db.query"],
      "readonly": true
    },
    "prod-web": {
      "agent_enabled": true,
      "allowed_commands": ["ssh.exec", "ssh.shell"],
      "interactive": true
    }
  },
  "extensions": {
    "com.example.onetcli-tools": {
      "enabled": true,
      "allowed_commands": ["example.inspect"]
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

Skill 内容应依赖稳定 CLI contract：

```text
1. 调用 `onetcli connection list --type database --format json` 发现连接。
2. 调用 `onetcli db schema <connection> --format json` 获取结构。
3. 只读问题调用 `onetcli db query <connection> --sql ... --readonly --format json`。
4. 需要连续远端操作时调用 `onetcli ssh shell <connection>`。
5. 写操作必须先向用户确认，再使用带 `--write` 的命令。
```

这样 extension marketplace 可以分发能力和 skill，但 agent 端仍依赖统一
`onetcli` 命令。

## 审计

所有 agent mode 命令写入审计日志：

```text
timestamp
actor: human | agent
command: db.query | ssh.exec | extension command id
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
  -> CLI entry and command router

one-core
  -> storage, connection repository, key storage, agent policy

db
  -> DbManager, DbConnection, SqlResult, IPC driver integration

ssh
  -> RusshClient, auth, shell, exec, port forward, socks

sftp
  -> remote file operations

extension-runtime
  -> extension registry, marketplace, composite manifest, CLI contributions

extension-host / extension-protocol
  -> process runtime transport for extension CLI handlers
```

需要新增的内部 crate 可以命名为：

```text
crates/cli
```

它负责：

1. command parser 和 dispatch。
2. unified output envelope。
3. extension CLI registry。
4. agent policy enforcement。
5. audit writer。

`main/src/main.rs` 只做最薄分流：

```text
if cli::should_handle_cli_args(args) {
    cli::run(args).await;
    return;
}

launch_gpui_app();
```

## Windows 二进制策略

当前主程序使用 `windows_subsystem = "windows"` 隐藏 release 控制台。这和 CLI，
尤其是 `ssh shell` 冲突。

推荐演进：

1. 第一阶段在 macOS/Linux 上实现并验证 CLI。
2. Windows 上拆分二进制：

```text
onetcli      -> console subsystem, CLI first, no args may launch GUI
onetcli-gui  -> windows subsystem, GUI launcher
```

3. 安装包中保留 `onetcli` 命令供 agent 调用。

## MVP

第一阶段实现：

```bash
onetcli connection list --format json
onetcli connection show <connection> --format json
onetcli db schema <connection> --format json
onetcli db query <connection> --sql "select 1" --readonly --format json
onetcli ssh exec <connection> --command "uptime" --format json --timeout 10s
onetcli ssh shell <connection>
```

同时定义但可延后实现：

```bash
onetcli extension cli list --format json
onetcli agent policy show --format json
```

MVP 验收标准：

1. `onetcli` 无参数仍启动桌面应用。
2. CLI 命令不初始化 GPUI。
3. 数据库和 SSH 命令可以复用已保存连接。
4. JSON 输出和错误 envelope 稳定。
5. `ssh shell` 可在真实 TTY 下交互，并在退出后恢复终端状态。
6. agent mode 默认不执行写操作。

## 扩展化阶段

第二阶段实现：

1. `ContributesManifest` 增加 `cli` 和 `skills` 字段。
2. `ExtensionRuntimeCatalog` 加载 installed composite extensions 的 CLI
   contributions。
3. `onetcli extension cli list` 展示扩展命令。
4. `onetcli ext <extension-id> <command>` 调用扩展 handler。
5. marketplace 安装包支持携带 CLI contribution 和 skill 文件。

第三阶段实现：

1. agent policy UI 和 CLI。
2. 扩展短路径冲突解析。
3. transcript 和审计查询。
4. MCP server 复用同一套 `crates/cli` service contract。

## 方案取舍

### 方案 A：所有 CLI 都内置

优点：实现最快，安全边界清晰。

缺点：扩展能力差，每个新命令都要改主程序。

### 方案 B：新增独立 CLI extension kind

优点：语义清晰，安装目录独立。

缺点：会复制 composite extension 的安装、权限、marketplace 和 runtime 逻辑。

### 方案 C：Composite extension 增加 CLI contribution

优点：复用现有扩展体系，扩展可同时贡献 UI、命令、skill、runtime。主程序只需要
加载 contribution registry。

缺点：manifest schema 需要扩展，catalog 需要识别 CLI contribution。

推荐方案是 C。核心数据库、SSH、SFTP 命令先内置，第三方和高级能力通过
composite extension 贡献 CLI 命令安装。

## 开放问题

1. 第一版 agent policy 是连接标签还是独立 policy 文件。
2. `ssh shell` 是否默认允许 agent 使用，还是需要用户显式 allow。
3. 扩展携带的 skill 是否由 OnetCli 自动安装到 Codex，还是只导出给用户安装。
4. 扩展 runtime 的 CLI handler 优先支持 IPC 还是 Wasm component。
5. Windows 是否第一阶段就拆分 `onetcli` 和 `onetcli-gui`。

## 建议决策

1. 采用 composite extension CLI contribution，不新增独立扩展 kind。
2. 第一阶段先实现核心内置命令，保证 skill 能马上调用数据库和 SSH。
3. `ssh exec` 作为 skill 默认入口，`ssh shell` 作为 Codex/human 的 PTY 入口。
4. Agent mode 下默认只读，写操作和交互式 shell 由 policy 显式放开。
5. 所有输出先稳定 JSON envelope，再补 table/text。
