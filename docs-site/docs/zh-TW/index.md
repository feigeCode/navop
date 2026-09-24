# Navop 使用說明

Navop 是 AI 時代的開發和運維工作台，將資料庫、Redis、MongoDB、SSH、SFTP、終端、遠端桌面、Notes、AI 和團隊同步放在同一個原生工作區。

## 目前版本：v0.19.1

前往[官網下載中心](https://navop.dev/zh-TW/extensions)下載最新穩定版。

- 修復 Windows 下終端頻繁卡頓的問題（git bash、PowerShell，操作後切換介面即卡、過會才恢復）：存在高亮/搜尋等裝飾時只重建發生變化的行，自訂高亮規則只重掃本幀變化的行，本機終端懸停偵測目錄項目的同步讀盤移到背景執行緒。
- 表格資料支援查找：瀏覽表資料時按 Cmd/Ctrl+F 開啟查找面板，命中儲存格高亮描邊，Cmd/Ctrl+G / Cmd/Ctrl+Shift+G 在命中間跳轉並把所在列橫向滾入視埠，換頁/重新整理後按當前詞重掃；表資料預覽支援欄位過濾隱藏列，寬表只看關心的列。
- SQL 編輯器物件詳情體驗重做：懸停彈詳情預設關閉（設定里可開，開啟後須停駐 600ms 才顯示），改由右鍵選單「查看物件詳情」開啟獨立視窗，內容可選取複製；右鍵新增「複製 DDL」，選取表名右鍵也能解析。
- 新增關閉當前頁籤快捷鍵 Cmd/Ctrl+Shift+W，可在設定里改綁；連線表單「工作區」欄位統一改稱「分組」。
- 修復 SSH MFA 登入把儲存的密碼當作驗證碼應答導致認證失敗的問題；修復老裝置（華為 VRP 系等）SSH 連線報 `` `mpint` encoding invalid `` 失敗的問題。
- 資料庫連線健壯性：複用前 ping 10 秒上限、超時判死並丟棄會話，斷開 5 秒上限、超時強制回收，MySQL/PostgreSQL 連線啟用 30 秒 TCP keepalive，長時間掛機後的死連線不再永久掛起；修復斷連殺死的手動交易卡在死循環。
- 修復 SQLite WITHOUT ROWID 表、視圖預覽整頁空白，PostgreSQL 序列目錄一直為空，MCP 客戶端設定寫入已廢棄參數；預設 HTTP 客戶端改為跟隨系統代理與環境變數代理。
- 打包：Linux 發佈包不再連結 WebKitGTK 4.1，渲染相依獨立成 gpu-stack 包按需安裝（安裝指令碼只補主機缺少的程式库，支援 --dry-run/--uninstall）；HTML 預覽 webview 改為全平台預設關閉，視窗降級提示「用瀏覽器開啟」「下載 HTML」不受影響。

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
