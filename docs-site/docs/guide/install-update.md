# 安装与更新

Navop 现在以三个应用发布：全量 **Navop**、远程运维 **Navop Shell**、数据工具 **Navop Data**。三者使用同一版本号、随同一版本发布，可以并排安装，但安装包、应用名、数据目录和钥匙串彼此独立。本文先帮你选对安装包，再分别说明各平台的安装、便携运行、更新和卸载方式。

## 选择要安装的产品

| 你的使用方式 | 安装产品 | 安装包前缀 |
| --- | --- | --- |
| 数据库、服务器、终端、远程桌面、监控都要用，或需要文件关联 | **Navop**（全量版） | `navop-` |
| 只做 SSH/SFTP、终端、端口转发、RDP/VNC、串口/Telnet | **Navop Shell** | `navop-shell-` |
| 只做 SQL、Redis、MongoDB、ER 图、数据迁移 | **Navop Data** | `navop-data-` |

拿不准时先装全量 Navop。之后如果只想保留远程运维或数据工具，可以再并排安装 Navop Shell 或 Navop Data，不需要先卸载 Navop。

## 安装包一览

当前版本为 **v0.23.0**。三产品使用同一套命名规则：

```text
navop-<version>-<label>.<ext>
navop-shell-<version>-<label>.<ext>
navop-data-<version>-<label>.<ext>
```

`<label>` 由平台和架构组成：

| label | 含义 | 可用产品 |
| --- | --- | --- |
| `macos-arm64` | macOS Apple Silicon | Navop / Navop Shell / Navop Data |
| `macos-x64` | macOS Intel | Navop / Navop Shell / Navop Data |
| `windows-x64` | Windows 64 位 | Navop / Navop Shell / Navop Data |
| `win32` | Windows 32 位 | 仅 Navop |
| `linux-x64` | Linux 64 位 | Navop / Navop Shell / Navop Data |
| `linux-arm64` | Linux ARM64 | 仅 Navop |

### Navop（全量版）

| 平台 | 安装包 |
| --- | --- |
| macOS Apple Silicon | `navop-<version>-macos-arm64.dmg`、`navop-<version>-macos-arm64.tar.gz` |
| macOS Intel | `navop-<version>-macos-x64.dmg`、`navop-<version>-macos-x64.tar.gz` |
| Windows x64 | `navop-<version>-windows-x64.msi`、`navop-<version>-windows-x64.exe`、`navop-<version>-windows-x64.zip`、`navop-<version>-windows-x64-portable.zip` |
| Windows 32 位 | `navop-<version>-win32.*` |
| Linux x64 | `navop-<version>-linux-x64.tar.gz`、`navop-<version>-linux-x64-portable.tar.gz`、`navop_<version>_amd64.deb`、`navop-<version>-1.x86_64.rpm`、`navop_<version>_amd64.AppImage`、`navop-<version>-linux-x64-gpu-stack.tar.gz` |
| Linux ARM64 | `navop-<version>-linux-arm64.tar.gz`、`navop-<version>-linux-arm64-portable.tar.gz` |

### Navop Shell（远程运维）

| 平台 | 安装包 |
| --- | --- |
| macOS Apple Silicon | `navop-shell-<version>-macos-arm64.dmg`、`navop-shell-<version>-macos-arm64.tar.gz` |
| macOS Intel | `navop-shell-<version>-macos-x64.dmg`、`navop-shell-<version>-macos-x64.tar.gz` |
| Windows x64 | `navop-shell-<version>-windows-x64.msi`、`navop-shell-<version>-windows-x64.exe`、`navop-shell-<version>-windows-x64.zip`、`navop-shell-<version>-windows-x64-portable.zip` |
| Linux x64 | `navop-shell-<version>-linux-x64.tar.gz`、`navop-shell-<version>-linux-x64-portable.tar.gz` |

### Navop Data（数据工具）

