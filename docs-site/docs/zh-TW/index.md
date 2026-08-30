# Navop 使用說明

Navop 是 AI 時代的開發和運維工作台，將資料庫、Redis、MongoDB、SSH、SFTP、終端、遠端桌面、Notes、AI 和團隊同步放在同一個原生工作區。

## 目前版本：v0.15.1

前往[官網下載中心](https://navop.dev/zh-TW/extensions)下載最新穩定版。

- 終端新增「選取文字後高亮相同內容」：選取一段文字後，可見區域內相同文字會以淡色背景高亮，SSH 與本機終端同時生效，可在終端側邊欄設定中開關（預設開啟）。
- 連線清單寬度支援持久化：拖曳調整側邊欄連線樹寬度後自動儲存，重新啟動應用恢復上次寬度；停靠模式側邊欄與主視窗背景統一，浮動模式改為浮層卡片樣式（圓角 + 陰影）。
- 「自動檢查更新」開關與「檢查更新」按鈕從通用設定頁遷移到關於頁面，與版本資訊同頁展示。
- 修復側邊欄與命令列圖示按鈕在終端/Agent 自訂主題下顏色不跟隨、誤顯示為黑色的問題。
- 修復 SFTP 覆寫遠端檔案時恢復舊修改時間（mtime），導致 rsync 部署、Web/應用快取與增量建置等基於 mtime 的變更檢測誤判檔案未更新的問題。

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
