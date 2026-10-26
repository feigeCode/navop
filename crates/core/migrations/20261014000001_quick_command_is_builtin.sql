-- 快捷命令：区分「内置默认」与「用户自建」
--
-- 背景：`20260720000001_default_quick_commands.sql` 把 30 条内置命令直接 INSERT 进
-- `quick_commands` 业务表，表上没有可区分来源的列。云同步若按整表同步，每台设备都会
-- 把自己那份默认副本推上云端、再拉回其他设备，造成「同步一次多 30 条」的指数级重复。
--
-- 本迁移补齐 `is_builtin` 列并回填，同步侧只处理 `is_builtin = 0` 的行。
--
-- 判定逻辑在「命令文本命中」之外收紧两点：
-- 1. `LOWER(TRIM(name))` 必须命中内置命令名（原迁移插入的 name，见
--    `20260720000001` 的 defaults CTE）——用户自建一条同文本但不同名的命令不再被误标；
-- 2. 排除 `pinned = 1` 的行——内置默认全部未置顶，用户置顶过的命令视为用户数据。
-- 代价是「用户自建了与内置完全同名同文本的命令」时仍会被误标，这个代价远小于重复污染。
--
-- 注意：`execute_batch` 中 ALTER 若因列已存在报错，整批（含回填）都不会执行。
-- 迁移执行器对 "duplicate column name" 有专门的吞错路径，因此不能依赖把回填与 ALTER
-- 放同一批来保证幂等——回填语句本身用 WHERE 条件限定即可重入。

ALTER TABLE quick_commands ADD COLUMN is_builtin INTEGER NOT NULL DEFAULT 0;

UPDATE quick_commands
SET is_builtin = 1
WHERE connection_id IS NULL
  AND pinned = 0
  AND LOWER(TRIM(name)) IN (
      'disk usage',
      'memory usage',
      'uptime',
      'current user',
      'system information',
      'running processes',
      'environment variables',
      'recent journal errors',
      'ip addresses',
      'interface config',
      'listening ports',
      'network connections',
      'ping public dns',
      'http response headers',
      'dns lookup',
      'print working directory',
      'list files',
      'directory size',
      'child sizes',
      'find files',
      'git status',
      'recent git history',
      'git branches',
      'git diff summary',
      'git remotes',
      'docker containers',
      'docker images',
      'compose services',
      'docker stats',
      'docker disk usage'
  )
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
