# Navop 使用說明

Navop 是 AI 時代的開發和運維工作台，將資料庫、Redis、MongoDB、SSH、SFTP、終端、遠端桌面、Notes、AI 和團隊同步放在同一個原生工作區。

## 目前版本：v0.11.0

[下載 Navop v0.11.0](https://github.com/feigeCode/navop/releases/tag/v0.11.0)

- 連線列表新增「連線排序」設定（設定 → 一般 → 連線顯示），預設依名稱自然排序（IP 等數字段以數值比較、忽略大小寫），也可切換為「最近使用優先」；首頁連線列表、Redis/MongoDB 工作區標籤頁與常駐側欄連線樹統一套用該設定。
- SSH 新增可選的舊版演算法相容支援，可連線僅支援 DSA 主機金鑰、SHA-1 金鑰交換/MAC 或 1024 位元 DH 組協商的舊設備。
- 複製標籤頁自動追加編號並重複使用已釋放的編號，標籤寬度隨內容自適應，長標題不再被截斷。

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
