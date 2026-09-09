# Navop 使用說明

Navop 是 AI 時代的開發和運維工作台，將資料庫、Redis、MongoDB、SSH、SFTP、終端、遠端桌面、Notes、AI 和團隊同步放在同一個原生工作區。

## 目前版本：v0.17.0

前往[官網下載中心](https://navop.dev/zh-TW/extensions)下載最新穩定版。

- 新增「已知主機」頁面：集中查看應用信任的 SSH 主機的金鑰演算法與指紋，支援複製主機識別、刪除可信主機，並可掃描系統 `known_hosts` 匯入。
- 最佳化主頁連線卡片、連線樹與帳戶入口版面配置，視窗空間利用更合理。
- 設定頁支援清除捷徑並停用系統捷徑。
- 最佳化 AI 對話中的執行中活動顯示；修復標籤列導覽切換插槽與工作區排序還原問題。
- Windows 現可透過 Scoop 安裝：`scoop bucket add extras && scoop install navop`。

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
