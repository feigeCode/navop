//! FTP LIST 输出解析。
//!
//! 不同服务器的 LIST 格式差异很大；这里只保证常见 Unix 风格
//! (`ls -l`) 输出可解析。无法识别的行返回 `None` 并由调用方跳过，
//! 不让整个目录列表失败。
//!
//! 安全约束：LIST 返回的名称是**不可信输入**（来自可能被攻陷的服务端），
//! 只接受"单层名称"；包含路径分隔符或 `..` 的名称一律拒绝，
//! 防止递归传输写出/删除用户所选目录之外的目标。

use sftp::FileEntry;
use std::time::{Duration, SystemTime};

/// 单行 LIST 解析结果。
///
/// 解析器必须区分三种情况："没有条目"不等于"解析失败"：
/// 空目录可能只包含 `total N` 汇总行或 `.`/`..` 条目，这些都是合法输出。
#[derive(Debug)]
pub(super) enum ListLine {
    /// 有效条目。
    Entry(FileEntry),
    /// 可正常忽略的行：空行、`total` 汇总行、`.`/`..` 目录项。
    Ignorable,
    /// 真正无法解析或名称不安全的行（可能来自被攻陷的服务端）。
    Unparsable,
}

/// LIST 返回的名称只接受单层名称。
///
/// 拒绝：空名、`.`、`..`、包含 `/`、`\`、NUL 的名称。
pub(super) fn is_valid_remote_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

/// 解析一行 Unix 风格 `ls -l` 输出。
///
/// 示例：
/// - `-rw-r--r-- 1 user group 123 Jan 01 12:00 file.txt`
/// - `drwxr-xr-x 1 user group 0 Jan 01 12:00 directory`
/// - `lrwxrwxrwx 1 user group 7 Jan 01 12:00 link -> target`
///
/// 文件名原样保留（连续空格、首尾空白不丢失）；只去掉行尾 CR/LF。
/// `total N` 汇总行、空行、`.`/`..` 目录项返回 `Ignorable`；
/// 其余无法解析或名称不安全的行返回 `Unparsable`。
pub(super) fn parse_list_line(line: &str) -> ListLine {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let line = line.strip_suffix('\n').unwrap_or(line);
    if line.is_empty() {
        return ListLine::Ignorable;
    }
    // `total N` 汇总行：真实条目以 10 位权限串开头，不会误命中。
    let first_end = line.find(' ').unwrap_or(line.len());
    if line[..first_end].eq_ignore_ascii_case("total") {
        return ListLine::Ignorable;
    }
    // 按空格逐字段切分并记录偏移；第 9 个字段起的剩余部分是原始文件名，
    // 不能用 split_whitespace 重建（会合并连续空格、丢失尾部空白）。
    let mut fields: [&str; 8] = [""; 8];
    let mut rest = line;
    for field in &mut fields {
        rest = rest.trim_start_matches(' ');
        if rest.is_empty() {
            return ListLine::Unparsable;
        }
        let end = rest.find(' ').unwrap_or(rest.len());
        *field = &rest[..end];
        rest = &rest[end..];
    }
    let permission_field = fields[0];
    // 只接受 10 位 Unix 权限串（`-`/`d`/`l` 开头）。
    if permission_field.len() != 10 {
        return ListLine::Unparsable;
    }
    if !permission_field.starts_with(['-', 'd', 'l', 'c', 'b', 'p', 's']) {
        return ListLine::Unparsable;
    }
    let is_dir = permission_field.starts_with('d');
    let is_symlink = permission_field.starts_with('l');
    let size: u64 = match fields[4].parse() {
        Ok(size) => size,
        Err(_) => return ListLine::Unparsable,
    };
    // Unix ls：最近文件为 `MMM dd HH:mm`，一年以上为 `MMM dd yyyy`。
    let modified = match parse_unix_list_time(fields[5], fields[6], fields[7]) {
        Some(modified) => modified,
        None => return ListLine::Unparsable,
    };
    // 文件名原样保留（含连续空格与首尾空白）。
    let mut name = rest.trim_start_matches(' ');
    // 符号链接行形如 `name -> target`；只取链接名本身，
    // 链接类型按非目录处理（是否指向目录由上层 stat 探测）。
    if is_symlink {
        name = name.split(" -> ").next().unwrap_or(name);
    }
    // `.`/`..` 是目录列表的正常组成：忽略而非报错。
    if name == "." || name == ".." {
        return ListLine::Ignorable;
    }
    if !is_valid_remote_name(name) {
        return ListLine::Unparsable;
    }
    let name = name.to_string();
    ListLine::Entry(FileEntry {
        path: name.clone(),
        name,
        size,
        modified,
        is_dir,
        // FTP LIST 不提供可靠的数值权限；填 0，UI 不展示伪造权限。
        permissions: 0,
        uid: None,
        gid: None,
        user: non_empty(fields[2]),
        group: non_empty(fields[3]),
    })
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() || value == "?" {
        None
    } else {
        Some(value.to_string())
    }
}

/// 解析 `MMM dd HH:mm` / `MMM dd yyyy` 形式的时间。
fn parse_unix_list_time(month: &str, day: &str, rest: &str) -> Option<SystemTime> {
    const MONTHS: [u8; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let month_index: usize = match month.trim_end_matches('.') {
        "Jan" => 0,
        "Feb" => 1,
        "Mar" => 2,
        "Apr" => 3,
        "May" => 4,
        "Jun" => 5,
        "Jul" => 6,
        "Aug" => 7,
        "Sep" => 8,
        "Oct" => 9,
        "Nov" => 10,
        "Dec" => 11,
        _ => return None,
    };
    let day: u64 = day.parse().ok()?;
    if day == 0 || day as usize > MONTHS[month_index] as usize {
        return None;
    }
    // 距 1970-01-01 的近似秒数：只用于排序展示，不要求精确。
    let days_since_epoch: u64 = {
        let years = if rest.len() == 4 {
            rest.parse::<u64>().ok()? // `MMM dd yyyy`
        } else {
            current_utc_year() // `MMM dd HH:mm`：视为当前年份
        };
        if years < 1970 {
            return None;
        }
        let mut days = 0u64;
        for year in 1970..years {
            days += if is_leap(year) { 366 } else { 365 };
        }
        days + month_start_days(month_index as u8, is_leap(years)) + day - 1
    };
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(days_since_epoch * 86_400))
}

