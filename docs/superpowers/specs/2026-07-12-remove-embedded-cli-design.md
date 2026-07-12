# 移除桌面应用内嵌 CLI 设计

## 目标

让 `navop` 二进制只承担桌面 GUI 和自更新职责，完整删除当前与 GUI 进程耦合的内嵌业务 CLI。未来独立的 `navop-cli` 通过 Public MCP discovery、认证 token 和稳定工具接口连接正在运行的 Navop，不直接链接桌面应用内部状态。

## 当前边界

当前 CLI 分为三层：

- `crates/onetcli_cli` 使用 Clap 定义 `tool`、`connection`、`db`、`ssh`、`sftp` 命令及参数；
- `onetcli_runtime::cli_host` 把 CLI command 转换为 `ToolRegistry` 调用；
- `main/src/main.rs` 在 GUI 初始化前调用 `handle_cli_command()`，使命令行模式和桌面应用共享同一个二进制。

`onetcli_runtime` 的其余模块并不是 CLI 专属代码。connection、database、redis、sftp、workspace 和 app-info registry 正被 Public MCP、Agent runtime 和应用内部工具注册使用，必须保留。

## 删除范围

本次删除：

- workspace member `crates/onetcli_cli`；
- workspace dependency `onetcli_cli`；
- `crates/onetcli_cli` 的源码、测试和 Cargo manifest；
- `onetcli_runtime` 对 `onetcli_cli` 的依赖；
- `onetcli_runtime::cli_host`、`cli_host/domain.rs`、`cli_host/tests.rs`；
- `main/src/main.rs` 的业务 CLI 分流调用和平台条件函数；
- `tool_runtime::ToolAdapter::Cli`；
- `ToolOrigin::Cli`、`ToolCaller::Cli` 和 `ResourceOrigin::Cli` 等没有其他调用方的 CLI 来源标识；
- tool descriptor、resource、invocation 和测试中的 `Cli` adapter 分支；
- 仅通过 `cli_host::run_tool_command` 测试工具行为的测试适配层。

当前 `update::handle_update_command()` 属于应用自更新安装协议，不属于业务 CLI，本次继续保留。

## 保留范围

本次保留：

- `crates/onetcli_runtime` crate 及其名称；
- app-info、connection、database、redis、sftp、workspace tool registry；
- `ToolAdapter::Mcp` 和 `ToolAdapter::FunctionCalling`；
- Public MCP runtime、discovery 文件、token 校验、权限和 stdio helper；
- Agent runtime 和应用内部通过工具注册表调用业务能力的路径；
- `onetcli.*` 历史 tool id；
- GUI 启动、更新检查和自更新命令。

本次不创建独立 `navop-cli` 二进制，也不重命名 `onetcli_runtime`。这两个任务应在后续独立规格中处理。

## 工具测试迁移

现有 `crates/onetcli_runtime/tests/redis_tools.rs` 通过 `cli_host::run_tool_command` 间接调用 Redis tools。删除 CLI host 时不能简单丢失这些行为保护。

测试将改为直接使用 `ToolRegistry::call()`：

- 选择工具使用其仍支持的 `ToolAdapter::Mcp` 或 `FunctionCalling`；
- 构造对应 `ToolContext`；
- 直接传入 JSON input；
- 继续断言无活动 Redis session、未知 session、只读/写入行为和错误 contract。

其他以 `ToolAdapter::Cli` 验证 registry exposure 的测试将改为使用仍存在的 adapter，并保持原有业务断言。只验证已删除 CLI adapter 自身的测试将删除。

## 未来独立 CLI 架构

未来 `navop-cli` 采用与 MCP helper 相同的进程边界：

```text
navop-cli
    │
    ├─ 读取 Public MCP discovery
    ├─ 使用 loopback 地址和 token 连接
    ├─ 调用 MCP tool / internal_functions.call
    └─ 将结构化结果渲染为 CLI 输出

Navop GUI
    │
    └─ Public MCP runtime → tool registries → application state
```

独立 CLI 不应依赖 `main` crate，也不应直接打开 Navop 的本地数据库来绕过正在运行的应用状态和权限模型。

## 测试策略

本次属于公开行为删除和共享 adapter contract 变更，使用 TDD/回归测试保护：

1. 先增加结构回归测试，要求 GUI 主入口不再调用或定义 `handle_cli_command`，并要求 runtime 不再导出 `cli_host`。
2. 运行红测，确认当前代码因内嵌 CLI 仍存在而失败。
3. 删除 CLI crate、host 和 GUI 分流入口。
4. 将 Redis/runtime 测试迁移为直接 registry 调用。
5. 删除 `ToolAdapter::Cli` 并修复所有穷尽匹配和 descriptor adapter 列表。
6. 运行 `tool_runtime`、`onetcli_runtime`、`public_mcp` 和 `main` 的定向测试/check。
7. 全仓搜索确认不存在 `onetcli_cli`、`cli_host` 或 `ToolAdapter::Cli`。
8. 搜索确认 `onetcli_runtime` registry 和 Public MCP 调用仍存在。

## 兼容性与风险

- `navop tool ...`、`navop db ...` 等当前内嵌业务命令将不再可用，这是本次明确接受的行为删除。
- GUI 自更新命令继续工作。
- Public MCP 对外能力保持，未来独立 CLI 可复用。
- 删除 `ToolAdapter::Cli`、`ToolOrigin::Cli`、`ToolCaller::Cli` 和 `ResourceOrigin::Cli` 会改变 `tool_runtime` 公共 Rust API；仓库内所有调用方必须在同一批次完成迁移。
- 不删除 tool id、registry 或 MCP adapter，避免把“删除 CLI”扩大为“删除自动化能力”。
- 当前工作区中的 `main/src/setting_tab.rs` 用户修改不在本次范围内，实施时不得覆盖。

## 验收标准

- workspace 中不存在 `crates/onetcli_cli` member、dependency 或目录；
- `onetcli_runtime` 不再依赖或导出 CLI host；
- `navop` 启动路径不再解析业务 CLI 参数；
- `ToolAdapter::Cli`、其他 CLI 来源枚举值及其专属分支全部删除；
- Redis 等原有工具行为仍由直接 registry 测试保护；
- `onetcli_runtime` 的业务 tool registries 和 Public MCP 集成继续编译并通过测试；
- `update::handle_update_command()` 保持不变；
- 全仓定向搜索没有 CLI 遗留；
- 用户已有的 `main/src/setting_tab.rs` 修改保持不变；
- 不自动创建 commit、push 或 PR。
