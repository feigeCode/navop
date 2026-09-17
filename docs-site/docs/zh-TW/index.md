# Navop 使用說明

Navop 是 AI 時代的開發和運維工作台，將資料庫、Redis、MongoDB、SSH、SFTP、終端、遠端桌面、Notes、AI 和團隊同步放在同一個原生工作區。

## 目前版本：v0.18.0

前往[官網下載中心](https://navop.dev/zh-TW/extensions)下載最新穩定版。

- 新增 FTP / FTPS 獨立連線類型，並可在同一條 SSH 連線的遠端檔案面板中切換 SFTP / FTP / FTPS；SSH 連線可設定雙擊預設開啟終端或雙欄檔案檢視。
- 新增原生資源工作台：擴充功能宣告集合、表格與操作，宿主以原生介面渲染；首個發布 Docker 工作台，提供引擎總覽、容器與映像管理、日誌、處理程序、檔案系統變更與 exec 終端。
- 本機終端下拉自動識別並列出 WSL 發行版，可依發行版一鍵啟動。
- TDengine 與 MQTT 改為擴充功能提供，內建實作移除，舊資料自動遷移並按需安裝擴充功能。
- Redis 統一使用內嵌 redis-rs 並讓大鍵載入保持記憶體有界；修復鍵樹搜尋不到 key、Shell Integration 握手導致輸入被暫存、MSTSC 游標抖動等問題。
- 關閉主視窗最小化到系統匣，應用程式繼續在背景執行；資源工作台樹支援靜態子項，展開與導覽解耦。
- AI 串流請求改用閒置讀取逾時，長任務不再被固定總逾時中斷，閒置逾時值可設定。

## 從這裡開始

- [快速開始](./guide/quick-start)
- [安裝與更新](./guide/install-update)
- [首頁、工作區與連線管理](./guide/workspace-connections)

## 按任務查找

- [資料庫連線、SQL、匯入匯出與 Schema 工具](./guide/database-connections)
- [SQL 編輯器、交易與查詢結果](./guide/sql-editor)
- [SSH、SFTP、連接埠轉送與 Agent Hub](./guide/ssh-terminal)
- [FTP 與 FTPS 遠端檔案](./guide/ftp-remote-files)
- [遠端桌面、串口與伺服器監控](./guide/remote-access)
- [原生資源工作台（Docker 等）](./guide/resource-workbench)
- [Notes Markdown 預覽與原始碼編輯](./guide/notes)
- [AI 工作台、Navop Skill 與 Public MCP](./guide/ai-workbench)
- [團隊同步與安全](./guide/teams-sync-security)
- [設定與疑難排解](./guide/settings-shortcuts)
