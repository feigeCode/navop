-- 连接的默认打开方式（终端 / 双栏文件视图）。
-- NULL = 按类型默认：SSH 终端、FTP 双栏。
ALTER TABLE connections ADD COLUMN preferred_open_mode TEXT;
