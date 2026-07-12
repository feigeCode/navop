# 外部编辑器通用自动保存与自动上传设计

> **已被用户修订：** 不修改任何外部编辑器或扩展行为，不生成编辑器工作区配置。当前实现规格以 `2026-07-11-external-file-monitoring-auto-upload-design.md` 为准。

## 背景

OnetCli 的 SFTP 外部编辑器链路已经能够监听本地临时文件的写盘事件，并在 750ms 防抖后把内容上传到远端。该机制只能观察磁盘文件，无法观察 Zed 尚未保存的内存 buffer。

外部编辑器的内存 buffer 归编辑器进程所有，OnetCli 不能直接替第三方编辑器执行保存。完整的通用方案必须由编辑器扩展声明该编辑器可靠支持的 autosave 准备方式，再由 Host 在隔离的临时会话中执行。Zed 支持工作区级 `.zed/settings.json`，可通过以下配置在停止输入 1 秒后自动写盘：

```json
{
  "autosave": {
    "after_delay": {
      "milliseconds": 1000
    }
  }
}
```

同时，用户需要一个 OnetCli 配置项决定外部编辑器保存本地临时文件后是否自动上传。该开关应适用于 Zed、Notepad-- 和后续所有外部编辑器，而不是绑定到某个编辑器扩展。

## 目标

- 建立适用于所有外部编辑器扩展的 autosave/workspace 准备 contract，而不是在 Host 中硬编码 Zed。
- 对声明了可靠 autosave 准备方式的编辑器，在启动前自动完成该配置。
- 从 OnetCli 打开的 Zed 临时工作区固定启用停止输入 1 秒后自动保存，作为首个完整适配。
- 不修改用户的 Zed 全局设置。
- 为 OnetCli 增加全局“自动上传外部编辑器的修改”开关。
- 新开关默认开启，保持现有外部编辑器保存后自动上传的行为。
- 自动上传关闭时不创建文件 watcher，也不执行远端状态查询或 SFTP 上传。
- 保留现有远端冲突检查和上传通知行为。
- 让工作区准备能力由编辑器扩展声明，避免 OnetCli 核心硬编码 Zed。

## 非目标

- 不修改、合并或恢复 `~/.config/zed/settings.json`。
- 不使用模拟键盘、Accessibility 自动点击或定时向第三方应用发送保存快捷键。这类做法会影响当前前台窗口、要求额外系统权限且无法证明保存的是目标会话。
- 不在已经打开的外部编辑会话中热切换 watcher。
- 不新增手动“立即上传”按钮；关闭自动上传后，修改只保留在会话临时文件中。
- 不改变内置远程文件编辑器的保存行为。
- 不改变现有 750ms OnetCli 上传防抖时间。

## 方案选择

### 采用：扩展声明工作区准备文件

在远程文件编辑器 command manifest 中增加可选 `workspaceFiles`：

```json
{
  "command": {
    "launchMode": "macos_open",
    "programCandidates": [
      "/Applications/Zed.app/Contents/MacOS/zed"
    ],
    "args": ["{file}"],
    "workspaceFiles": [
      {
        "path": ".zed/settings.json",
        "content": "{\n  \"autosave\": {\n    \"after_delay\": {\n      \"milliseconds\": 1000\n    }\n  }\n}"
      }
    ]
  }
}
```

Host 在下载远程文件后、启动外部编辑器前，将这些文件写入本次会话目录。这样编辑器专属配置由扩展维护，Host 只负责安全地准备文件。

### 未采用：Host 根据 editor key 硬编码 Zed

该方案改动较小，但会让核心代码依赖 `zed-macos`、`.zed/settings.json` 和 Zed 配置格式，后续编辑器会继续增加特殊分支。

### 未采用：修改 Zed 全局设置

该方案会影响所有 Zed 项目，并需要安全合并 JSONC、保留注释、处理并发写入和失败恢复，不符合外部编辑会话隔离原则。

## Manifest 与运行时 contract

新增工作区文件声明：

```rust
pub struct RemoteFileEditorWorkspaceFileContrib {
    pub path: String,
    pub content: String,
}
```

`RemoteFileEditorCommandContrib` 增加：

```rust
#[serde(default)]
pub workspace_files: Vec<RemoteFileEditorWorkspaceFileContrib>,
```

注册后的运行时 command 保留等价字段，使 manifest 解析、注册和实际启动链路完整传递工作区文件。

未声明 `workspaceFiles` 时默认空数组，现有扩展和 direct/macOS LaunchServices 启动行为保持不变。

