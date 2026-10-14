-- 快捷命令：区分「内置默认」与「用户自建」
--
-- 背景：`20260720000001_default_quick_commands.sql` 把 30 条内置命令直接 INSERT 进
-- `quick_commands` 业务表，表上没有可区分来源的列。云同步若按整表同步，每台设备都会
-- 把自己那份默认副本推上云端、再拉回其他设备，造成「同步一次多 30 条」的指数级重复。
--
-- 本迁移补齐 `is_builtin` 列并回填，同步侧只处理 `is_builtin = 0` 的行。
--
-- 判定逻辑复用原迁移的比对方式：`connection_id IS NULL` 且
-- `LOWER(TRIM(command))` 命中内置命令文本。代价是「用户自建了一条与内置完全同文本的
-- 命令」时会被误标为内置从而不参与同步 —— 这个代价远小于重复污染。

ALTER TABLE quick_commands ADD COLUMN is_builtin INTEGER NOT NULL DEFAULT 0;

UPDATE quick_commands
SET is_builtin = 1
WHERE connection_id IS NULL
  AND LOWER(TRIM(command)) IN (
      'df -h',
      'free -h',
      'uptime',
      'whoami',
      'uname -a',
      'ps aux',
      'env | sort',
      'journalctl -xe --no-pager',
      'ip addr',
      'ifconfig',
      'ss -lntup',
      'netstat -an',
      'ping -c 4 8.8.8.8',
      'curl -i https://example.com',
      'nslookup example.com',
      'pwd',
      'ls -la',
      'du -sh .',
      'du -sh * | sort -h',
      'find . -maxdepth 2 -type f',
      'git status',
      'git log --oneline --graph --decorate -20',
      'git branch -vv',
      'git diff --stat',
      'git remote -v',
      'docker ps',
      'docker images',
      'docker compose ps',
      'docker stats --no-stream',
      'docker system df'
  );

CREATE INDEX IF NOT EXISTS idx_quick_commands_is_builtin ON quick_commands (is_builtin);
