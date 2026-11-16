# 安装与更新

Navop 是用 Rust 与 GPUI 原生构建的桌面工作台，安装包覆盖 macOS、Windows 和 Linux。本文说明如何选择安装包、校验下载、首次启动、便携运行、应用内更新，以及卸载时如何保留或清理数据。

同一份源码还会构建两个范围更小的版本：**Navop Shell**（SSH、终端与远程桌面）和 **Navop Data**（数据库与数据浏览）。它们使用 `navop-shell-` / `navop-data-` 前缀，平台目标较少；除非明确只需要这部分能力，否则直接安装 Navop 即可。两者的安装步骤与 Navop 相同，把下文的应用名和可执行文件名替换即可。

## 安装包一览

安装包命名规则：

```text
navop-<version>-<label>.<ext>
```

`<label>` 由平台和架构组成：

| label | 含义 |
| --- | --- |
| `macos-arm64` | macOS Apple Silicon |
| `macos-x64` | macOS Intel |
| `windows-x64` | Windows 64 位 |
| `win32` | Windows 32 位 |
| `linux-x64` | Linux 64 位 |
| `linux-arm64` | Linux ARM64 |

| 平台 | 安装包 |
| --- | --- |
| macOS Apple Silicon | `navop-<version>-macos-arm64.dmg`、`navop-<version>-macos-arm64.tar.gz` |
| macOS Intel | `navop-<version>-macos-x64.dmg`、`navop-<version>-macos-x64.tar.gz` |
| Windows x64 | `navop-<version>-windows-x64.msi`、`navop-<version>-windows-x64.exe`、`navop-<version>-windows-x64.zip`、`navop-<version>-windows-x64-portable.zip` |
| Windows 32 位 | `navop-<version>-win32.*` |
| Linux x64 | `navop-<version>-linux-x64.tar.gz`、`navop-<version>-linux-x64-portable.tar.gz`、`navop_<version>_amd64.deb`、`navop-<version>-1.x86_64.rpm`、`navop_<version>_amd64.AppImage`、`navop-<version>-linux-x64-gpu-stack.tar.gz` |
| Linux ARM64 | `navop-<version>-linux-arm64.tar.gz`、`navop-<version>-linux-arm64-portable.tar.gz`、`navop_<version>_arm64.deb`、`navop-<version>-1.aarch64.rpm`、`navop_<version>_arm64.AppImage`、`navop-<version>-linux-arm64-gpu-stack.tar.gz` |

