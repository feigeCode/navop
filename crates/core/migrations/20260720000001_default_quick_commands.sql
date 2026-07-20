WITH defaults(name, command, description, group_name, group_color, sort_order) AS (
    VALUES
        ('Disk Usage', 'df -h', 'Show mounted filesystem usage.', 'System', 'blue', 10),
        ('Memory Usage', 'free -h', 'Show memory usage.', 'System', 'blue', 20),
        ('Uptime', 'uptime', 'Show uptime and load average.', 'System', 'blue', 30),
        ('Current User', 'whoami', 'Show the current user.', 'System', 'blue', 40),
        ('System Information', 'uname -a', 'Show kernel and system information.', 'System', 'blue', 50),
        ('Running Processes', 'ps aux', 'List running processes.', 'System', 'blue', 60),
        ('Environment Variables', 'env | sort', 'List environment variables in name order.', 'System', 'blue', 70),
        ('Recent Journal Errors', 'journalctl -xe --no-pager', 'Show recent system journal errors.', 'System', 'blue', 80),

        ('IP Addresses', 'ip addr', 'Show network interface addresses.', 'Network', 'cyan', 110),
        ('Interface Config', 'ifconfig', 'Show network interfaces on systems without iproute2.', 'Network', 'cyan', 120),
        ('Listening Ports', 'ss -lntup', 'Show listening TCP and UDP sockets.', 'Network', 'cyan', 130),
        ('Network Connections', 'netstat -an', 'Show active network connections and listeners.', 'Network', 'cyan', 140),
        ('Ping Public DNS', 'ping -c 4 8.8.8.8', 'Check basic network reachability.', 'Network', 'cyan', 150),
        ('HTTP Response Headers', 'curl -I https://example.com', 'Fetch HTTP response headers.', 'Network', 'cyan', 160),
        ('DNS Lookup', 'nslookup example.com', 'Resolve a domain name through DNS.', 'Network', 'cyan', 170),

        ('Print Working Directory', 'pwd', 'Show the current directory.', 'Files', 'green', 210),
        ('List Files', 'ls -la', 'List files with details.', 'Files', 'green', 220),
        ('Directory Size', 'du -sh .', 'Show the total size of the current directory.', 'Files', 'green', 230),
        ('Child Sizes', 'du -sh * | sort -h', 'Show child sizes ordered from small to large.', 'Files', 'green', 240),
        ('Find Files', 'find . -maxdepth 2 -type f', 'List files up to two levels below the current directory.', 'Files', 'green', 250),

        ('Git Status', 'git status', 'Show repository status.', 'Git', 'purple', 310),
        ('Recent Git History', 'git log --oneline --graph --decorate -20', 'Show a compact graph of recent commits.', 'Git', 'purple', 320),
        ('Git Branches', 'git branch -vv', 'Show local branches and upstream tracking.', 'Git', 'purple', 330),
        ('Git Diff Summary', 'git diff --stat', 'Show a summary of unstaged changes.', 'Git', 'purple', 340),
        ('Git Remotes', 'git remote -v', 'Show configured repository remotes.', 'Git', 'purple', 350),

        ('Docker Containers', 'docker ps', 'List running containers.', 'Docker', 'orange', 410),
        ('Docker Images', 'docker images', 'List local container images.', 'Docker', 'orange', 420),
        ('Compose Services', 'docker compose ps', 'Show Docker Compose service status.', 'Docker', 'orange', 430),
        ('Docker Stats', 'docker stats --no-stream', 'Show a one-time container resource snapshot.', 'Docker', 'orange', 440),
        ('Docker Disk Usage', 'docker system df', 'Show Docker disk usage.', 'Docker', 'orange', 450)
)
INSERT INTO quick_commands (
    name,
    command,
    description,
    pinned,
    sort_order,
    connection_id,
    created_at,
    updated_at,
    group_name,
    group_color
)
SELECT
    defaults.name,
    defaults.command,
    defaults.description,
    0,
    defaults.sort_order,
    NULL,
    CAST(strftime('%s', 'now') AS INTEGER),
    CAST(strftime('%s', 'now') AS INTEGER),
    defaults.group_name,
    defaults.group_color
FROM defaults
WHERE NOT EXISTS (
    SELECT 1
    FROM quick_commands existing
    WHERE existing.connection_id IS NULL
      AND LOWER(TRIM(existing.command)) = LOWER(defaults.command)
);
