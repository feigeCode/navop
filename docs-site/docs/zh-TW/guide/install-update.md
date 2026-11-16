# 安裝與更新

Navop 是以 Rust 與 GPUI 原生建構的桌面工作台，安裝檔涵蓋 macOS、Windows 和 Linux。本文說明如何安裝、驗證下載、首次啟動、便攜執行、應用程式內更新，以及解除安裝時如何保留或清理資料。

同一份原始碼也會建構兩個範圍更小的版本：**Navop Shell**（SSH、終端機與遠端桌面）和 **Navop Data**（資料庫與資料瀏覽）。它們使用 `navop-shell-` / `navop-data-` 前綴，平台目標較少；除非明確只需要這部分能力，否則直接安裝 Navop 即可。兩者的安裝步驟與 Navop 相同，把下文的名稱和可執行檔名替換即可。

## 安裝檔一覽

安裝檔命名規則：

```text
navop-<version>-<label>.<ext>
```

`<label>` 由平台和架構組成：

| label | 含義 |
| --- | --- |
| `macos-arm64` | macOS Apple Silicon |
| `macos-x64` | macOS Intel |
| `windows-x64` | Windows 64 位元 |
| `win32` | Windows 32 位元 |
| `linux-x64` | Linux 64 位元 |
| `linux-arm64` | Linux ARM64 |

| 平台 | 安裝檔 |
| --- | --- |
| macOS Apple Silicon | `navop-<version>-macos-arm64.dmg`、`navop-<version>-macos-arm64.tar.gz` |
| macOS Intel | `navop-<version>-macos-x64.dmg`、`navop-<version>-macos-x64.tar.gz` |
| Windows x64 | `navop-<version>-windows-x64.msi`、`navop-<version>-windows-x64.exe`、`navop-<version>-windows-x64.zip`、`navop-<version>-windows-x64-portable.zip` |
| Windows 32 位元 | `navop-<version>-win32.*` |
| Linux x64 | `navop-<version>-linux-x64.tar.gz`、`navop-<version>-linux-x64-portable.tar.gz`、`navop_<version>_amd64.deb`、`navop-<version>-1.x86_64.rpm`、`navop_<version>_amd64.AppImage`、`navop-<version>-linux-x64-gpu-stack.tar.gz` |
| Linux ARM64 | `navop-<version>-linux-arm64.tar.gz`、`navop-<version>-linux-arm64-portable.tar.gz`、`navop_<version>_arm64.deb`、`navop-<version>-1.aarch64.rpm`、`navop_<version>_arm64.AppImage`、`navop-<version>-linux-arm64-gpu-stack.tar.gz` |

另外也提供 Scoop 與 Flatpak，見下文。目前版本號以下載中心或 Releases 頁面為準，本文不硬編碼。

## 下載與驗證

優先從以下位址下載：