该 contract 是通用外部编辑器能力：任何扩展都可以通过安全的会话相对文件声明项目级 autosave、格式化、编码或其他启动前工作区配置。Host 不根据 editor ID、应用名称或平台写特殊分支。

## Autosave 能力语义

OnetCli 将“autosave 已适配”定义为：扩展能通过声明式启动准备，让目标编辑器把当前会话的内存修改可靠写回 OnetCli 提供的本地临时主文件。只有扩展能够提供可验证的原生接口时才声明该能力。

- Zed：通过会话内 `.zed/settings.json` 完整适配 1000ms `after_delay` autosave。
- 其他支持项目级设置的编辑器：由各自扩展声明对应 workspace 文件，复用相同 Host contract。
- Notepad--：上游源码包含“Cycle Auto Save”功能，但当前实现是应用内工具栏开关，周期固定为 3 分钟；没有发现会话级配置文件、CLI 参数或 LaunchServices 参数可以只为 OnetCli 会话可靠开启。当前扩展因此不伪造 autosave 声明，用户在 Notepad-- 内主动开启原生 Cycle Auto Save 后，OnetCli 的通用 auto upload 会接收其写盘事件。
- Notepad++：当前官方核心没有可由 OnetCli 会话隔离配置的原生 autosave contract；若用户安装提供真实文件写盘能力的插件，OnetCli 的通用 auto upload 同样接收写盘事件。

“通用实现”指 Host contract、验证、会话准备、上传策略和设置对所有外部编辑器统一生效；不声称 OnetCli 能越过第三方编辑器公开能力保存其私有内存 buffer。

## 工作区文件安全边界

每个声明路径必须满足：

- 是非空相对路径。
- 不包含 `ParentDir`（`..`）、`RootDir` 或平台路径前缀。
- 规范化组件后仍位于当前会话目录内。
- 最终目标不能是会话主编辑文件本身。
- 单文件内容和全部工作区文件内容受具名常量限制，防止扩展生成过大临时内容。

Host 仅在当前会话目录内创建父目录和文件，不经过 shell，不解析 content 中的模板，不访问用户主目录。任何声明非法或写入失败都会终止本次外部编辑器启动，并通过现有错误通知路径反馈。

## 会话目录与启动数据流

Zed 会话目录示例：

```text
<temp>/onetcli/remote-edit/<session-id>/
├── dashboard.html
└── .zed/
    └── settings.json
```

启动顺序：

1. 生成本次会话目录和主编辑文件路径。
2. 从 SFTP 获取远端 metadata 和文件内容。
3. 创建会话目录并写入主编辑文件。
4. 校验并写入扩展声明的工作区文件。
5. 根据自动上传设置决定是否创建 watcher 和上传 controller。
6. 通过既有 direct 或 `macos_open` 启动计划打开编辑器。
7. 自动上传开启时，启动 watch loop；关闭时，本次会话不持有 watcher/controller。

工作区文件必须在 Zed 启动前写完，保证 Zed 创建 workspace 时即可读取 autosave 配置。

## 自动上传配置

`RemoteFileEditorUserSettings` 增加：

```rust
#[serde(default = "default_auto_upload_external_changes")]
pub auto_upload_external_changes: bool,
```

默认函数返回 `true`，`Default` 实现也显式设置为 `true`。因此旧配置文件省略该字段时仍保持当前自动上传行为。

配置示例：

```json
{
  "remote_file_editor": {
    "auto_upload_external_changes": true,
    "check_remote_modified_before_upload": true
  }
}
```

该值在创建外部编辑会话时读取并形成会话快照。运行中修改设置只影响之后新开的会话，不热切换已经存在的 watcher。

## 设置页面

在“设置 → 常规 → 远程文件编辑器”中按以下顺序显示：

1. 默认外部编辑器。
2. 自动上传外部编辑器的修改。
3. 上传前检查远程文件变更。
4. 各编辑器本机 executable override。

简体中文文案：

- 标题：`自动上传外部编辑器的修改`
- 说明：`外部编辑器保存本地临时文件后，自动将修改上传到远程服务器`

英文文案：

- 标题：`Automatically Upload External Editor Changes`
- 说明：`Automatically upload changes after an external editor saves the local temporary file`

繁体中文文案与现有 locale 结构同步补充。

冲突检查选项始终可见并独立持久化。关闭自动上传不会覆盖冲突检查偏好；重新开启自动上传后继续使用原值。

