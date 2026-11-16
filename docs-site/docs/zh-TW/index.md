# Navop 使用說明

Navop 是一個**以 Rust 與 GPUI 原生建構**的桌面工作台：沒有 WebView，也沒有 Electron，介面、終端、表格和圖表都由 GPU 加速繪製。它把資料庫、伺服器、遠端桌面和 AI Agent 放進同一個視窗，涵蓋從連線管理、SQL 開發、檔案傳輸到遠端排查的日常工作。

## 為什麼是 Navop

- **原生效能**：GPUI 直接繪製介面，多連線、終端分屏和大結果集不會因為瀏覽器版面配置而變慢。
- **全端 Rust**：資料庫驅動、SSH/SFTP、終端模擬、AI 執行環境與介面共用同一套 Rust 程式碼。
- **一個視窗完成工作**：SQL 工作區、終端、檔案管理、遠端桌面和 AI 側邊欄可以並排組織，不必在多個工具之間切換。
- **可擴充**：資料庫驅動、遠端桌面、文件轉譯和資源工作台都可以透過 WASM 擴充安裝，無需重啟應用程式。
- **跨平台一致**：macOS（arm64/x64）、Windows（x64/win32）和 Linux（x64/arm64）使用同一套介面與能力。

## Navop 支援什麼

- **資料庫**：MySQL、PostgreSQL、MariaDB、SQLite、DuckDB、SQL Server、Oracle、ClickHouse、TDengine，以及達夢、金倉、GBase、OceanBase、openGauss、IoTDB、Oscar 等擴充驅動。
- **資料工具**：物件瀏覽、SQL 編輯器與執行計畫、匯入匯出、結構與資料比較、ER 圖、表資料批次編輯。
- **Redis / MongoDB / MQTT**：Key 與值管理、集合與文件查詢、訂閱與發佈。
- **SSH 與終端**：本機/遠端終端、原生分屏、廣播輸入、快速命令、Shell 整合、工作階段錄製回放與日誌搜尋。
- **檔案傳輸**：SFTP、FTP、FTPS，拖曳上傳下載、遠端檔案編輯、ZMODEM、伺服器間複製。
- **遠端存取**：本機/遠端/SOCKS 連接埠轉送、X11、RDP、VNC、串口、Telnet、主機金鑰管理與 SecureCRT 匯入。
- **AI 與自動化**：SQL 產生與解釋、結果分析、圖表、終端輔助、ACP Agent（Codex / Claude Code / OpenCode）、Agent Hub。
- **協作與擴充**：Notes、團隊與共享連線、加密同步、擴充市場、Public MCP、Navop CLI 與 Agent Skill。

## 安裝與版本

從[官網下載中心](https://navop.dev/zh-TW/extensions)、[GitHub Releases](https://github.com/feigeCode/navop/releases) 或 [CNB](https://cnb.cool/navop-dev/navop/-/releases) 取得最新安裝包。安裝包命名遵循 `navop-<version>-<label>.<ext>`，支援 macOS、Windows 和 Linux；每次發佈都附帶校驗檔案。

同一份原始碼還會建構兩個更小的版本：**Navop Shell**（SSH、終端與遠端桌面）和 **Navop Data**（資料庫與資料瀏覽）。它們與 Navop 共用原生外殼、AI、同步與擴充體系，適合只需要部分能力的場景；大多數使用者直接使用 Navop 即可。

安裝包矩陣、平台差異與升級說明見[安裝與更新](./guide/install-update)。

## 從這裡開始

- [快速開始：完成第一個連線與任務](./guide/quick-start)
- [安裝與更新：安裝包、平台差異與資料目錄](./guide/install-update)
- [首頁、工作區與連線管理](./guide/workspace-connections)

## 按任務查找

- 資料庫連線、SQL、匯入匯出與 Schema 工具：[資料庫連線](./guide/database-connections)
- SQL 編輯器、交易與查詢結果：[SQL 編輯器](./guide/sql-editor)
- SSH、SFTP、連接埠轉送與 Agent Hub：[SSH 與終端](./guide/ssh-terminal)
- FTP 與 FTPS 遠端檔案：[FTP 與遠端檔案](./guide/ftp-remote-files)
- 遠端桌面、串口與伺服器監控：[遠端存取](./guide/remote-access)
- 原生資源工作台（Docker 等）：[資源工作台](./guide/resource-workbench)
- Notes Markdown 預覽與原始碼編輯：[Notes](./guide/notes)
- AI 工作台、Navop Skill 與 Public MCP：[AI 與 MCP](./guide/ai-workbench)
- 團隊同步與安全：[團隊同步](./guide/teams-sync-security)
- 設定與疑難排解：[設定與疑難排解](./guide/settings-shortcuts)

本文件隨 Navop 持續更新。涉及正式資料庫、遠端伺服器、寫入 SQL、檔案覆寫和批次同步時，請先確認目前環境和操作範圍。
