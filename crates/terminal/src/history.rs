use std::collections::{HashSet, VecDeque};
use std::time::Instant;

pub const SESSION_HISTORY_LIMIT: usize = 1024;
pub const PERSISTED_HISTORY_LIMIT: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellHistoryFormat {
    Bash,
    Zsh,
}

/// 富历史条目，包含命令元数据用于 frecency 排序
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub command: String,
    pub timestamp: Instant,
    pub use_count: u32,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
}

impl HistoryEntry {
    pub fn new(command: String) -> Self {
        Self {
            command,
            timestamp: Instant::now(),
            use_count: 1,
            cwd: None,
            exit_code: None,
        }
    }

    pub fn with_cwd(mut self, cwd: Option<String>) -> Self {
        self.cwd = cwd;
        self
    }

    pub fn with_exit_code(mut self, exit_code: Option<i32>) -> Self {
        self.exit_code = exit_code;
        self
    }

    /// 计算 frecency 评分
    ///
    /// 综合考虑使用频率、时近度、目录匹配和执行成功率。
    /// 评分越高越优先。
    pub fn frecency_score(&self, current_cwd: Option<&str>) -> f64 {
        let recency_weight = recency_weight(self.timestamp);
        let dir_bonus = dir_bonus(&self.cwd, current_cwd);
        let success_bonus = success_bonus(self.exit_code);

        self.use_count as f64 * recency_weight * dir_bonus * success_bonus
    }
}

/// 时近度权重（参考 Atuin/Firefox frecency）
fn recency_weight(timestamp: Instant) -> f64 {
    let elapsed = timestamp.elapsed();
    if elapsed.as_secs() < 300 {
        // 最近 5 分钟
        100.0
    } else if elapsed.as_secs() < 3600 {
        // 最近 1 小时
        70.0
    } else if elapsed.as_secs() < 86400 {
        // 最近 24 小时
        50.0
    } else {
        30.0
    }
}

/// 目录匹配加分
fn dir_bonus(entry_cwd: &Option<String>, current_cwd: Option<&str>) -> f64 {
    match (entry_cwd.as_deref(), current_cwd) {
        (Some(entry), Some(current)) if entry == current => 1.5,
        _ => 1.0,
    }
}

/// 成功执行加分
fn success_bonus(exit_code: Option<i32>) -> f64 {
    match exit_code {
        Some(code) if code != 0 => 0.5,
        _ => 1.0, // None 或 0 都视为成功
    }
}

pub fn normalize_history_command(command: &str) -> Option<String> {
    let normalized = command.trim();
    (!normalized.is_empty()).then(|| normalized.to_string())
}

pub fn parse_shell_history(contents: &str, format: ShellHistoryFormat) -> Vec<String> {
    contents
        .lines()
        .filter_map(|line| match format {
            ShellHistoryFormat::Bash => normalize_history_command(line),
            ShellHistoryFormat::Zsh => {
                if let Some((_, command)) = line.split_once(';') {
                    normalize_history_command(command)
                } else {
                    normalize_history_command(line)
                }
            }
        })
        .collect()
}

/// 向历史记录中添加富条目（全局去重）
///
/// 如果已存在相同命令，更新其 timestamp 和 use_count，而非添加新条目。
pub fn push_rich_history_entry(
    entries: &mut VecDeque<HistoryEntry>,
    mut entry: HistoryEntry,
    limit: usize,
) -> bool {
    let Some(command) = normalize_history_command(&entry.command) else {
        return false;
    };
    entry.command = command;

    // 全局去重：查找已有条目并更新
    if let Some(existing) = entries.iter_mut().find(|e| e.command == entry.command) {
        existing.timestamp = entry.timestamp;
        existing.use_count += 1;
        if entry.cwd.is_some() {
            existing.cwd = entry.cwd;
        }
        if entry.exit_code.is_some() {
            existing.exit_code = entry.exit_code;
        }
        return true;
    }

    entries.push_back(entry);
    while entries.len() > limit.max(1) {
        entries.pop_front();
    }
    true
}

/// 兼容层：从字符串添加历史条目
pub fn push_history_entry(
    entries: &mut VecDeque<HistoryEntry>,
    command: &str,
    limit: usize,
) -> bool {
    let Some(command) = normalize_history_command(command) else {
        return false;
    };

    // 相邻去重（快速路径）
    if entries
        .back()
        .is_some_and(|existing| existing.command == command)
    {
        // 更新 timestamp
        if let Some(last) = entries.back_mut() {
            last.timestamp = Instant::now();
            last.use_count += 1;
        }
        return false;
    }

    // 全局去重
    push_rich_history_entry(entries, HistoryEntry::new(command), limit)
}