- 下載中心：[https://navop.dev/zh-TW/extensions](https://navop.dev/zh-TW/extensions)
- GitHub Releases：[https://github.com/feigeCode/navop/releases](https://github.com/feigeCode/navop/releases)
- CNB：[https://cnb.cool/navop-dev/navop/-/releases](https://cnb.cool/navop-dev/navop/-/releases)
- Cloudflare R2 鏡像：`https://navop.pdyyds.cn/releases/<tag>/<file>`

每次正式發佈都附帶 `sha256sums.txt`。下載後先驗證再安裝：

```bash
# Linux
sha256sum -c sha256sums.txt

# macOS
shasum -a 256 -c sha256sums.txt
```

Windows 可以用 `certutil -hashfile <檔案> SHA256` 計算雜湊，再與 `sha256sums.txt` 中對應行比對。只驗證你實際下載的檔案，不要把不同版本的校驗檔混用。

## 首次啟動

啟動後先選擇介面語言、主題和預設啟動頁。SSH、SFTP、網路資料庫和遠端桌面都需要存取區域網路或網際網路；系統彈出本機網路、防火牆或鑰匙圈權限時，依實際連線範圍授權。Notes 目錄、外部編輯器和自訂字型需要相應的檔案系統權限。

建議先建立一個不含正式環境憑證的測試連線，確認 DNS、代理、防火牆和憑證鏈正常。需要連線匯入、遠端桌面 Provider、資料庫擴充或 ACP Agent 時，再從擴充商店安裝對應元件。

## macOS 安裝

1. 下載 `.dmg`（建議）或 `.tar.gz`。
2. 開啟 `.dmg`，把 `Navop.app` 拖入 `/Applications`。
3. 如果從網路下載後提示「已損壞」或無法開啟，移除隔離屬性：

```bash
sudo xattr -rd com.apple.quarantine /Applications/Navop.app
```

使用 `.tar.gz` 時，解壓後同樣把 `.app` 移到 `/Applications`，再視需要執行上面的指令。

首次連線區域網路裝置時，macOS 會要求「本機網路」權限。若之前拒絕過，請在「系統設定 > 隱私權與安全性 > 本機網路」中重新允許 Navop。

## Windows 安裝

- **安裝版**：執行 `.msi` 或 `navop-<version>-windows-x64.exe`（EXE 安裝檔），依精靈完成安裝。`.exe` 內嵌並啟動同一套 MSI 流程，兩者預設都安裝到目前使用者目錄，建立開始功能表與桌面捷徑，使用正常的 Windows 使用者資料目錄，並支援記住主密鑰後自動解鎖；使用預設位置時不需要系統管理員權限。
- **免安裝 ZIP**：解壓 `navop-<version>-windows-x64.zip` 後直接執行目錄中的 `navop.exe`。它不會建立捷徑或檔案關聯，但仍使用正常的 Windows 使用者資料目錄。
- **便攜版 ZIP**：解壓 `...-portable.zip`，資料會寫入解壓目錄下的 `data`，適合放在 USB 隨身碟或受控目錄中使用。不要在一般 ZIP 旁放置標記檔，除非確實想切換到便攜模式。

如果 SmartScreen 提示未知發行者，請先確認檔案來自上面的官方位址並通過 `sha256sums.txt` 驗證，再決定是否繼續。32 位元 Windows 使用 `win32` 目標。

## Linux 安裝

### 便攜版 tar.gz

```bash
tar -xzf navop-<version>-linux-x64.tar.gz
cd navop-<version>-linux-x64
./navop
```

`-portable.tar.gz` 解壓後直接以 `<安裝目錄>/data` 作為資料目錄，不會寫入使用者主目錄。

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

這三種套件隨 Linux x64 與 ARM64 兩個目標發佈，檔名中的架構後綴不同：x64 用 `amd64`（deb / AppImage）與 `x86_64`（rpm），ARM64 用 `arm64`（deb / AppImage）與 `aarch64`（rpm）。

### GPU 圖形相依套件

如果啟動時報 `Failed to create surface: Failed to create surface for any enabled backend`，表示系統缺少可用的圖形後端函式庫。Linux x64 和 arm64 都有對應的 GPU 相依套件：x64 用 `navop-<version>-linux-x64-gpu-stack.tar.gz`，arm64 用 `navop-<version>-linux-arm64-gpu-stack.tar.gz`。下載後解壓並執行套件內的 `install.sh`：

```bash
mkdir navop-gpu-stack
tar -xzf navop-<version>-linux-x64-gpu-stack.tar.gz -C navop-gpu-stack
sudo navop-gpu-stack/install.sh --dry-run   # 先查看將要安裝的相依套件
sudo navop-gpu-stack/install.sh
```

安裝指令碼是增量的：系統已能解析的 SONAME 會跳過，宿主自帶的版本優先；系統已有可用的 EGL 與 DRI 驅動時，整個 Mesa 渲染器也會跳過。`install.sh` 支援 `--dry-run`、`--force` 和 `--uninstall`，安裝記錄保存在 `/usr/lib/navop-gpu-stack/installed.tsv`；解除安裝只刪除指令碼實際寫入的檔案。此套件以 glibc 2.28 為基準。

## Scoop

Scoop 安裝的是便攜形態，`data` 由 Scoop 管理：

```powershell
scoop bucket add extras
scoop install navop
```

後續用 `scoop update navop` 升級。Scoop 安裝與 MSI/EXE 安裝版使用不同的資料目錄，切換安裝方式時請依便攜版說明搬移 `data`。

## Flatpak

```bash
flatpak --user remote-add --if-not-exists flatpark https://dl.flatpark.org/flatpark.flatpakrepo
flatpak --user install flatpark dev.navop.Navop
flatpak run dev.navop.Navop
```

套件名為 `dev.navop.Navop`，應用程式頁面見 [https://flatpark.org/apps/dev.navop.Navop/](https://flatpark.org/apps/dev.navop.Navop/)。Flatpak 在沙盒中執行，SSH、資料庫、SFTP 或遠端桌面等整合可能需要額外授權。

## 便攜版

在可執行檔同目錄放置標記檔 `navop.portable`（內容可以為空），應用程式就會把設定、狀態和快取寫入 `<安裝目錄>/data`。也可以不改檔案，直接使用環境變數或命令列：

```bash
NAVOP_PORTABLE=1 ./navop
NAVOP_DATA_DIR=/path/to/data ./navop
./navop --portable
./navop --data-dir /path/to/data
```

`NAVOP_PORTABLE` 接受 `1`、`true`、`yes`、`on`。資料目錄優先順序為：`--data-dir` > `--portable` > `NAVOP_DATA_DIR` > `NAVOP_PORTABLE` 或標記檔 > 一般使用者目錄。

首次啟動後，便攜目錄結構如下：

```text
<安裝目錄>/
├── navop
├── navop.portable
└── data/
    ├── config/
    ├── state/
    └── cache/
```

便攜目錄必須可寫；放在唯讀位置會導致啟動失敗。便攜模式預設不持久化主密鑰，每次啟動都需要重新輸入；可以在設定中明確選擇把加密副本保存到 `data/state/key_storage`，但這會降低保護強度。

便攜版不註冊檔案關聯，也不支援應用程式內更新和自動更新檢查。升級時先完全結束應用程式，把新版本解壓到新目錄，再把舊目錄的 `data` 整體複製過去，確認連線、擴充和 Notes 正常後再刪除舊目錄。搬移前應備份 `data`，並確保沒有兩個執行個體同時寫入同一份資料。

## 資料目錄與鑰匙圈

| 內容 | macOS | Linux | Windows |
| --- | --- | --- | --- |
| 設定 | `~/.config/navop` | `~/.config/navop` | `%APPDATA%\navop` |
| 資料 | `~/Library/Application Support/navop` | `~/.local/share/navop` | `%APPDATA%\navop` |
| 快取 | `~/Library/Caches/navop` | `~/.cache/navop` | `%APPDATA%\navop` |

憑證保存在系統鑰匙圈或認證儲存中，服務名為 `com.navop.desktop`，bundle id 為 `com.onetcli.app`。解除安裝通常不會自動刪除鑰匙圈項目和資料目錄，需要依下面的步驟單獨清理。

如果同時安裝了 Navop Shell 或 Navop Data，它們分別使用 `navop-shell` / `navop-data` 目錄和獨立的鑰匙圈項目，不會與 Navop 共用連線密文。不要把這幾個目錄合併或互相複製。

## 檔案關聯與命令列開啟

Navop 註冊 `.db`、`.duckdb`、`.md` 和 `.cast` 檔案關聯。雙擊這些檔案會交給 Navop 開啟；便攜模式不註冊系統關聯，需要穩定關聯時請使用安裝版。

命令列可以直接傳入檔案：

```bash
navop ./data.db
navop ./notes.md
```

Windows 使用 `navop.exe`。如果系統仍把關聯指向舊版本，先用安裝檔重裝，或在系統「預設應用程式」中重新選擇。

## 應用程式內更新與回復

安裝版、一般 ZIP 和其他常規安裝形態可以在設定中開啟自動檢查或手動檢查更新。Navop 讀取 `https://navop.pdyyds.cn/updates/latest.json`；如果同時安裝了精簡版，它們各自讀取自己的更新清單，互不影響。

便攜版（包括 Windows `-portable.zip`、Linux `-portable.tar.gz` 和 Scoop 安裝）沒有應用程式內更新，也不會自動檢查更新，需要依「便攜版」一節的步驟手動升級。

回復到舊版本時，下載舊版安裝檔覆蓋安裝即可，連線和設定通常保留在資料目錄中。但如果新版本升級過本機資料結構，舊版本可能無法讀取；回復前先備份資料目錄。

## 解除安裝與保留資料

解除安裝應用程式不會自動刪除連線、設定和歷史記錄。需要徹底清理時，先結束應用程式，再刪除對應目錄。

### macOS

把 `Navop.app` 移到垃圾桶。然後視需要刪除：

```text
~/Library/Application Support/navop
~/Library/Caches/navop
~/.config/navop
```

### Windows

在「設定 > 應用程式」中解除安裝 Navop。安裝版會移除程式檔案，但保留 `%APPDATA%\navop`；便攜版直接刪除整個解壓目錄即可。需要徹底清理時再刪除 `%APPDATA%\navop`。

### Linux

便攜版刪除解壓目錄；deb/rpm 安裝版用套件管理器解除安裝：

```bash
sudo apt remove navop
sudo dnf remove navop
```

AppImage 直接刪除檔案。之後視需要刪除 `~/.local/share/navop`、`~/.config/navop` 和 `~/.cache/navop`。

## 安裝後驗證清單

- [ ] 應用程式名稱與圖示為 Navop。
- [ ] 「關於」頁顯示的版本與下載頁一致。
- [ ] 資料目錄建立為 `navop`，沒有混入其他目錄。
- [ ] 檔案關聯包含 `.db`、`.duckdb`、`.md`、`.cast`。
- [ ] Public MCP 的探索名稱為 `navop`。
- [ ] 更新檢查讀取的是 Navop 的更新清單。
- [ ] 能建立一個測試連線並成功執行「測試連線」。
