# Navop 使用說明

Navop 是 AI 時代的開發和運維工作台，將資料庫、Redis、MongoDB、SSH、SFTP、終端、遠端桌面、Notes、AI 和團隊同步放在同一個原生工作區。

## 目前版本：v0.14.0

前往[官網下載中心](https://navop.dev/zh-TW/extensions)下載最新穩定版。

- SQL 編輯器新增跨資料庫/跨 Schema 限定名補全（惰性載入），並最佳化 FROM 子句的資料庫提示、選中資料庫限定符建議與限定符元資料作用域隔離。
- SQL 格式化支援保留關鍵字大小寫，新增格式化設定（關鍵字大小寫、縮排）與即時預覽，並透過模板遮罩避免範例程式碼/佔位符被誤格式化。
- 終端將連線狀態與驗證提示內嵌顯示，不再以彈窗打斷操作；背景任務對話框重構為帶計數篩選頁籤，檔案操作分組展示更清晰。
- SSH 跳板機設定在停用後仍保留，便於快速重新啟用；SFTP 左側遠端面板遵循設定的 SFTP 初始目錄。
- 擴充市場頁支援「有更新」篩選，更新通知跳轉只顯示可更新擴充，並移除 MCP 助手分類。

## 從這裡開始

- [快速開始](./guide/quick-start)
- [安裝與更新](./guide/install-update)
- [首頁、工作區與連線管理](./guide/workspace-connections)

## 按任務查找

- [資料庫連線、SQL、匯入匯出與 Schema 工具](./guide/database-connections)
- [SQL 編輯器、交易與查詢結果](./guide/sql-editor)
- [SSH、SFTP、連接埠轉送與 Agent Hub](./guide/ssh-terminal)
- [遠端桌面、串口與伺服器監控](./guide/remote-access)
- [Notes Markdown 預覽與原始碼編輯](./guide/notes)
- [AI 工作台、Navop Skill 與 Public MCP](./guide/ai-workbench)
- [團隊同步與安全](./guide/teams-sync-security)
- [設定與疑難排解](./guide/settings-shortcuts)
