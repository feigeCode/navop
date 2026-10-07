# Navop 使用說明

Navop 是 AI 時代的開發和運維工作台，將資料庫、Redis、MongoDB、SSH、SFTP、終端、遠端桌面、Notes、AI 和團隊同步放在同一個原生工作區。

## 目前版本：v0.19.5

前往[官網下載中心](https://navop.dev/zh-TW/extensions)下載最新穩定版。

- 個人同步新增 WebDAV 後端：與「資料夾」「Git」並列，填伺服器位址、使用者名稱、密碼即可用，密碼加密後落盤、不以明文保存。協定只用 GET / PUT / DELETE 加 Basic 認證，不做 PROPFIND 探測，以相容堅果雲、群暉、Nextcloud 與自建服務；首次寫入用 MKCOL 建立目錄，伺服器回傳 409 會提示「目錄不可用」而非誤報版本衝突。設定頁只顯示目前後端的項目，密碼框可切換明文/遮罩，WebDAV 沒有本機目錄可監聽，同步由 60 秒週期掃描驅動。
- SSH Agent 轉送：憑證中的私鑰可勾選「透過 ssh-agent 轉送此密鑰」，連線側也有「SSH Agent 轉送」（ForwardAgent）開關。本機 ssh-agent 轉送給遠端後，遠端（例如跳板機）能代表你以本機私鑰向更內層主機認證，對新開啟的終端工作階段生效。
- 表結構設計頁新增「重新整理表結構」：重新讀取最新欄位、索引與資料表資訊；設計器中有未儲存變更時會先提示，確認後「放棄並重新整理」。
- 終端貼上確認視窗補上「開啟設定」與「不再提示」：多行貼上、高危命令、大段貼上三類提示都能直接跳到對應設定項；大段貼上是硬閾值，不提供「不再提示」。
- 外部驅動改為依呼叫類別區分請求逾時，並支援連線層級覆寫：查詢、執行、游標、匯入匯出等使用者操作預設 30 分鐘，中繼資料與結構瀏覽維持 30 秒；連線進階設定新增「請求逾時（秒）」（留空依類別預設，0 表示不限制），慢庫上的大查詢、大匯入不再被統一的 30 秒硬逾時打斷。
- SQL 編輯器手動交易的提交/回復按鈕、表資料頁的「提交變更」在寫入期間顯示 loading，能看出哪一步還在跑，另一個按鈕只停用、不跟著轉。
- 修復 WHERE / ORDER BY 過濾輸入框輸入中文導致崩潰（#326），過濾列輸入框比遷移前高出一行的問題也已修正（40px → 20px）；修復 AI 對話取消回答後工作階段被誤判為失敗、達夢等方言改完欄位註解仍顯示舊註解。
- 修復 Linux（Arch + Hyprland / Wayland）應用內退出以 SIGABRT 收尾並留下 core（#336）；macOS Touch Bar 機型關閉視窗閃退交由上游 zed#65186 修復，撤銷「關閉即隱藏」實驗開關，macOS 兩個架構、Windows、Linux 統一回到「關閉即銷毀」。

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