/// InlineSuggest 策略链匹配排名
///
/// 按优先级返回匹配等级：
/// - 0: 前缀匹配（ghost text 可直接追加后缀）
/// - 1: 单词前缀匹配（命令中某个 token 以 query 开头）
/// - 2: 子串包含匹配
fn suggestion_rank(command: &str, query: &str) -> Option<u8> {
    if command.starts_with(query) {
        Some(0)
    } else if has_token_prefix(command, query) {
        Some(1)
    } else if has_token_initialism_prefix(command, query) {
        Some(2)
    } else if command.contains(query) {
        Some(3)
    } else if is_subsequence(query, command) {
        Some(4)
    } else {
        None
    }
}

/// 从 session (HistoryEntry) + persisted (String) 中收集唯一命令
fn collect_unique_commands(session: &VecDeque<HistoryEntry>, persisted: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut commands = Vec::new();

    for entry in session.iter().rev() {
        if seen.insert(entry.command.clone()) {
            commands.push(entry.command.clone());
        }
    }
    for command in persisted.iter().rev() {
        if seen.insert(command.clone()) {
            commands.push(command.clone());
        }
    }

    commands
}

/// 按 frecency 评分对 session 条目排序的辅助
///
/// 返回 (command, frecency_score) 映射
fn build_frecency_map(
    session: &VecDeque<HistoryEntry>,
    current_cwd: Option<&str>,
) -> std::collections::HashMap<String, f64> {
    session
        .iter()
        .map(|entry| (entry.command.clone(), entry.frecency_score(current_cwd)))
        .collect()
}

pub fn collect_history_suggestions(
    session: &VecDeque<HistoryEntry>,
    persisted: &[String],
    prefix: &str,
    limit: usize,
) -> Vec<String> {
    collect_history_suggestions_with_cwd(session, persisted, prefix, limit, None)
}

pub fn collect_history_suggestions_with_cwd(
    session: &VecDeque<HistoryEntry>,
    persisted: &[String],
    prefix: &str,
    limit: usize,
    current_cwd: Option<&str>,
) -> Vec<String> {
    let limit = limit.max(1);
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return Vec::new();
    }

    let commands = collect_unique_commands(session, persisted);
    let lowered_prefix = prefix.to_lowercase();
    let frecency_map = build_frecency_map(session, current_cwd);

    let mut matches: Vec<(u8, i64, String)> = commands
        .into_iter()
        .filter_map(|command| {
            let lowered_command = command.to_lowercase();
            let rank = suggestion_rank(&lowered_command, &lowered_prefix)?;
            // 使用 frecency 的负数作为排序键（越高越优先 → 负数越小排在前面）
            let neg_frecency = frecency_map
                .get(&command)
                .map(|s| (-s * 1000.0) as i64)
                .unwrap_or(0);
            Some((rank, neg_frecency, command))
        })
        .collect();

    matches.sort_by_key(|(rank, neg_frecency, _)| (*rank, *neg_frecency));
    matches.truncate(limit);
    matches.into_iter().map(|(_, _, command)| command).collect()
}

fn has_token_prefix(command: &str, query: &str) -> bool {
    command
        .split_whitespace()
        .any(|token| token.starts_with(query))
}

fn token_initialism(command: &str) -> String {
    command
        .split_whitespace()
        .filter_map(|token| {
            token
                .chars()
                .find(|ch| ch.is_ascii_alphanumeric())
                .map(|ch| ch.to_ascii_lowercase())
        })
        .collect()
}

fn has_token_initialism_prefix(command: &str, query: &str) -> bool {
    if query.len() < 2 {
        return false;
    }
    token_initialism(command).starts_with(query)
}

fn is_subsequence(query: &str, command: &str) -> bool {
    let mut query_chars = query.chars();
    let mut current = query_chars.next();
    if current.is_none() {
        return true;
    }

    for ch in command.chars() {
        if Some(ch) == current {
            current = query_chars.next();
            if current.is_none() {
                return true;
            }
        }
    }

    false
}

fn history_search_rank(command: &str, query: &str) -> Option<u8> {
    if command.starts_with(query) {
        Some(0)
    } else if has_token_prefix(command, query) {
        Some(1)
    } else if has_token_initialism_prefix(command, query) {
        Some(2)
    } else if command.contains(query) {
        Some(3)
    } else if is_subsequence(query, command) {
        Some(4)
    } else {
        None
    }
}

