# Public MCP 與外部自動化

Public MCP 讓 Codex、Claude Desktop、Claude Code 與相容用戶端呼叫目前正在執行的 Navop。它不是固定雲端 API：使用動態 loopback 連接埠與僅目前使用者可讀的 discovery token，真實工具和 Schema 來自本機應用程式。

## 服務模式與探索

Temporary 適合短期工作，任務或應用程式結束後不應依賴它持續存在；Persistent 適合需要長期本機探索的用戶端。兩者都應只監聽 loopback，並使用受保護的使用者級探索資料。

不要把 discovery 檔、Token 或連接埠公開到網際網路。切換模式、重新啟動 Navop 或變更 Tool Exposure 可能重啟 endpoint，MCP/ACP 用戶端需重新連線。

## 權限檔位與 Tool Exposure

Safe、Confirm 和 Auto 控制審批強度。首次使用選 Safe 或 Confirm；只有任務、目標與工具高度可控時才使用 Auto。這些設定不會取代資料庫或伺服器權限。

Tool Exposure 可分別開放 Terminal、SSH Exec、可見終端、Connections、SFTP、Redis、MongoDB、Database 和內部函式。只啟用當前用戶端需要的群組，任務結束後關閉。

## 安裝用戶端依賴

橋接需要 Node.js 20+ 和可用的 `npx`。先確認版本，再從 Navop 設定複製 Codex、Claude Desktop、Claude Code 或通用 MCP JSON。依各用戶端的路徑和重啟方式完成設定。

Navop 也可安裝或更新供 Codex 與 Agents 使用的 Skill。npm 套件不是靜態工具 registry，用戶端仍需連接執行中的 Navop 才能取得即時工具。

## 使用 @navop/mcp CLI

`@navop/mcp` CLI 提供 `status`、`tools`、`schema`、`call` 和 `mcp`。status 檢查探索，tools 列出目前工具，schema 讀取即時參數，call 明確呼叫工具，mcp 提供橋接。

執行 `npx @navop/mcp ...` 前確認套件來源和版本。資源 ID 與參數必須來自當前 tools/schema，不得猜測、重用其他裝置 ID 或試圖繞過審批。

## 審批與排查

審批視窗會顯示用戶端、操作、資源和參數。非預期請求應拒絕並修正用戶端，不要全面放寬權限。ACP 授權不代表 Public MCP 自動允許。

失敗時檢查 Navop 是否執行、服務模式、Node.js、用戶端設定、discovery 權限、Tool Exposure 和即時 Schema。endpoint 變更後重新連線，分享日誌前遮蔽 Token、路徑和業務參數。
