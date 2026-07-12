# OnetCli 到 Navop 迁移审计

## 结论

仓库已经完成主要产品外观和发布链路迁移：主二进制、应用显示名、图标、各平台安装包、GitHub Release、更新资产、README 和主要应用内文案均已切换到 Navop。

全仓仍有约 885 处、分布在约 155 个文件中的 `onetcli`、`OnetCli` 或 `ONETCLI_*` 标识。这些命中不能视为同一类漏改，其中混合了用户可见品牌、公开配置、持久化身份、协议字段、扩展生态 ID、升级兼容入口和纯内部代码名。迁移必须按类别处理，禁止直接全局替换。

## 已完成的迁移

- 主二进制为 `navop`，位于 `main/Cargo.toml`。
- macOS 显示名、App 名和图标为 Navop。
- Linux Desktop Entry 使用 `Exec=navop`。
- Windows MSI 产品名、安装目录和快捷方式为 Navop。
- Release 产物使用 `navop-*` / `navop_*` 命名。
- GitHub 更新源已切换到 `feigeCode/navop`。
- README、AI system prompt、LLM Provider 显示名和主要 MCP 描述已使用 Navop。
- 更新器同时接受 `Navop.app`、历史 `OnetCli.app`、`navop.exe`、`onetcli.exe` 和 Linux 对应二进制，保留了旧版本升级能力。

## 应直接修改的项目

### 高优先级

1. 内嵌业务 CLI
   - 审计时现状：`navop` GUI 二进制内嵌 `tool`、`connection`、`db`、`ssh`、`sftp` 命令解析和进程内 tool adapter。
   - 问题：CLI 与 GUI 生命周期、应用内部状态和 runtime crate 直接耦合，不利于形成稳定的独立自动化边界。
   - 处理结果：内嵌 CLI crate、CLI host 和 CLI adapter 已删除；未来独立 `navop-cli` 应通过 Public MCP discovery、token 和工具接口连接正在运行的 Navop。

2. License 购买链接
   - 现状：`main/src/license.rs` 仍打开 `https://onetcli.app/pricing`。
   - 影响：升级入口可能跳转到旧站点或错误商品。
   - 做法：确认 Navop 正式购买地址后修改；地址未确认前不猜测替换。

3. 开发上下文文档
   - 现状：`CLAUDE.md` 仍将项目介绍为 OnetCli。
   - 影响：后续开发代理可能重新生成旧品牌文案或误判产品身份。
   - 做法：产品介绍改为 Navop；`OnetCliApp`、`onetcli_app.rs` 等真实内部标识按事实保留并标注为历史命名。

4. 扩展兼容错误文案
   - 现状：错误提示要求用户“升级 onetcli”。
   - 影响：用户在 Navop 中看到旧产品名。
   - 做法：人类可读产品名称改为 Navop；协议字段 `engines.onetcli` 原样保留。

5. 公开环境变量
   - 现状：更新、Public Base URL、Team Management、Public MCP 等只暴露 `ONETCLI_*`。
   - 影响：Navop 的部署和文档长期依赖旧品牌配置面。
   - 做法：新增 `NAVOP_*` 并优先读取，旧 `ONETCLI_*` 作为兼容 fallback；CI 和新文档改用新变量。

6. MCP 客户端配置名称
   - 现状：生成的 Claude/Codex MCP Server key 仍是 `onetcli`。
   - 影响：应用 UI 说 Navop，但实际配置显示旧名称。
   - 做法：新安装使用 `navop`，检查、修复和卸载同时识别旧 `onetcli`；helper 物理 ID 暂时保留。

### 中低优先级

- HTTP User-Agent 从 `onetcli` / `onetcli-updater` 迁移为 Navop 品牌；先确认服务端没有旧 UA 白名单。
- 新建 SQLite/DuckDB 的默认建议文件名改为 `navop_default.db` / `navop_default.duckdb`；不改已有连接路径。
- 新日志和更新临时目录使用 `navop.log` / `navop-update`；清理逻辑兼容旧临时目录。
- 清理 `.gitignore` 中已过时的 `resources/macos/OnetCli.icns`。
- 普通注释、测试临时名和非协议内部字符串可在独立清理中逐步更新。

## 不能直接修改的项目

以下项目不是永远不能迁移，而是不能通过搜索替换完成；必须提供双读、alias、版本协商或一次性数据迁移。

### macOS Bundle ID

- 当前值：`com.onetcli.app`。
- 风险：影响系统应用身份、钥匙串、TCC 权限、通知、偏好、LaunchServices、签名和覆盖升级。
- 建议：若 Navop 是直接延续，可永久保留；若要成为独立应用，需设计完整的应用身份和用户数据迁移。

### `one-hub` 数据目录

- 覆盖配置数据库、设置、认证、License、加密密钥、扩展和 MCP helper 路径。
- 直接改名会表现为连接、设置、登录、License 或密钥全部丢失。
- 建议：新目录优先、旧目录探测、原子迁移、失败回退，并保证加密 salt、verification magic 和固定 key 不变。

### LLM Provider 历史 ID