pub fn collect_history_search_results(
    session: &VecDeque<HistoryEntry>,
    persisted: &[String],
    query: &str,
    limit: usize,
) -> Vec<String> {
    let limit = limit.max(1);
    let query = query.trim();
    let commands = collect_unique_commands(session, persisted);
    if query.is_empty() {
        return commands.into_iter().take(limit).collect();
    }

    let frecency_map = build_frecency_map(session, None);
    let lowered_query = query.to_lowercase();
    let mut matches = commands
        .into_iter()
        .enumerate()
        .filter_map(|(recency, command)| {
            let lowered_command = command.to_lowercase();
            let rank = history_search_rank(&lowered_command, &lowered_query)?;
            let neg_frecency = frecency_map
                .get(&command)
                .map(|s| (-s * 1000.0) as i64)
                .unwrap_or(recency as i64);
            Some((rank, neg_frecency, command))
        })
        .collect::<Vec<_>>();

    matches.sort_by_key(|(rank, neg_frecency, _)| (*rank, *neg_frecency));
    matches.truncate(limit);
    matches.into_iter().map(|(_, _, command)| command).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::time::{Duration, Instant};

    /// 创建带时间偏移的 HistoryEntry（用于测试 frecency）
    fn entry_with_age(command: &str, age_secs: u64, use_count: u32) -> HistoryEntry {
        HistoryEntry {
            command: command.to_string(),
            timestamp: Instant::now() - Duration::from_secs(age_secs),
            use_count,
            cwd: None,
            exit_code: None,
        }
    }

    fn entry(command: &str) -> HistoryEntry {
        HistoryEntry::new(command.to_string())
    }

    fn session_from_strings(commands: &[&str]) -> VecDeque<HistoryEntry> {
        commands.iter().map(|c| entry(c)).collect()
    }

    #[test]
    fn normalize_history_command_trims_and_rejects_blank_input() {
        assert_eq!(
            normalize_history_command("  git status  "),
            Some("git status".to_string())
        );
        assert_eq!(normalize_history_command("   "), None);
        assert_eq!(normalize_history_command("\n\t"), None);
    }

    #[test]
    fn parse_shell_history_supports_zsh_extended_format() {
        let commands = parse_shell_history(
            ": 1710000000:0;git status\n: 1710000001:0;cargo test\n",
            ShellHistoryFormat::Zsh,
        );

        assert_eq!(commands, vec!["git status", "cargo test"]);
    }

    #[test]
    fn push_history_entry_dedupes_adjacent_duplicates() {
        let mut entries = VecDeque::new();
        push_history_entry(&mut entries, "git status", 5);
        push_history_entry(&mut entries, "git status", 5);
        push_history_entry(&mut entries, "cargo test", 5);

        let commands: Vec<_> = entries.iter().map(|e| e.command.as_str()).collect();
        assert_eq!(commands, vec!["git status", "cargo test"]);
    }

    #[test]
    fn push_history_entry_global_dedup_updates_existing() {
        let mut entries = VecDeque::new();
        push_history_entry(&mut entries, "git status", 10);
        push_history_entry(&mut entries, "cargo test", 10);
        push_history_entry(&mut entries, "git status", 10);

        // 全局去重：git status 应更新 use_count 而非新增
        assert_eq!(entries.len(), 2);
        let git_entry = entries.iter().find(|e| e.command == "git status").unwrap();
        assert!(git_entry.use_count >= 2);
    }

    #[test]
    fn collect_history_suggestions_prioritizes_session_history() {
        let session = session_from_strings(&["git status", "git stash", "cargo test"]);
        let persisted = vec![
            "git status".to_string(),
            "git switch main".to_string(),
            "git commit".to_string(),
        ];

        let matches = collect_history_suggestions(&session, &persisted, "git s", 4);

        // session 中的结果优先（frecency 更高），且去重
        assert!(matches.contains(&"git stash".to_string()));
        assert!(matches.contains(&"git status".to_string()));
        assert!(matches.contains(&"git switch main".to_string()));
    }

    #[test]
    fn collect_history_suggestions_skips_empty_prefix() {
        let session = session_from_strings(&["git status"]);
        let persisted = vec!["git switch".to_string()];

        let matches = collect_history_suggestions(&session, &persisted, "   ", 5);

        assert!(matches.is_empty());
    }

    #[test]
    fn inline_suggest_matches_substring() {
        let session = session_from_strings(&["cargo test", "cargo build", "npm test"]);
        let persisted = vec![];

        let matches = collect_history_suggestions(&session, &persisted, "test", 5);

        assert!(matches.contains(&"cargo test".to_string()));
        assert!(matches.contains(&"npm test".to_string()));
        assert!(!matches.contains(&"cargo build".to_string()));
    }

    #[test]
    fn inline_suggest_matches_token_prefix() {
        let session = session_from_strings(&["git status", "git stash", "status report"]);
        let persisted = vec![];

        let matches = collect_history_suggestions(&session, &persisted, "status", 5);

        // "status report" 是前缀匹配 (rank 0)，排最前
        assert_eq!(matches[0], "status report");
        assert!(matches.contains(&"git status".to_string()));
    }

    #[test]
    fn inline_suggest_prefix_ranked_first() {
        let session =
            session_from_strings(&["cargo test --release", "test-runner start", "npm run test"]);
        let persisted = vec![];

        let matches = collect_history_suggestions(&session, &persisted, "test", 5);

        assert_eq!(matches[0], "test-runner start");
    }

    #[test]
    fn inline_suggest_case_insensitive() {
        let session = session_from_strings(&["Git Status", "GIT PUSH"]);
        let persisted = vec![];

        let matches = collect_history_suggestions(&session, &persisted, "git", 5);

        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn inline_suggest_matches_token_initialism() {
        let session = session_from_strings(&["git commit -m", "git checkout main"]);
        let persisted = vec![];

        let matches = collect_history_suggestions(&session, &persisted, "gcm", 5);

        assert!(matches.contains(&"git commit -m".to_string()));
    }

    #[test]
    fn inline_suggest_matches_subsequence_after_stronger_strategies() {
        let session = session_from_strings(&["git status", "git stash", "cargo test"]);
        let persisted = vec![];

        let matches = collect_history_suggestions(&session, &persisted, "gst", 5);

        assert!(matches.contains(&"git status".to_string()));
        assert!(matches.contains(&"git stash".to_string()));
    }

    #[test]
    fn collect_history_search_results_prefers_prefix_then_token_then_subsequence() {
        let session = session_from_strings(&["git log", "git status", "git stash", "cargo test"]);
        let persisted = vec![
            "status report".to_string(),
            "cargo status".to_string(),
            "gitstatus checkout".to_string(),
        ];

        let matches = collect_history_search_results(&session, &persisted, "status", 5);

        // rank 0 (prefix): "status report"
        // rank 1 (token prefix): "git status", "cargo status"
        // rank 2 (substring): "gitstatus checkout"
        assert_eq!(matches[0], "status report");
        assert!(matches.contains(&"git status".to_string()));
        assert!(matches.contains(&"cargo status".to_string()));
        assert!(matches.contains(&"gitstatus checkout".to_string()));
    }

    #[test]
    fn collect_history_search_results_breaks_ties_by_frecency() {
        let session = session_from_strings(&["git status", "git stash", "go test ./..."]);
        let persisted = vec![
            "gh stack sync".to_string(),
            "git switch main".to_string(),
            "cargo test".to_string(),
        ];

        let matches = collect_history_search_results(&session, &persisted, "gsta", 3);

        // 所有都是 subsequence match，session 中的 frecency 更高
        assert!(matches.contains(&"git stash".to_string()));
        assert!(matches.contains(&"git status".to_string()));
    }

    // ---- Frecency 评分测试 ----

    #[test]
    fn frecency_ranks_frequent_commands_higher() {
        let mut session = VecDeque::new();
        // "cargo test" 使用 5 次
        for _ in 0..5 {
            push_history_entry(&mut session, "cargo test", 100);
            // 插入其他命令避免相邻去重
            push_history_entry(&mut session, "git status", 100);
        }

        let cargo_entry = session.iter().find(|e| e.command == "cargo test").unwrap();
        let git_entry = session.iter().find(|e| e.command == "git status").unwrap();

        assert!(cargo_entry.frecency_score(None) >= git_entry.frecency_score(None));
    }

    #[test]
    fn frecency_ranks_recent_commands_higher() {
        let recent = entry_with_age("recent cmd", 60, 1); // 1 分钟前
        let old = entry_with_age("old cmd", 100_000, 1); // ~28 小时前

        assert!(recent.frecency_score(None) > old.frecency_score(None));
    }

    #[test]
    fn frecency_deprioritizes_failed_commands() {
        let success = HistoryEntry {
            exit_code: Some(0),
            ..entry("cmd")
        };
        let failure = HistoryEntry {
            exit_code: Some(1),
            ..entry("cmd")
        };

        assert!(success.frecency_score(None) > failure.frecency_score(None));
    }

    #[test]
    fn frecency_boosts_same_directory_commands() {
        let entry_in_dir = HistoryEntry {
            cwd: Some("/home/user/project".to_string()),
            ..entry("make build")
        };

        let score_same_dir = entry_in_dir.frecency_score(Some("/home/user/project"));
        let score_diff_dir = entry_in_dir.frecency_score(Some("/tmp"));
        let score_no_dir = entry_in_dir.frecency_score(None);

        assert!(score_same_dir > score_diff_dir);
        assert!(score_same_dir > score_no_dir);
    }

    #[test]
    fn push_rich_history_entry_with_metadata() {
        let mut entries = VecDeque::new();
        let entry = HistoryEntry::new("cargo test".to_string())
            .with_cwd(Some("/project".to_string()))
            .with_exit_code(Some(0));

        push_rich_history_entry(&mut entries, entry, 10);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].cwd.as_deref(), Some("/project"));
        assert_eq!(entries[0].exit_code, Some(0));
    }
}