| 平台 | 安装包 |
| --- | --- |
| macOS Apple Silicon | `navop-data-<version>-macos-arm64.dmg`、`navop-data-<version>-macos-arm64.tar.gz` |
| macOS Intel | `navop-data-<version>-macos-x64.dmg`、`navop-data-<version>-macos-x64.tar.gz` |
| Windows x64 | `navop-data-<version>-windows-x64.msi`、`navop-data-<version>-windows-x64.exe`、`navop-data-<version>-windows-x64.zip`、`navop-data-<version>-windows-x64-portable.zip` |
| Linux x64 | `navop-data-<version>-linux-x64.tar.gz`、`navop-data-<version>-linux-x64-portable.tar.gz` |

### 平台支持差异

| 目标或安装方式 | Navop | Navop Shell | Navop Data |
| --- | --- | --- | --- |
| macOS ARM64 / x64 | 支持 | 支持 | 支持 |
| Windows x64 | 支持 | 支持 | 支持 |
| Windows 32 位（win32） | 支持 | — | — |
| Linux x64 | 支持 | 支持 | 支持 |
| Linux ARM64 | 支持 | — | — |
| `.deb` / `.rpm` / `.AppImage` | 仅 Linux x64 | — | — |
| GPU 依赖包（`-gpu-stack`） | 仅 Linux x64 / arm64 | — | — |
| Scoop | 支持 | — | — |
| Flatpak | 支持 | — | — |
| 文件关联（`.db`/`.duckdb`/`.md`/`.cast`） | 支持 | — | — |

独立产品没有 Linux ARM64、Windows 32 位、deb/rpm/AppImage 和 GPU 依赖包。这些平台的用户需要使用全量 Navop。

## 下载与校验

优先从以下地址下载：