## 自动上传行为矩阵

| 自动上传 | 上传前冲突检查 | 实际行为 |
| --- | --- | --- |
| 开启 | 开启 | 写盘后检查远端 snapshot；发现远端变化或缺失时提示 |
| 开启 | 关闭 | 写盘后直接上传并覆盖远端 |
| 关闭 | 开启或关闭 | 不创建 watcher，不读取远端状态，不上传 |

自动上传开启时继续使用现有行为：精确监听主编辑文件、750ms 防抖、同步串行化、pending sync 合并、上传成功通知和失败通知。

## 编辑器扩展变更

Zed macOS 和 Linux contribution 都声明同一个 `.zed/settings.json` 工作区文件，使两端行为一致。Notepad-- 与 Notepad++ 扩展继续使用原有启动机制；只要编辑器自身或插件将目标文件写盘，Host 的自动上传行为完全一致。未来确认到可靠的会话级 autosave 接口时，只需更新扩展 manifest，无需修改 OnetCli Host。

扩展版本递增，并同步更新 marketplace 版本、release tag、archive 和校验信息。打包后必须检查 archive 内的 manifest，确认 Zed 两个平台 contribution 都携带 1000ms autosave 设置。

## 错误处理

- 非法工作区路径：扩展注册或启动失败，错误包含 editor ID 和路径原因。
- 工作区目录或文件写入失败：不启动编辑器，OnetCli 显示错误通知。
- Zed 自动保存失败：由 Zed 报告；磁盘文件未变化时 OnetCli 不触发上传。
- 本地 watcher 初始化失败：自动上传开启时保持当前失败语义，不启动不受监控的编辑会话。
- SFTP 上传失败：保留本地临时文件和现有错误通知；后续再次保存可重新触发同步。
- 远端冲突：继续使用覆盖、重新加载和取消三项提示。

## 测试策略

该改动新增 manifest/runtime contract、设置字段和明确行为变化，采用 TDD。

### Manifest 与 runtime

- 缺少 `workspaceFiles` 时默认空数组。
- 正确解析和注册 Zed 工作区文件。
- runtime catalog 保留 path/content。
- 拒绝空路径、绝对路径、`..`、平台前缀和目标逃逸。
- 限制单文件与总内容大小。

### 会话准备

- 在主编辑文件同级会话目录创建嵌套工作区文件。
- 写入内容与 manifest 完全一致。
- 工作区文件准备发生在外部编辑器启动计划之前。
- 准备失败时不调用 launcher。
- `.zed/settings.json` 事件不会被主文件 watcher 接受。

### 设置与上传行为

- `RemoteFileEditorUserSettings::default()` 默认开启自动上传。
- 旧 JSON 缺少字段时反序列化为开启。
- 显式 `false` 能正确读取、保存和展示。
- 自动上传关闭时不创建 watcher/controller，也不触发远端 stat/write。
- 自动上传开启时现有上传、冲突检查和通知测试继续通过。
- 设置页面 checkbox 正确更新持久化配置。

### 扩展仓库与人工验收

- 扩展仓库 verifier、archive 验证和 manifest contract 测试通过。
- 手工打开 Zed，停止输入约 1 秒后确认本地临时文件写盘并自动上传。
- 关闭自动上传后重新打开会话，确认 Zed 仍自动保存本地文件，但远端内容不变。
- 重新开启自动上传后新会话恢复上传。
- 关闭 Zed 后 OnetCli 保持运行；Zed 自身的 `app_will_quit` 超时日志不作为 OnetCli 失败判据。

## 验收标准

- Zed 从 OnetCli 打开的临时工作区无需依赖用户全局设置即可在停止输入 1 秒后自动保存。
- OnetCli 设置页存在默认开启的全局自动上传开关。
- 自动上传关闭时，新开的外部编辑会话不会执行 watcher、远端检查或上传。
- 自动上传开启时，现有防抖、冲突保护和上传通知保持不变。
- 工作区文件不能逃逸会话目录，非法扩展声明被明确拒绝。
- 通用 contract 能被任意外部编辑器扩展使用，且 Host 不包含 Zed/Notepad--/Notepad++ 的 editor-key 特判。
- Zed macOS/Linux 扩展完成原生 autosave 适配；Notepad--、Notepad++ 和现有不含新字段的第三方扩展保持兼容，并在实际写盘后使用同一自动上传链路。
- 相关 Rust 测试、扩展 verifier、格式检查、构建检查和人工验收均有真实验证记录。
