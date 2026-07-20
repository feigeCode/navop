# Notes 本機知識工作區

Notes 在你選擇的本機目錄管理 Markdown、富文字和白板，讓查詢記錄、維運步驟和專案資料留在連線工作旁。檔案仍由你管理，Navop 不能取代備份與版本控制。

<div class="notes-screenshot-grid">
  <figure><img src="/images/markdown.png" alt="Markdown Notes 工作區"><figcaption>Markdown 語法、受限 HTML 與表格圖片</figcaption></figure>
  <figure><img src="/images/richtext.png" alt="富文字 Notes 工作區"><figcaption>富文字區塊編輯與嵌入內容</figcaption></figure>
  <figure><img src="/images/whiteboard.png" alt="Notes 白板工作區"><figcaption>白板畫布與自由排版</figcaption></figure>
</div>

## 選擇工作目錄

首次進入時選擇可讀寫的資料夾，也可在設定更換 Notes 路徑。不同專案使用清楚的目錄；更換設定不會自動搬移舊檔案。雲端硬碟、網路磁碟和 Git 目錄的同步衝突與鎖定仍由相應工具處理。

不要把資料庫密碼、SSH 私鑰、主密鑰或 Token 直接寫入筆記。跨裝置分享前檢查目錄本身的權限。

## Markdown、富文字與白板

Markdown 適合技術說明和程式碼，富文字適合快速排版，白板適合自由整理圖形與想法。Markdown 文件檢視支援標準語法、受限 HTML 安全渲染，以及 Markdown 表格內圖片顯示；它不是不受限制的瀏覽器渲染環境。長期內容應選擇可維護格式，進階效果在不同渲染器間可能不同。

系統開啟 `.md` 時會進入 Notes Markdown 編輯器。相對圖片與媒體資源會以目前 Markdown 檔案所在目錄解析。含白板的富文字文件轉成 Markdown 時，Navop 會匯出白板預覽圖，不會把可編輯白板來源資料寫入 Markdown；若仍需編輯白板，請保留原始文件與完整資源目錄。

## 編輯、儲存與快捷鍵

常用輸入、選取、搜尋和儲存快捷鍵可在「設定 → 快捷鍵 → Notes」查看。關閉未儲存內容時注意提示；自動儲存依設定與格式而異，重要變更仍應主動儲存並保留版本。

貼上富文字後檢查結構與外部資源，圖片、連結和程式碼素材使用穩定的相對路徑。

## 匯出 HTML、PDF 與 Word

安裝 Notes 文件匯出擴充後，可把目前文件匯出為自包含 HTML、PDF 或 Word DOCX。Rust WASM 匯出器不需要本機 Chrome、Office 或宿主轉換函式庫；匯出前確認目錄與檔名，並在目標環境檢查圖片、程式碼、Mermaid 和數學公式。

## 資源與外部修改

圖片和附件通常位於筆記目錄或資源區。移動、重新命名或刪除會破壞引用，整理前先搜尋使用位置。不要把超大二進位檔長期堆在普通筆記中。

其他編輯器修改檔案後，重新載入前比較本機未儲存內容。Git、雲端衝突副本和編碼問題必須在檔案層解決。

## 備份與分享

備份整個 Notes 目錄，包括隱藏資料和資源。Markdown 通常可攜，富文字、白板和擴充語法可能依賴 Navop 或文件渲染器；分享前在目標環境預覽。

對外傳送前遮蔽連線名稱、內網位址、查詢結果、日誌、畫面中繼資料和個人資訊。