- 下载中心：[https://navop.dev/zh-CN/extensions](https://navop.dev/zh-CN/extensions)
- GitHub Releases：[https://github.com/feigeCode/navop/releases](https://github.com/feigeCode/navop/releases)
- CNB：[https://cnb.cool/navop-dev/navop/-/releases](https://cnb.cool/navop-dev/navop/-/releases)
- Cloudflare R2 镜像：`https://navop.pdyyds.cn/releases/<tag>/<file>`

每个正式发布都附带 `sha256sums.txt`。下载后先校验再安装：

```bash
# Linux
sha256sum -c sha256sums.txt

# macOS
shasum -a 256 -c sha256sums.txt
```

Windows 可以用 `certutil -hashfile <文件> SHA256` 计算哈希，再与 `sha256sums.txt` 中对应行比对。只校验你实际下载的文件，不要把不同版本的校验文件混用。

## 首次启动

启动后先选择界面语言、主题和默认启动页。SSH、SFTP、网络数据库和远程桌面都需要访问局域网或互联网；系统弹出本地网络、防火墙或钥匙串权限时，按实际连接范围授权。Notes 目录、外部编辑器和自定义字体需要相应的文件系统权限。

建议先创建一个不含生产凭据的测试连接，确认 DNS、代理、防火墙和证书链正常。需要连接导入、远程桌面 Provider、数据库扩展或 ACP Agent 时，再从扩展市场安装对应组件。

## macOS 安装

1. 下载对应产品的 `.dmg`（推荐）或 `.tar.gz`。
2. 打开 `.dmg`，把 `Navop.app`、`Navop Shell.app` 或 `Navop Data.app` 拖入 `/Applications`。
3. 如果从网络下载后提示“已损坏”或无法打开，移除隔离属性：

```bash
sudo xattr -rd com.apple.quarantine /Applications/Navop.app
```

安装的是 Navop Shell 或 Navop Data 时，把命令中的 `/Applications/Navop.app` 换成对应的应用包路径。

使用 `.tar.gz` 时，解压后同样把 `.app` 移到 `/Applications`，再按需执行上面的去隔离命令。三个应用的应用包名不同，可以同时放在 `/Applications` 中。

首次连接局域网设备时，macOS 会请求“本地网络”权限。若之前拒绝过，请在“系统设置 > 隐私与安全性 > 本地网络”中重新允许对应应用。

## Windows 安装

- **安装版**：运行 `.msi` 或 `navop-<version>-windows-x64.exe`（EXE 安装包），按向导完成安装。`.exe` 内嵌并启动同一套 MSI 安装流程，两者默认都安装到当前用户目录，创建开始菜单和桌面快捷方式，使用正常的 Windows 用户数据目录，并支持记住主密钥后自动解锁；使用默认位置时不需要管理员权限。
- **免安装 ZIP**：解压 `navop-<version>-windows-x64.zip` 后直接运行目录中的 `navop.exe`、`navop-shell.exe` 或 `navop-data.exe`。它不会创建快捷方式或文件关联，但仍使用正常的 Windows 用户数据目录。
- **便携版 ZIP**：解压 `...-portable.zip`，数据会写入解压目录下的 `data`，适合放在 U 盘或受控目录中使用。不要在普通 ZIP 旁放置标记文件，除非确实想切换到便携模式。

如果 SmartScreen 提示未知发布者，请先确认文件来自上面的官方地址并通过 `sha256sums.txt` 校验，再决定是否继续。全量 Navop 另有 win32 目标，供 32 位 Windows 使用。

## Linux 安装

### 便携版 tar.gz

三产品都提供 Linux x64 的 `tar.gz` 与 `-portable.tar.gz`：

```bash
tar -xzf navop-<version>-linux-x64.tar.gz
cd navop-<version>-linux-x64
./navop
```

Navop Shell 和 Navop Data 对应可执行文件分别是 `navop-shell` 和 `navop-data`。`-portable.tar.gz` 解压后直接以 `<安装目录>/data` 作为数据目录，不会写入用户主目录。

### deb / rpm / AppImage（仅全量）

```bash
sudo apt install ./navop_<version>_amd64.deb
sudo dnf install ./navop-<version>-1.x86_64.rpm
chmod +x navop_<version>_amd64.AppImage
./navop_<version>_amd64.AppImage
```

这三种包只随全量 Navop 的 Linux x64 目标发布，Navop Shell 与 Navop Data 没有对应包。

### GPU 图形依赖包（仅全量 Linux）

如果启动时报 `Failed to create surface: Failed to create surface for any enabled backend`，说明系统缺少可用的图形后端库。全量 Navop 的 Linux x64 和 arm64 都有对应的 GPU 图形依赖包：x64 用 `navop-<version>-linux-x64-gpu-stack.tar.gz`，arm64 用 `navop-<version>-linux-arm64-gpu-stack.tar.gz`。下载后解压并运行包内的 `install.sh`：

```bash
mkdir navop-gpu-stack
tar -xzf navop-<version>-linux-x64-gpu-stack.tar.gz -C navop-gpu-stack
sudo navop-gpu-stack/install.sh --dry-run   # 先查看将要安装的依赖
sudo navop-gpu-stack/install.sh
```

安装脚本是增量式的：系统已经能解析的 SONAME 会被跳过，宿主自带的版本优先；系统已有可用的 EGL 与 DRI 驱动时，整个 Mesa 渲染器也会跳过。`install.sh` 支持 `--dry-run`、`--force` 和 `--uninstall`，安装记录保存在 `/usr/lib/navop-gpu-stack/installed.tsv`；卸载只删除脚本实际写入的文件。该包以 glibc 2.28 为基线，只随全量 Navop 的 Linux 版本发布；Navop Shell 与 Navop Data 没有 GPU 依赖包。

## Scoop（仅全量）

Scoop 只分发全量 Navop，安装的是便携形态，`data` 由 Scoop 管理：

```powershell
scoop bucket add extras
scoop install navop
```

后续用 `scoop update navop` 升级。Scoop 安装与 MSI/EXE 安装版使用不同的数据目录，切换安装方式时请按便携版说明迁移 `data`。Navop Shell 和 Navop Data 需要通过安装版、ZIP 或便携版安装。

## Flatpak（仅全量）

Flatpak 也只覆盖全量 Navop：

```bash
flatpak --user remote-add --if-not-exists flatpark https://dl.flatpark.org/flatpark.flatpakrepo
flatpak --user install flatpark dev.navop.Navop
flatpak run dev.navop.Navop
```

包名为 `dev.navop.Navop`，应用页面见 [https://flatpark.org/apps/dev.navop.Navop/](https://flatpark.org/apps/dev.navop.Navop/)。Flatpak 运行在沙盒中，SSH、数据库、SFTP 或远程桌面等集成可能需要额外授权。Navop Shell 与 Navop Data 没有 Flatpak 包。

## 便携版

三个产品都支持便携运行，但使用各自的标记文件：

| 产品 | 标记文件 | 可执行文件 |
| --- | --- | --- |
| Navop | `navop.portable` | `navop` / `navop.exe` |
| Navop Shell | `navop-shell.portable` | `navop-shell` / `navop-shell.exe` |
| Navop Data | `navop-data.portable` | `navop-data` / `navop-data.exe` |

在可执行文件同目录放置对应标记文件（内容可以为空），应用就会把配置、状态和缓存写入 `<安装目录>/data`。也可以不改文件，直接用环境变量或命令行：

```bash
NAVOP_PORTABLE=1 ./navop
NAVOP_DATA_DIR=/path/to/data ./navop
./navop --portable
./navop --data-dir /path/to/data
```

`NAVOP_PORTABLE` 接受 `1`、`true`、`yes`、`on`。数据目录优先级为：`--data-dir` > `--portable` > `NAVOP_DATA_DIR` > `NAVOP_PORTABLE` 或标记文件 > 常规用户目录。`NAVOP_*` 变量对三个产品通用，但标记文件名必须与产品匹配。

首次启动后，便携目录结构如下：

```text
<安装目录>/
├── navop            # 或 navop-shell / navop-data
├── navop.portable   # 或 navop-shell.portable / navop-data.portable
└── data/
    ├── config/
    ├── state/
    └── cache/
```

便携目录必须可写；放在只读位置会导致启动失败。便携模式默认不持久化主密钥，每次启动都需要重新输入；可以在设置中显式选择把加密副本保存到 `data/state/key_storage`，但这会降低保护强度。

便携版不注册文件关联，也不支持应用内更新和自动更新检查。升级时先完全退出应用，把新版本解压到新目录，再把旧目录的 `data` 整体复制过去，确认连接、扩展和 Notes 正常后再删除旧目录。迁移前应备份 `data`，并确保没有两个实例同时写入同一份数据。

## 数据目录、钥匙串与并排安装

三个产品的目录名与系统身份彼此独立：

| 产品 | 目录名 | 钥匙串服务 | bundle id |
| --- | --- | --- | --- |
| Navop | `navop` | `com.navop.desktop` | `com.onetcli.app` |
| Navop Shell | `navop-shell` | `com.navop.desktop.shell` | `com.navop.shell` |
| Navop Data | `navop-data` | `com.navop.desktop.data` | `com.navop.data` |

目录位置按平台区分：

| 内容 | macOS | Linux | Windows |
| --- | --- | --- | --- |
| 配置 | `~/.config/<目录名>` | `~/.config/<目录名>` | `%APPDATA%\<目录名>` |
| 数据 | `~/Library/Application Support/<目录名>` | `~/.local/share/<目录名>` | `%APPDATA%\<目录名>` |
| 缓存 | `~/Library/Caches/<目录名>` | `~/.cache/<目录名>` | `%APPDATA%\<目录名>` |

例如 Navop Shell 的 Linux 数据目录是 `~/.local/share/navop-shell`，Navop Data 的 Windows 配置目录是 `%APPDATA%\navop-data`。目录名不同，三个应用不会读写同一份连接和设置，可以并排安装、同时运行。

凭据保存在系统钥匙串或凭据存储中，服务名使用上表中的值。卸载应用通常不会自动删除钥匙串条目和数据目录，需要按下面的步骤单独清理。

## 文件关联与命令行打开

只有全量 Navop 注册 `.db`、`.duckdb`、`.md` 和 `.cast` 文件关联。双击这些文件会交给 Navop 打开；Navop Shell 和 Navop Data 不注册关联，也不会抢占已有的默认打开方式。便携模式同样不注册系统关联，需要稳定关联时请使用安装版。

命令行可以直接传入文件：

```bash
navop ./data.db
navop ./notes.md
```

Windows 使用 `navop.exe`。如果系统仍把关联指向旧版本，先用安装包重装全量 Navop，或在系统“默认应用”中重新选择。

## 应用内更新与回退

三个产品各自检查自己的更新清单：

| 产品 | 更新清单 |
| --- | --- |
| Navop | `https://navop.pdyyds.cn/updates/latest.json` |
| Navop Shell | `https://navop.pdyyds.cn/updates/navop-shell/shell/latest.json` |
| Navop Data | `https://navop.pdyyds.cn/updates/navop-data/data/latest.json` |

应用内更新只替换当前产品，不会把 Navop Shell 升级成全量 Navop，也不会影响并排安装的其他产品。需要换用另一个产品时，请下载对应安装包单独安装。

便携版（包括 Windows `-portable.zip`、Linux `-portable.tar.gz` 和 Scoop 安装）没有应用内更新，也不会自动检查更新，需要按“便携版”一节的步骤手工升级。安装版、普通 ZIP 和其他常规安装形态可以在设置中开启自动检查或手动检查更新。

回退到旧版本时，下载旧版安装包覆盖安装即可，连接和设置通常保留在数据目录中。但如果新版本升级过本地数据结构，旧版本可能无法读取；回退前先备份对应产品的数据目录。

## 卸载与保留数据

卸载应用不会自动删除连接、设置和历史记录。需要彻底清理时，先退出应用，再按产品删除对应目录。

### macOS

把 `Navop.app`、`Navop Shell.app` 或 `Navop Data.app` 移到废纸篓。然后按需删除：

```text
~/Library/Application Support/<目录名>
~/Library/Caches/<目录名>
~/.config/<目录名>
```

### Windows

在“设置 > 应用”中卸载对应产品。安装版会移除程序文件，但保留 `%APPDATA%\<目录名>`；便携版直接删除整个解压目录即可。需要彻底清理时再删除 `%APPDATA%\<目录名>`。

### Linux

便携版删除解压目录；deb/rpm 安装版用包管理器卸载：

```bash
sudo apt remove navop
sudo dnf remove navop
```

AppImage 直接删除文件。之后按需删除 `~/.local/share/<目录名>`、`~/.config/<目录名>` 和 `~/.cache/<目录名>`。

如果只想重置某个产品而不影响另外两个，只删除该产品目录即可。不要把 `navop`、`navop-shell`、`navop-data` 的目录合并或互相复制。

## 安装后验证清单

- [ ] 应用名与图标正确：Navop、Navop Shell 或 Navop Data，而不是另一个产品。
- [ ] “关于”页显示的版本为 v0.23.0。
- [ ] 数据目录创建在对应产品的目录名下，没有写入另外两个产品的目录。
- [ ] 文件关联只出现在全量 Navop（`.db`/`.duckdb`/`.md`/`.cast`）。
- [ ] Public MCP 的发现名与产品一致：`navop`、`navop-shell` 或 `navop-data`。
- [ ] 并排安装时，三个应用可以同时启动，连接列表互不混入。
- [ ] 更新检查读取的是当前产品自己的清单。
