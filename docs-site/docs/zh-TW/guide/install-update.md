# 安裝與更新

Navop 現在以三個應用程式發布：完整版 **Navop**、遠端維運 **Navop Shell**、資料工具 **Navop Data**。三者使用同一版本號、隨同一版本發布，可以並排安裝，但安裝套件、應用程式名稱、資料目錄與鑰匙圈彼此獨立。本文先幫你選對安裝套件，再分別說明各平台的安裝、便攜執行、更新與解除安裝方式。

## 選擇要安裝的產品

| 你的使用方式 | 安裝產品 | 安裝檔前綴 |
| --- | --- | --- |
| 資料庫、伺服器、終端機、遠端桌面、監控都要用，或需要檔案關聯 | **Navop**（完整版） | `navop-` |
| 只做 SSH/SFTP、終端機、連接埠轉送、RDP/VNC、串口/Telnet | **Navop Shell** | `navop-shell-` |
| 只做 SQL、Redis、MongoDB、ER 圖、資料搬移 | **Navop Data** | `navop-data-` |

不確定時先裝完整版 Navop。之後如果只想保留遠端維運或資料工具，可以再並排安裝 Navop Shell 或 Navop Data，不需要先解除安裝 Navop。

## 安裝套件一覽

目前版本為 **v0.23.0**。三個產品使用同一套命名規則：

```text
navop-<version>-<label>.<ext>
navop-shell-<version>-<label>.<ext>
navop-data-<version>-<label>.<ext>
```

`<label>` 由平台與架構組成：

| label | 含義 | 可用產品 |
| --- | --- | --- |
| `macos-arm64` | macOS Apple Silicon | Navop / Navop Shell / Navop Data |
| `macos-x64` | macOS Intel | Navop / Navop Shell / Navop Data |
| `windows-x64` | Windows 64 位元 | Navop / Navop Shell / Navop Data |
| `win32` | Windows 32 位元 | 僅 Navop |
| `linux-x64` | Linux 64 位元 | Navop / Navop Shell / Navop Data |
| `linux-arm64` | Linux ARM64 | 僅 Navop |

### Navop（完整版）

| 平台 | 安裝套件 |
| --- | --- |
| macOS Apple Silicon | `navop-<version>-macos-arm64.dmg`、`navop-<version>-macos-arm64.tar.gz` |
| macOS Intel | `navop-<version>-macos-x64.dmg`、`navop-<version>-macos-x64.tar.gz` |
| Windows x64 | `navop-<version>-windows-x64.msi`、`navop-<version>-windows-x64.exe`、`navop-<version>-windows-x64.zip`、`navop-<version>-windows-x64-portable.zip` |
| Windows 32 位元 | `navop-<version>-win32.*` |
| Linux x64 | `navop-<version>-linux-x64.tar.gz`、`navop-<version>-linux-x64-portable.tar.gz`、`navop_<version>_amd64.deb`、`navop-<version>-1.x86_64.rpm`、`navop_<version>_amd64.AppImage`、`navop-<version>-linux-x64-gpu-stack.tar.gz` |
| Linux ARM64 | `navop-<version>-linux-arm64.tar.gz`、`navop-<version>-linux-arm64-portable.tar.gz` |

### Navop Shell（遠端維運）

| 平台 | 安裝套件 |
| --- | --- |
| macOS Apple Silicon | `navop-shell-<version>-macos-arm64.dmg`、`navop-shell-<version>-macos-arm64.tar.gz` |
| macOS Intel | `navop-shell-<version>-macos-x64.dmg`、`navop-shell-<version>-macos-x64.tar.gz` |
| Windows x64 | `navop-shell-<version>-windows-x64.msi`、`navop-shell-<version>-windows-x64.exe`、`navop-shell-<version>-windows-x64.zip`、`navop-shell-<version>-windows-x64-portable.zip` |
| Linux x64 | `navop-shell-<version>-linux-x64.tar.gz`、`navop-shell-<version>-linux-x64-portable.tar.gz` |

### Navop Data（資料工具）