此外还提供 Scoop 与 Flatpak 安装方式，见下文。当前版本号以[下载中心](https://navop.dev/zh-CN/extensions)或 Releases 页面为准，不在本文硬编码。

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

1. 下载 `.dmg`（推荐）或 `.tar.gz`。
2. 打开 `.dmg`，把 `Navop.app` 拖入 `/Applications`。
3. 如果从网络下载后提示“已损坏”或无法打开，移除隔离属性：

```bash
sudo xattr -rd com.apple.quarantine /Applications/Navop.app
```

使用 `.tar.gz` 时，解压后同样把 `.app` 移到 `/Applications`，再按需执行上面的去隔离命令。

首次连接局域网设备时，macOS 会请求“本地网络”权限。若之前拒绝过，请在“系统设置 > 隐私与安全性 > 本地网络”中重新允许 Navop。

## Windows 安装

- **安装版**：运行 `.msi` 或 `navop-<version>-windows-x64.exe`（EXE 安装包），按向导完成安装。`.exe` 内嵌并启动同一套 MSI 安装流程，两者默认都安装到当前用户目录，创建开始菜单和桌面快捷方式，使用正常的 Windows 用户数据目录，并支持记住主密钥后自动解锁；使用默认位置时不需要管理员权限。
- **免安装 ZIP**：解压 `navop-<version>-windows-x64.zip` 后直接运行目录中的 `navop.exe`。它不会创建快捷方式或文件关联，但仍使用正常的 Windows 用户数据目录。
- **便携版 ZIP**：解压 `...-portable.zip`，数据会写入解压目录下的 `data`，适合放在 U 盘或受控目录中使用。不要在普通 ZIP 旁放置标记文件，除非确实想切换到便携模式。

如果 SmartScreen 提示未知发布者，请先确认文件来自上面的官方地址并通过 `sha256sums.txt` 校验，再决定是否继续。32 位 Windows 使用 `win32` 目标。

## Linux 安装

### 便携版 tar.gz

```bash
tar -xzf navop-<version>-linux-x64.tar.gz
cd navop-<version>-linux-x64
./navop
```

`-portable.tar.gz` 解压后直接以 `<安装目录>/data` 作为数据目录，不会写入用户主目录。

### deb / rpm / AppImage

```bash
# Linux x64
sudo apt install ./navop_<version>_amd64.deb
sudo dnf install ./navop-<version>-1.x86_64.rpm
chmod +x navop_<version>_amd64.AppImage
./navop_<version>_amd64.AppImage

# Linux ARM64
sudo apt install ./navop_<version>_arm64.deb
sudo dnf install ./navop-<version>-1.aarch64.rpm
chmod +x navop_<version>_arm64.AppImage
./navop_<version>_arm64.AppImage
```

这三种包随 Linux x64 与 ARM64 两个目标发布，文件名里的架构后缀不同：x64 用 `amd64`（deb / AppImage）和 `x86_64`（rpm），ARM64 用 `arm64`（deb / AppImage）和 `aarch64`（rpm）。

### GPU 图形依赖包

如果启动时报 `Failed to create surface: Failed to create surface for any enabled backend`，说明系统缺少可用的图形后端库。Linux x64 和 arm64 都有对应的 GPU 图形依赖包：x64 用 `navop-<version>-linux-x64-gpu-stack.tar.gz`，arm64 用 `navop-<version>-linux-arm64-gpu-stack.tar.gz`。下载后解压并运行包内的 `install.sh`：

```bash
mkdir navop-gpu-stack
tar -xzf navop-<version>-linux-x64-gpu-stack.tar.gz -C navop-gpu-stack
sudo navop-gpu-stack/install.sh --dry-run   # 先查看将要安装的依赖
sudo navop-gpu-stack/install.sh
```

安装脚本是增量式的：系统已经能解析的 SONAME 会被跳过，宿主自带的版本优先；系统已有可用的 EGL 与 DRI 驱动时，整个 Mesa 渲染器也会跳过。`install.sh` 支持 `--dry-run`、`--force` 和 `--uninstall`，安装记录保存在 `/usr/lib/navop-gpu-stack/installed.tsv`；卸载只删除脚本实际写入的文件。该包以 glibc 2.28 为基线。

## Scoop

Scoop 安装的是便携形态，`data` 由 Scoop 管理：

```powershell
scoop bucket add extras
scoop install navop
```

后续用 `scoop update navop` 升级。Scoop 安装与 MSI/EXE 安装版使用不同的数据目录，切换安装方式时请按便携版说明迁移 `data`。

## Flatpak

```bash
flatpak --user remote-add --if-not-exists flatpark https://dl.flatpark.org/flatpark.flatpakrepo
flatpak --user install flatpark dev.navop.Navop
flatpak run dev.navop.Navop
```

包名为 `dev.navop.Navop`，应用页面见 [https://flatpark.org/apps/dev.navop.Navop/](https://flatpark.org/apps/dev.navop.Navop/)。Flatpak 运行在沙盒中，SSH、数据库、SFTP 或远程桌面等集成可能需要额外授权。

## 便携版

在可执行文件同目录放置标记文件 `navop.portable`（内容可以为空），应用就会把配置、状态和缓存写入 `<安装目录>/data`。也可以不改文件，直接用环境变量或命令行：

```bash
NAVOP_PORTABLE=1 ./navop
NAVOP_DATA_DIR=/path/to/data ./navop
./navop --portable
./navop --data-dir /path/to/data
```

`NAVOP_PORTABLE` 接受 `1`、`true`、`yes`、`on`。数据目录优先级为：`--data-dir` > `--portable` > `NAVOP_DATA_DIR` > `NAVOP_PORTABLE` 或标记文件 > 常规用户目录。

首次启动后，便携目录结构如下：

```text
<安装目录>/
├── navop
├── navop.portable
└── data/
    ├── config/
    ├── state/
    └── cache/
```

便携目录必须可写；放在只读位置会导致启动失败。便携模式默认不持久化主密钥，每次启动都需要重新输入；可以在设置中显式选择把加密副本保存到 `data/state/key_storage`，但这会降低保护强度。

便携版不注册文件关联，也不支持应用内更新和自动更新检查。升级时先完全退出应用，把新版本解压到新目录，再把旧目录的 `data` 整体复制过去，确认连接、扩展和 Notes 正常后再删除旧目录。迁移前应备份 `data`，并确保没有两个实例同时写入同一份数据。

## 数据目录与钥匙串

| 内容 | macOS | Linux | Windows |
| --- | --- | --- | --- |
| 配置 | `~/.config/navop` | `~/.config/navop` | `%APPDATA%\navop` |
| 数据 | `~/Library/Application Support/navop` | `~/.local/share/navop` | `%APPDATA%\navop` |
| 缓存 | `~/Library/Caches/navop` | `~/.cache/navop` | `%APPDATA%\navop` |

凭据保存在系统钥匙串或凭据存储中，服务名为 `com.navop.desktop`，bundle id 为 `com.onetcli.app`。卸载应用通常不会自动删除钥匙串条目和数据目录，需要按下面的步骤单独清理。

如果同时安装了 Navop Shell 或 Navop Data，它们分别使用 `navop-shell` / `navop-data` 目录和独立的钥匙串条目，不会与 Navop 共用连接密文。不要把这几个目录合并或互相复制。

## 文件关联与命令行打开

Navop 注册 `.db`、`.duckdb`、`.md` 和 `.cast` 文件关联。双击这些文件会交给 Navop 打开；便携模式不注册系统关联，需要稳定关联时请使用安装版。

命令行可以直接传入文件：

```bash
navop ./data.db
navop ./notes.md
```

Windows 使用 `navop.exe`。如果系统仍把关联指向旧版本，先用安装包重装，或在系统“默认应用”中重新选择。

## 应用内更新与回退

安装版、普通 ZIP 和其他常规安装形态可以在设置中开启自动检查或手动检查更新。Navop 读取 `https://navop.pdyyds.cn/updates/latest.json`；如果同时安装了精简版，它们各自读取自己的更新清单，互不影响。

便携版（包括 Windows `-portable.zip`、Linux `-portable.tar.gz` 和 Scoop 安装）没有应用内更新，也不会自动检查更新，需要按“便携版”一节的步骤手工升级。

回退到旧版本时，下载旧版安装包覆盖安装即可，连接和设置通常保留在数据目录中。但如果新版本升级过本地数据结构，旧版本可能无法读取；回退前先备份数据目录。

## 卸载与保留数据

卸载应用不会自动删除连接、设置和历史记录。需要彻底清理时，先退出应用，再删除对应目录。

### macOS

把 `Navop.app` 移到废纸篓。然后按需删除：

```text
~/Library/Application Support/navop
~/Library/Caches/navop
~/.config/navop
```

### Windows

在“设置 > 应用”中卸载 Navop。安装版会移除程序文件，但保留 `%APPDATA%\navop`；便携版直接删除整个解压目录即可。需要彻底清理时再删除 `%APPDATA%\navop`。

### Linux

便携版删除解压目录；deb/rpm 安装版用包管理器卸载：

```bash
sudo apt remove navop
sudo dnf remove navop
```

AppImage 直接删除文件。之后按需删除 `~/.local/share/navop`、`~/.config/navop` 和 `~/.cache/navop`。

## 安装后验证清单

- [ ] 应用名与图标为 Navop。
- [ ] “关于”页显示的版本与下载页一致。
- [ ] 数据目录创建为 `navop`，没有混入其他目录。
- [ ] 文件关联包含 `.db`、`.duckdb`、`.md`、`.cast`。
- [ ] Public MCP 的发现名为 `navop`。
- [ ] 更新检查读取的是 Navop 的更新清单。
- [ ] 能新建一个测试连接并成功执行“测试连接”。