- `ProviderType::OnetCli` 和序列化值 `onet_cli` 已写入历史配置。
- 显示名已经是 Navop，这是正确的兼容方式。
- 若以后重命名 Rust enum，反序列化仍需长期接受 `onet_cli`，并避免创建重复内置 Provider。

### 个人同步格式

- `.onetcli-sync` 和 manifest `app: onetcli` 是跨设备、跨 Git 仓库的持久化格式。
- 直接改名会让已有同步仓库看起来为空。
- 建议：永久保留格式名，或让 `.navop-sync` 与 `.onetcli-sync` 双读并定义冲突规则。

### 扩展 manifest 与扩展 ID

- `engines.onetcli` 已由现有扩展声明。
- `com.onetcli.*` 已用于安装、升级、启用状态、导入器引用和缓存。
- 建议：未来新增 `engines.navop` 时优先读取新字段、回退旧字段；扩展 ID 只能通过 alias/migration map 演进。

### 扩展仓库和 MCP helper

- `onetcli-extensions` URL 只有在新仓库和 Release 镜像准备好后才能迁移，并应保留旧 URL fallback。
- `onetcli-public-mcp` 同时是市场 ID、目录名、二进制和 artifact 名；需要新旧 helper 双识别和修复逻辑。

### Public MCP 与工具 ID

- `onetcli.app_info`、`onetcli.runtime_status`、`onetcli.connections.*` 等可能已被脚本和 MCP 客户端调用。
- 建议：新增 `navop.*` alias，新文档使用新 ID，旧 ID 经正式弃用周期后再决定是否移除。

### 扩展协议与 IPC wire

- 包括 `kss://onetcli`、`sht://onetcli`、`/*onetcli-ipc-wire*/`、`ONETCLI_EXT_SOCKET`。
- 宿主、扩展、驱动和独立进程必须同步兼容。
- 建议：parser 双读、环境变量双注入、协议协商后输出，不能单端替换。

### 远端 Shell integration

- 包括 `~/.config/onetcli`、`~/.onetcli-monitor`、`_ONETCLI_*` 和 `__ONETCLI_*` marker。
- 这些内容已经部署到用户远端主机。
- 建议：新安装使用新命名时，检测、升级和卸载必须同时覆盖旧脚本、rc block 和 marker。

### 更新和系统包兼容

- 更新器继续识别 `OnetCli.app`、`onetcli.exe` 和 Linux `onetcli`。
- Debian/RPM 的 `Provides`、`Replaces`、`Conflicts`、`Obsoletes: onetcli` 用于包管理升级。
- 这些是正确的历史升级入口，应继续保留。

### Git 上游 remote

- `origin` 指向 Navop，`onetcli-upstream` 用于同步旧上游。
- 只要仓库仍需要合并 OnetCli 上游变化，就不应为了品牌清理而删除或改名。

## 可稍后整理的内部命名

以下内容不会直接影响用户，可在公开迁移完成后以独立重构处理：

- `crates/onetcli_cli`、`crates/onetcli_runtime`；
- `main/src/onetcli_app.rs`；
- `OnetCliApp`、`GlobalOnetCliApp`、`OnetCliCommand`、`OnetCliLLMProvider`；
- 测试数据库、临时目录、thread name、socket 临时前缀；
- 不进入 UI、不属于公开协议的内部注释和局部变量。

如果 Navop 仍会持续合并 `onetcli-upstream`，暂时保留这些内部名称可以显著减少同步冲突。

## 推荐迁移顺序

1. 修复明确的用户可见品牌：CLI help、错误文案、开发文档和过时 ignore 规则。
2. 为 `NAVOP_*` 环境变量建立新优先、旧 fallback 的双栈配置。
3. 迁移 MCP Server key，并为 `navop.*` 工具建立旧 ID alias。
4. 协调扩展仓库、`engines.navop` 和 helper artifact 的生态迁移。
5. 单独设计 `one-hub`、同步格式、远端 shell 和 Bundle ID 的持久化迁移。
6. 最后整理 Rust crate、module、type 和测试临时命名。

## 第一批安全修复范围

本次只实施以下内容：

- `navop --help` 使用 Navop 命令名；
- 扩展兼容错误使用 Navop 产品名；
- `CLAUDE.md` 正确介绍当前产品；
- 删除 `.gitignore` 中旧 `OnetCli.icns` 条目。

本次不触碰本文列出的任何兼容敏感标识，也不修改工作区中用户已有改动的文件。

## 第一批执行结果

2026-07-12 已完成本批安全修复：

- `navop` GUI 二进制的内嵌业务 CLI 已完整移除，不再解析数据库、SSH、SFTP 或 tool 子命令；
- `onetcli_runtime` 的业务 tool registry 和 Public MCP 集成继续保留，作为未来独立 `navop-cli` 的调用边界；
- 扩展 schema 过新、缺少 `engines.onetcli` 和宿主版本不匹配错误已使用 Navop 产品名；
- `CLAUDE.md` 已将当前产品介绍改为 Navop，并明确内部历史标识的兼容性质；
- `.gitignore` 已删除过时的 `resources/macos/OnetCli.icns` 条目；
- `engines.onetcli` 字段、Bundle ID、同步格式、Provider ID、tool ID 和旧更新入口等兼容标识保持不变。
