# Navop 使用說明

Navop 現在是同一套原生核心上的三個應用程式：完整版的 **Navop**、面向遠端維運的 **Navop Shell**，以及面向資料工作的 **Navop Data**。三者以同一個版本號發佈，可以並排安裝，共用 AI、Notes、團隊同步、擴充市場和主題等基礎，差別在於涵蓋的工作範圍。

## 三個應用程式

| 應用程式 | 適合誰 | 包含的能力 |
| --- | --- | --- |
| **Navop**（完整版） | 資料庫和伺服器都要管，希望一個應用程式涵蓋全部工作 | 資料庫（PostgreSQL、MySQL、MariaDB、SQLite、SQL Server、Oracle、ClickHouse、達夢、人大金倉、OceanBase、OpenGauss 等）、Redis、MongoDB、SSH/SFTP、本機與遠端終端、連接埠轉送、遠端桌面 RDP/VNC、串口/Telnet、SQL 編輯器、ER 圖、資料移轉、監控、Notes、AI 工作台、團隊同步、擴充市場、Public MCP |
| **Navop Shell**（遠端維運） | 主要工作是登入伺服器、傳檔案、開遠端桌面 | SSH/SFTP、本機與遠端終端、連接埠轉送、遠端桌面 RDP/VNC、串口/Telnet；共用 AI、Notes、團隊與擴充體系 |
| **Navop Data**（資料工具） | 主要工作是連資料庫、寫 SQL、看 Redis/MongoDB | SQL 工作區、Redis、MongoDB、ER 圖、資料移轉；共用 AI、Notes、團隊與擴充體系 |

## 共用基礎

三個應用程式使用同一套原生核心，並共用以下能力：

- AI 工作台、Notes、團隊同步與個人同步；
- 擴充市場，以及已安裝的擴充體系；
- 主題、快速鍵、tab 還原、系統匣、單一執行個體、自動更新、Public MCP。

三者以同一個版本號發佈，可以並排安裝；資料目錄與鑰匙圈各自獨立，同時安裝不會互相覆蓋設定。只有完整版 Navop 註冊 `.db`、`.duckdb`、`.md`、`.cast` 檔案關聯；Scoop 與 Flatpak 目前也只涵蓋完整版 Navop。

## 怎麼選

- **資料庫和伺服器都要管**，或需要檔案關聯、監控與資料移轉：選 Navop。
- **只做遠端維運**，日常是 SSH、SFTP、終端、連接埠轉送和遠端桌面：選 Navop Shell。
- **只做資料工作**，日常是 SQL、Redis、MongoDB、ER 圖和資料移轉：選 Navop Data。
- **不確定**：先裝 Navop，它的能力是三者中最完整的；之後仍可以並排安裝獨立產品，兩者互不衝突。

## 目前版本：v0.23.0

三個應用程式目前同為 v0.23.0。可以從[官網下載中心](https://navop.dev/zh-TW/extensions)、[GitHub Releases](https://github.com/feigeCode/navop/releases) 或 [CNB](https://cnb.cool/navop-dev/navop/-/releases) 取得安裝包。產品形態上的主要變化：

- Navop 從單一完整應用程式擴展為同一套原生核心上的三個應用程式，共用 AI、Notes、團隊、擴充與更新體系。
- 安裝包以 `navop-`、`navop-shell-`、`navop-data-` 前綴區分，三者同版本發佈，可以並排安裝。
- 資料庫與遠端維運能力依產品裁剪；資料目錄、鑰匙圈與應用程式內更新通道各自獨立。
- 只有完整版 Navop 提供 `.db`、`.duckdb`、`.md`、`.cast` 檔案關聯，以及 Linux 的 `.deb`、`.rpm`、AppImage 和圖形相依套件。

安裝包矩陣與平台差異見[安裝與更新](./guide/install-update)。

## 從這裡開始

- [快速開始：先選應用程式，再完成第一項任務](./guide/quick-start)
- [安裝與更新：安裝包、平台差異與並排安裝](./guide/install-update)
- [首頁、工作區與連線管理](./guide/workspace-connections)（三個應用程式）

## 按任務查找

- 資料庫連線、SQL、匯入匯出與 Schema 工具（Navop / Navop Data）：[資料庫連線](./guide/database-connections)
- SQL 編輯器、交易與查詢結果（Navop / Navop Data）：[SQL 編輯器](./guide/sql-editor)
- SSH、SFTP、連接埠轉送與 Agent Hub（Navop / Navop Shell）：[SSH 與終端](./guide/ssh-terminal)
- FTP 與 FTPS 遠端檔案（Navop）：[FTP 與遠端檔案](./guide/ftp-remote-files)
- 遠端桌面、串口與伺服器監控（遠端桌面與串口：Navop / Navop Shell；伺服器監控：Navop）：[遠端存取](./guide/remote-access)
- 原生資源工作台（Docker 等，Navop）：[資源工作台](./guide/resource-workbench)
- Notes Markdown 預覽與原始碼編輯（三個應用程式）：[Notes](./guide/notes)
- AI 工作台、Navop Skill 與 Public MCP（三個應用程式，Public MCP 依產品分別探索）：[AI 與 MCP](./guide/ai-workbench)
- 團隊同步與安全（三個應用程式）：[團隊同步](./guide/teams-sync-security)
- 設定與疑難排解（三個應用程式）：[設定與疑難排解](./guide/settings-shortcuts)

本文件隨三個應用程式持續更新。涉及正式資料庫、遠端伺服器、寫入 SQL、檔案覆寫和批次同步時，請先確認目前環境和操作範圍。