fn month_start_days(month_index: u8, leap: bool) -> u64 {
    const CUMULATIVE: [u64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let days = CUMULATIVE[month_index as usize];
    if leap && month_index >= 2 {
        days + 1
    } else {
        days
    }
}

fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn current_utc_year() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| 1970 + elapsed.as_secs() / 31_536_000)
        .unwrap_or(1970)
}

#[cfg(test)]
mod tests {
    use super::{ListLine, is_valid_remote_name, parse_list_line};

    fn entry_of(line: &str) -> sftp::FileEntry {
        match parse_list_line(line) {
            ListLine::Entry(entry) => entry,
            other => panic!("expected entry for {line:?}, got {other:?}"),
        }
    }

    fn is_ignorable(line: &str) -> bool {
        matches!(parse_list_line(line), ListLine::Ignorable)
    }

    fn is_unparsable(line: &str) -> bool {
        matches!(parse_list_line(line), ListLine::Unparsable)
    }

    #[test]
    fn parses_unix_file_listing() {
        let entry = entry_of("-rw-r--r-- 1 user group 123 Jan 01 12:00 file.txt");
        assert_eq!(entry.name, "file.txt");
        assert_eq!(entry.size, 123);
        assert!(!entry.is_dir);
        assert_eq!(entry.user.as_deref(), Some("user"));
        assert_eq!(entry.group.as_deref(), Some("group"));
        assert_eq!(entry.permissions, 0);
    }

    #[test]
    fn parses_unix_directory_listing() {
        let entry = entry_of("drwxr-xr-x 1 user group 0 Jan 01 12:00 directory");
        assert_eq!(entry.name, "directory");
        assert!(entry.is_dir);
    }

    #[test]
    fn classifies_ignorable_lines() {
        // 汇总行、空行、`.`/`..` 都是合法输出，不应记为解析失败。
        assert!(is_ignorable("total 0"));
        assert!(is_ignorable("total 128"));
        assert!(is_ignorable(""));
        assert!(is_ignorable("drwxr-xr-x 1 user group 0 Jan 01 12:00 ."));
        assert!(is_ignorable("drwxr-xr-x 1 user group 0 Jan 01 12:00 .."));
    }

    #[test]
    fn classifies_unparsable_lines() {
        assert!(is_unparsable(
            "-rw-r--r-- 1 user group abc Jan 01 12:00 file"
        ));
        assert!(is_unparsable("garbage"));
        // 路径穿越名称（恶意/被攻陷服务端）必须被视为不安全行。
        assert!(is_unparsable(
            "-rw-r--r-- 1 u g 4 Jan 01 2025 ../escaped.txt"
        ));
        assert!(is_unparsable("-rw-r--r-- 1 u g 4 Jan 01 2025 a/b.txt"));
        assert!(is_unparsable(
            "-rw-r--r-- 1 u g 4 Jan 01 2025 ..\\escaped.txt"
        ));
        assert!(is_unparsable("-rw-r--r-- 1 u g 4 Jan 01 2025 /abs"));
    }

    #[test]
    fn preserves_spaces_in_file_names() {
        let entry = entry_of("-rw-r--r-- 1 user group 12 Jan 01 12:00 my file v2.txt");
        assert_eq!(entry.name, "my file v2.txt");
    }

    #[test]
    fn preserves_consecutive_and_trailing_spaces_in_names() {
        // 连续空格不得被合并。
        let entry = entry_of("-rw-r--r-- 1 user group 12 Jan 01 12:00 two  spaces.txt");
        assert_eq!(entry.name, "two  spaces.txt");
        // 尾部空格不得被丢弃（只去 CR/LF）。
        let entry = entry_of("-rw-r--r-- 1 user group 12 Jan 01 12:00 padded \r");
        assert_eq!(entry.name, "padded ");
    }

    #[test]
    fn parses_unicode_names() {
        let entry = entry_of("-rw-r--r-- 1 user group 12 Jan 01 12:00 中文 文件.txt");
        assert_eq!(entry.name, "中文 文件.txt");
    }

    #[test]
    fn parses_symlink_line_without_target() {
        let entry = entry_of("lrwxrwxrwx 1 user group 7 Jan 01 12:00 link -> target/dir");
        assert_eq!(entry.name, "link");
        assert!(!entry.is_dir, "symlink 类型不应按目录处理");
    }

    #[test]
    fn rejects_unsafe_names() {
        assert!(!is_valid_remote_name(""));
        assert!(!is_valid_remote_name("."));
        assert!(!is_valid_remote_name(".."));
        assert!(!is_valid_remote_name("a/b"));
        assert!(!is_valid_remote_name("a\\b"));
        assert!(!is_valid_remote_name("a\0b"));
        assert!(is_valid_remote_name("..."));
        assert!(is_valid_remote_name("a..b.txt"));
        assert!(is_valid_remote_name("中文.txt"));
    }

    #[test]
    fn parses_single_digit_day() {
        let entry = entry_of("-rw-r--r-- 1 user group 12 Jan  1 12:00 a.txt");
        assert_eq!(entry.name, "a.txt");
    }
}
