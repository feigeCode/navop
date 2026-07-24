# 安裝、更新與檔案關聯

Navop 提供 macOS、Windows 與 Linux 桌面版本。安裝包必須符合系統與 CPU 架構；更新前先儲存 SQL、Notes 和遠端檔案，結束手動交易並等待傳輸完成。

## 下載與安裝

從 [GitHub Releases](https://github.com/feigeCode/navop/releases) 選擇最新穩定版。macOS 依裝置選 Apple Silicon 或 Intel，將應用程式移入「應用程式」；Windows 執行相符安裝包；Linux 依發佈頁提供的格式安裝。

若 macOS Gatekeeper 阻擋首次啟動，先確認來源為正式發佈頁，再到「隱私權與安全性」允許開啟。Windows 或 Linux 的安全警告也應核對來源，不要關閉全域防護。受管理裝置可能需要管理員核准。

## 安裝包選擇

| 平台 | 裝置/架構 | 建議檔案 | 適用情境 |
| --- | --- | --- | --- |
| macOS | Apple Silicon | Apple Silicon `.dmg` 或 `.tar.gz` | M 系列 Mac |
| macOS | Intel | Intel `.dmg` 或 `.tar.gz` | Intel Mac |
| Windows | x86_64 | `.msi` | 一般安裝、開始功能表、桌面捷徑與穩定的檔案關聯 |
| Windows | x86_64 | `.zip` | 便攜使用或自行管理目錄 |
| Linux | x86_64 | `.deb`、`.rpm`、`.AppImage` 或 `.tar.gz` | 依發行版與桌面環境選擇 |

可使用同一發佈版本中的 `sha256sums.txt` 驗證下載完整性。

## Windows 便攜版

### 解壓縮與首次啟動

官方 Windows `.zip` 是便攜版，壓縮包內含：

```text
navop.exe
navop.portable
```

`navop.portable` 是啟用便攜模式的標記檔案，必須與 `navop.exe` 保持在同一目錄。不要直接在 ZIP 壓縮包內執行，也不建議刪除或重新命名此檔案。請先將壓縮包完整解壓縮到目前使用者可寫入的一般目錄，例如：

```text
D:\Apps\NavopPortable\
├── navop.exe
└── navop.portable
```

接著連按兩下 `navop.exe`，或在 PowerShell 中執行：

```powershell
.\navop.exe
```

首次啟動後，Navop 會在執行檔旁自動建立 `data` 目錄：

```text
D:\Apps\NavopPortable\
├── navop.exe
├── navop.portable
└── data\
    ├── config\
    ├── state\
    └── cache\
```

便攜目錄必須可寫入。不要放在 `Program Files`、唯讀目錄或唯讀移動媒體中；使用 USB 隨身碟或外接磁碟時，也應確認媒體連線穩定且允許寫入。目錄無法寫入時，Navop 會拒絕啟動。

### 資料目錄與主密鑰

便攜版將設定、應用程式狀態和快取分別儲存在 `data/config`、`data/state` 和 `data/cache`，方便連同程式一起搬移或備份。但是，**便攜模式不會在本機持久化主密鑰，每次啟動都必須重新輸入主密鑰**。複製 `data`、重新解壓縮或重裝程式都無法恢復忘記的主密鑰。

請獨立保管主密鑰，不要以明文形式放入便攜目錄或同一個 USB 隨身碟。`data` 中可能包含連線設定、狀態、擴充和快取，不要公開分享、提交到 Git，或放在不可信任的雲端同步目錄中。

### 更新便攜版

便攜模式不支援在應用程式內安裝更新，也不會執行自動更新檢查。仍可手動檢查以得知新版本，但確認更新時會開啟 GitHub Releases，需要自行下載新的 Windows `.zip`。

不要將新版本直接覆蓋到仍在使用的舊目錄。建議依下列步驟升級：

1. 儲存 SQL、Notes 和遠端檔案，提交或回復手動交易，並等待 SFTP、遠端編輯等工作完成。
2. 完全結束 Navop。
3. 備份舊便攜目錄，至少備份完整的 `data` 目錄。
4. 下載符合目前架構的新 Windows `.zip`，解壓縮到新的空目錄。
5. 將舊版本的整個 `data` 目錄複製到新目錄，與新版 `navop.exe` 位於同一層。
6. 確認新目錄中仍有與 `navop.exe` 同一層的 `navop.portable`。
7. 啟動新版本並輸入原主密鑰，檢查版本、連線、擴充、Notes、主題與快捷鍵。
8. 驗證完成後再刪除舊目錄；如需回退，可暫時保留舊目錄副本。

例如：

```text
D:\Apps\
├── NavopPortable-old\
│   ├── navop.exe
│   ├── navop.portable
│   └── data\
└── NavopPortable-new\
    ├── navop.exe
    ├── navop.portable
    └── data\   ← 從舊版本複製
```

### 搬移、檔案關聯與安全

搬移便攜版前應完全結束 Navop，並確認交易、傳輸和遠端編輯工作已完成。通常可以搬移整個便攜目錄，但不要讓兩個 Navop 執行個體或兩台裝置同時寫入同一份 `data`。

便攜模式不會自動註冊 `.db`、`.duckdb` 和 `.md` 的 Windows 系統檔案關聯。仍可從 Navop 內開啟檔案，或使用 Windows「開啟方式」手動選擇 `navop.exe`；若搬移便攜目錄，手動建立的「開啟方式」路徑可能失效。需要穩定檔案關聯、開始功能表/桌面捷徑、應用程式內更新或由作業系統持久化主密鑰時，應選擇 `.msi` 一般安裝版。

移動媒體遺失可能暴露加密資料和相關中繼資料。搬到新裝置後仍需輸入正確主密鑰；驗證新副本可正常使用之前，不要刪除原目錄或備份。

### 進階啟動方式

官方 Windows ZIP 已包含 `navop.portable`，一般使用不需要額外參數。測試或自訂部署時，也可以透過下列方式啟用便攜模式或指定資料目錄：

```powershell
# 暫時啟用便攜模式，預設使用 navop.exe 旁的 data 目錄
.\navop.exe --portable

# 指定資料目錄；此參數本身也會啟用便攜路徑模式
.\navop.exe --data-dir "E:\NavopData"

# 透過環境變數啟用便攜模式
$env:NAVOP_PORTABLE = "1"
.\navop.exe

# 透過環境變數指定資料目錄
$env:NAVOP_DATA_DIR = "E:\NavopData"
.\navop.exe
```

`NAVOP_PORTABLE` 支援 `1`、`true`、`yes` 或 `on`。資料目錄的選擇優先順序為 `--data-dir`、`--portable`、`NAVOP_DATA_DIR`、`NAVOP_PORTABLE`/`navop.portable`，最後才是一般安裝模式。指定的資料目錄必須可寫入；建議使用絕對路徑，因為相對路徑會按照啟動 Navop 時的目前工作目錄解析。

## 首次啟動與權限

選擇語言、主題和啟動頁。資料庫、SSH、SFTP 與遠端桌面需要網路權限；本機網路、防火牆、鑰匙圈或檔案權限只按實際需求授予。Notes 目錄、外部編輯器和自訂字型需要各自的檔案存取權。

先建立不含正式憑證的測試連線。只有需要資料庫驅動、遠端桌面 Provider、匯入器或 ACP Agent 時才安裝相應擴充。

## 應用程式內更新與回退

一般安裝版可在設定開啟自動檢查，或手動檢查更新。更新前關閉活動連線，提交或回復手動交易並完成 SFTP 傳輸；重新啟動後測試重要連線、擴充與快捷鍵。Windows `.zip` 便攜版請依照上方的獨立更新流程手動升級。

若新版本與關鍵擴充不相容，先備份 Navop 資料目錄，再從 Releases 安裝上一個穩定版。降版不能取代備份，因本機設定格式可能已變更。

## 系統檔案關聯

Navop 支援透過系統開啟 `.db`、`.duckdb` 和 `.md`。資料庫檔案會建立或開啟本機 SQLite/DuckDB 連線，Markdown 會進入 Notes。若未自動關聯，可在「開啟方式」選擇 Navop 並設為預設。

不要直接開啟仍被其他程式寫入的正式資料庫檔案，應先複製副本。外部 Markdown 的圖片與連結仍相對於原目錄，移動後需檢查資源路徑。

## 解除安裝與資料保留

移除應用程式本體通常不會自動刪除設定、加密連線、Notes 或擴充快取。重裝時保留資料目錄；徹底移除前先匯出必要資料、停止同步並完成團隊交接。

重裝無法恢復忘記的主密鑰。刪除本機資料前，確認主密鑰與團隊密鑰的安全備份狀態。