| 平台 | 安裝套件 |
| --- | --- |
| macOS Apple Silicon | `navop-data-<version>-macos-arm64.dmg`、`navop-data-<version>-macos-arm64.tar.gz` |
| macOS Intel | `navop-data-<version>-macos-x64.dmg`、`navop-data-<version>-macos-x64.tar.gz` |
| Windows x64 | `navop-data-<version>-windows-x64.msi`、`navop-data-<version>-windows-x64.exe`、`navop-data-<version>-windows-x64.zip`、`navop-data-<version>-windows-x64-portable.zip` |
| Linux x64 | `navop-data-<version>-linux-x64.tar.gz`、`navop-data-<version>-linux-x64-portable.tar.gz` |

### 平台支援差異

| 目標或安裝方式 | Navop | Navop Shell | Navop Data |
| --- | --- | --- | --- |
| macOS ARM64 / x64 | 支援 | 支援 | 支援 |
| Windows x64 | 支援 | 支援 | 支援 |
| Windows 32 位元（win32） | 支援 | — | — |
| Linux x64 | 支援 | 支援 | 支援 |
| Linux ARM64 | 支援 | — | — |
| `.deb` / `.rpm` / `.AppImage` | 僅 Linux x64 | — | — |
| GPU 相依套件（`-gpu-stack`） | 僅 Linux x64 / arm64（完整版） | — | — |
| Scoop | 支援 | — | — |
| Flatpak | 支援 | — | — |
| 檔案關聯（`.db`/`.duckdb`/`.md`/`.cast`） | 支援 | — | — |

獨立產品沒有 Linux ARM64、Windows 32 位元、deb/rpm/AppImage 與 GPU 相依套件。這些平台請使用完整版 Navop。

## 下載與校驗

優先從以下位址下載：

- 下載中心：[https://navop.dev/zh-TW/extensions](https://navop.dev/zh-TW/extensions)
- GitHub Releases：[https://github.com/feigeCode/navop/releases](https://github.com/feigeCode/navop/releases)
- CNB：[https://cnb.cool/navop-dev/navop/-/releases](https://cnb.cool/navop-dev/navop/-/releases)
- Cloudflare R2 鏡像：`https://navop.pdyyds.cn/releases/<tag>/<file>`

每個正式版本都會附上 `sha256sums.txt`。下載後先校驗再安裝：

```bash
# Linux
sha256sum -c sha256sums.txt

# macOS
shasum -a 256 -c sha256sums.txt
```

Windows 可用 `certutil -hashfile <檔案> SHA256` 計算雜湊，再與 `sha256sums.txt` 中對應行比對。只校驗你實際下載的檔案，不要把不同版本的校驗檔混用。

## 首次啟動

啟動後先選擇介面語言、主題與預設啟動頁。SSH、SFTP、網路資料庫與遠端桌面都需要存取區域網路或網際網路；系統彈出本機網路、防火牆或鑰匙圈權限時，依實際連線範圍授權。Notes 目錄、外部編輯器與自訂字型需要對應的檔案系統權限。

建議先建立一個不含正式環境憑證的測試連線，確認 DNS、代理、防火牆與憑證鏈正常。需要連線匯入、遠端桌面 Provider、資料庫擴充或 ACP Agent 時，再從擴充商店安裝對應元件。

## macOS

1. 下載對應產品的 `.dmg`（建議）或 `.tar.gz`。
2. 開啟 `.dmg`，把 `Navop.app`、`Navop Shell.app` 或 `Navop Data.app` 拖進 `/Applications`。
3. 如果從網路下載後提示「已損毀」或無法開啟，移除隔離屬性：

```bash
sudo xattr -rd com.apple.quarantine /Applications/Navop.app
```

安裝的是 Navop Shell 或 Navop Data 時，把指令中的 `/Applications/Navop.app` 換成對應的應用程式套件路徑。

使用 `.tar.gz` 時，解壓後同樣把 `.app` 移到 `/Applications`，再視需要執行上面的指令。三個應用程式的套件名稱不同，可以同時放在 `/Applications`。

首次連線區域網路裝置時，macOS 會要求「本機網路」權限。若之前拒絕過，請在「系統設定 > 隱私權與安全性 > 本機網路」中重新允許對應應用程式。

## Windows

- **安裝版**：執行 `.msi` 或 `navop-<version>-windows-x64.exe`（EXE 安裝包），依精靈完成安裝。`.exe` 內嵌並啟動同一套 MSI 流程，兩者預設都安裝到目前使用者目錄，建立開始功能表與桌面捷徑，使用正常的 Windows 使用者資料目錄，並支援記住主密鑰後自動解鎖；使用預設位置不需要系統管理員權限。
- **免安裝 ZIP**：解壓 `navop-<version>-windows-x64.zip` 後直接執行目錄中的 `navop.exe`、`navop-shell.exe` 或 `navop-data.exe`。它不會建立捷徑或檔案關聯，但仍使用正常的 Windows 使用者資料目錄。
- **便攜版 ZIP**：解壓 `...-portable.zip`，資料會寫入解壓目錄下的 `data`，適合放在隨身碟或受控目錄使用。不要在一般 ZIP 旁放置標記檔，除非確實想切換到便攜模式。

如果 SmartScreen 提示不明的發行者，請先確認檔案來自上述官方來源並通過 `sha256sums.txt` 校驗，再決定是否繼續。完整版 Navop 另有 win32 目標，供 32 位元 Windows 使用。

## Linux

### 便攜版 tar.gz

三個產品都提供 Linux x64 的 `tar.gz` 與 `-portable.tar.gz`：

```bash
tar -xzf navop-<version>-linux-x64.tar.gz
cd navop-<version>-linux-x64
./navop
```

Navop Shell 與 Navop Data 的執行檔分別是 `navop-shell` 與 `navop-data`。`-portable.tar.gz` 解壓後直接以 `<安裝目錄>/data` 作為資料目錄，不會寫入使用者家目錄。

### deb / rpm / AppImage（僅完整版）

```bash
sudo apt install ./navop_<version>_amd64.deb
sudo dnf install ./navop-<version>-1.x86_64.rpm
chmod +x navop_<version>_amd64.AppImage
./navop_<version>_amd64.AppImage
```

這三種格式只隨完整版 Navop 的 Linux x64 目標發布，Navop Shell 與 Navop Data 沒有對應套件。

### GPU 圖形相依套件（僅完整版 Linux）

如果啟動時出現 `Failed to create surface: Failed to create surface for any enabled backend`，表示系統缺少可用的圖形後端。完整版 Navop 的 Linux x64 與 arm64 都提供對應的 GPU 圖形相依套件：x64 用 `navop-<version>-linux-x64-gpu-stack.tar.gz`，arm64 用 `navop-<version>-linux-arm64-gpu-stack.tar.gz`。下載後解壓並執行安裝腳本：

```bash
mkdir navop-gpu-stack
tar -xzf navop-<version>-linux-x64-gpu-stack.tar.gz -C navop-gpu-stack
sudo navop-gpu-stack/install.sh --dry-run   # 先查看將安裝的相依套件
sudo navop-gpu-stack/install.sh
```

安裝腳本是增量式的：系統已能解析的 SONAME 會跳過，主機自帶的版本優先；系統已有可用的 EGL 與 DRI 驅動時，整個 Mesa 渲染器也會跳過。`install.sh` 支援 `--dry-run`、`--force` 與 `--uninstall`，安裝紀錄保存在 `/usr/lib/navop-gpu-stack/installed.tsv`；解除安裝只會刪除腳本實際寫入的檔案。此套件以 glibc 2.28 為基準，只隨完整版 Navop 的 Linux 版本發布；Navop Shell 與 Navop Data 沒有 GPU 相依套件。

## Scoop（僅完整版）

Scoop 只發佈完整版 Navop，安裝的是便攜形態，`data` 由 Scoop 管理：

```powershell
scoop bucket add extras
scoop install navop
```

後續用 `scoop update navop` 升級。Scoop 安裝與 MSI/EXE 安裝版使用不同的資料目錄，切換安裝方式時請依便攜版說明搬移 `data`。Navop Shell 與 Navop Data 需要透過安裝版、ZIP 或便攜版安裝。

## Flatpak（僅完整版）

Flatpak 也只涵蓋完整版 Navop：

```bash
flatpak --user remote-add --if-not-exists flatpark https://dl.flatpark.org/flatpark.flatpakrepo
flatpak --user install flatpark dev.navop.Navop
flatpak run dev.navop.Navop
```

套件名為 `dev.navop.Navop`，應用程式頁面見 [https://flatpark.org/apps/dev.navop.Navop/](https://flatpark.org/apps/dev.navop.Navop/)。Flatpak 執行於沙盒中，SSH、資料庫、SFTP 或遠端桌面等整合可能需要額外授權。Navop Shell 與 Navop Data 沒有 Flatpak 套件。

## 便攜版

三個產品都支援便攜執行，但使用各自的標記檔：

| 產品 | 標記檔 | 執行檔 |
| --- | --- | --- |
| Navop | `navop.portable` | `navop` / `navop.exe` |
| Navop Shell | `navop-shell.portable` | `navop-shell` / `navop-shell.exe` |
| Navop Data | `navop-data.portable` | `navop-data` / `navop-data.exe` |

在執行檔同目錄放置對應標記檔（內容可以為空），應用程式就會把設定、狀態與快取寫入 `<安裝目錄>/data`。也可以不改檔案，直接使用環境變數或命令列：

```bash
NAVOP_PORTABLE=1 ./navop
NAVOP_DATA_DIR=/path/to/data ./navop
./navop --portable
./navop --data-dir /path/to/data
```

`NAVOP_PORTABLE` 接受 `1`、`true`、`yes`、`on`。資料目錄優先順序為：`--data-dir` > `--portable` > `NAVOP_DATA_DIR` > `NAVOP_PORTABLE` 或標記檔 > 一般使用者目錄。`NAVOP_*` 變數對三個產品通用，但標記檔名稱必須與產品相符。

首次啟動後，便攜目錄結構如下：

```text
<安裝目錄>/
├── navop            # 或 navop-shell / navop-data
├── navop.portable   # 或 navop-shell.portable / navop-data.portable
└── data/
    ├── config/
    ├── state/
    └── cache/
```

便攜目錄必須可寫；放在唯讀位置會導致啟動失敗。便攜模式預設不持久化主密鑰，每次啟動都需要重新輸入；可以在設定中明確選擇把加密副本保存到 `data/state/key_storage`，但這會降低保護強度。

便攜版不註冊檔案關聯，也不支援應用程式內更新與自動更新檢查。升級時先完全結束應用程式，把新版本解壓到新目錄，再把舊目錄的 `data` 整個複製過去，確認連線、擴充與 Notes 正常後再刪除舊目錄。搬移前應備份 `data`，並確保沒有兩個執行個體同時寫入同一份資料。

## 資料目錄、鑰匙圈與並排安裝

三個產品的目錄名稱與系統身分彼此獨立：

| 產品 | 目錄名稱 | 鑰匙圈服務 | bundle id |
| --- | --- | --- | --- |
| Navop | `navop` | `com.navop.desktop` | `com.onetcli.app` |
| Navop Shell | `navop-shell` | `com.navop.desktop.shell` | `com.navop.shell` |
| Navop Data | `navop-data` | `com.navop.desktop.data` | `com.navop.data` |

目錄位置依平台區分：

| 內容 | macOS | Linux | Windows |
| --- | --- | --- | --- |
| 設定 | `~/.config/<目錄名>` | `~/.config/<目錄名>` | `%APPDATA%\<目錄名>` |
| 資料 | `~/Library/Application Support/<目錄名>` | `~/.local/share/<目錄名>` | `%APPDATA%\<目錄名>` |
| 快取 | `~/Library/Caches/<目錄名>` | `~/.cache/<目錄名>` | `%APPDATA%\<目錄名>` |

例如 Navop Shell 的 Linux 資料目錄是 `~/.local/share/navop-shell`，Navop Data 的 Windows 設定目錄是 `%APPDATA%\navop-data`。目錄名稱不同，三個應用程式不會讀寫同一份連線與設定，可以並排安裝、同時執行。

憑證保存在系統鑰匙圈或認證儲存中，服務名稱使用上表的值。解除安裝應用程式通常不會自動刪除鑰匙圈項目與資料目錄，需要依下方步驟另行清理。

## 檔案關聯與命令列開啟

只有完整版 Navop 註冊 `.db`、`.duckdb`、`.md` 與 `.cast` 檔案關聯。雙擊這些檔案會交給 Navop 開啟；Navop Shell 與 Navop Data 不註冊關聯，也不會搶占既有的預設開啟方式。便攜模式同樣不註冊系統關聯，需要穩定關聯時請使用安裝版。

命令列可以直接傳入檔案：

```bash
navop ./data.db
navop ./notes.md
```

Windows 使用 `navop.exe`。如果系統仍把關聯指向舊版本，請先用安裝套件重新安裝完整版 Navop，或在系統「預設應用程式」中重新選擇。

## 應用程式內更新與回復

三個產品各自檢查自己的更新清單：

| 產品 | 更新清單 |
| --- | --- |
| Navop | `https://navop.pdyyds.cn/updates/latest.json` |
| Navop Shell | `https://navop.pdyyds.cn/updates/navop-shell/shell/latest.json` |
| Navop Data | `https://navop.pdyyds.cn/updates/navop-data/data/latest.json` |

應用程式內更新只替換目前產品，不會把 Navop Shell 升級成完整版 Navop，也不會影響並排安裝的其他產品。需要換用另一個產品時，請下載對應安裝套件另行安裝。

便攜版（包括 Windows `-portable.zip`、Linux `-portable.tar.gz` 與 Scoop 安裝）沒有應用程式內更新，也不會自動檢查更新，需要依「便攜版」一節的步驟手動升級。安裝版、一般 ZIP 與其他非便攜安裝形態可以在設定中開啟自動檢查或手動檢查更新。

回復到舊版時，下載舊版安裝套件覆蓋安裝即可，連線與設定通常保留在資料目錄中。但如果新版本升級過本機資料結構，舊版可能無法讀取；回復前先備份對應產品的資料目錄。

## 解除安裝與保留資料

解除安裝應用程式不會自動刪除連線、設定與歷史紀錄。需要徹底清理時，先結束應用程式，再依產品刪除對應目錄。

### macOS

把 `Navop.app`、`Navop Shell.app` 或 `Navop Data.app` 移到垃圾桶。然後視需要刪除：

```text
~/Library/Application Support/<目錄名>
~/Library/Caches/<目錄名>
~/.config/<目錄名>
```

### Windows

在「設定 > 應用程式」中解除安裝對應產品。安裝版會移除程式檔案，但保留 `%APPDATA%\<目錄名>`；便攜版直接刪除整個解壓目錄即可。需要徹底清理時再刪除 `%APPDATA%\<目錄名>`。

### Linux

便攜版刪除解壓目錄；deb/rpm 安裝版用套件管理器移除：

```bash
sudo apt remove navop
sudo dnf remove navop
```

AppImage 直接刪除檔案。之後視需要刪除 `~/.local/share/<目錄名>`、`~/.config/<目錄名>` 與 `~/.cache/<目錄名>`。

如果只想重設某個產品而不影響另外兩個，只刪除該產品目錄即可。不要把 `navop`、`navop-shell`、`navop-data` 的目錄合併或互相複製。

## 安裝後驗證清單

- [ ] 應用程式名稱與圖示正確：Navop、Navop Shell 或 Navop Data，而不是另一個產品。
- [ ] 「關於」頁顯示的版本為 v0.23.0。
- [ ] 資料目錄建立在對應產品的目錄名稱下，沒有寫入另外兩個產品的目錄。
- [ ] 檔案關聯只出現在完整版 Navop（`.db`/`.duckdb`/`.md`/`.cast`）。
- [ ] Public MCP 的探索名稱與產品一致：`navop`、`navop-shell` 或 `navop-data`。
- [ ] 並排安裝時，三個應用程式可以同時啟動，連線清單互不混入。
- [ ] 更新檢查讀取的是目前產品自己的清單。
