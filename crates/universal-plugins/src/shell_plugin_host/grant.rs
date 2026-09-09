//! shell 视图权限 → gpui-shell `Capabilities` 装配。
//!
//! Navop manifest 的 `fs:read:` / `fs:write:` / `net:tcp:` / `spawn:`
//! 权限此前只做静态校验，从未授予 shell JS 运行时：工具类 shell 页面
//!（hosts 编辑、加解密等）在宿主里拿到的是 `Capabilities::new()`，任何
//! `fs/promises`、`fetch`、`process.run` 调用都会被 capability 层拒绝。
//!
//! 这里把已通过 `security_rules` 校验的权限翻译成 `Capabilities`：
//!
//! - `fs:read:<path>` / `fs:write:<path>` → read/write roots（`~`、
//!   `%USERPROFILE%` 等按 connection-import 相同规则展开；`${pluginDir}`
//!   展开为扩展根，供工具打包静态资源）
//! - `net:tcp:<host>:<port>` → network host（host 粒度；gpui-shell 的
//!   `Capabilities::may_reach` 不做端口区分，端口约束仍由 provider IPC
//!   的 endpoint preflight 承担）
//! - `spawn:<path>` → execute allowlist（basename 粒度，与
//!   `Capabilities::may_run` 匹配方式一致）
//!
//! fail-closed：没列出的权限一律不进入 grant；无法展开的路径（如缺失
//! env var）跳过该条并记录，而不是把未展开字面量当 root。

use std::path::PathBuf;

use extension_runtime::extension::manifest::security::{PermissionKind, validate_permissions};
use gpui_shell::{Capabilities, ExecuteGrant};

/// 从 manifest 权限装配 shell 运行时 capabilities。
///
/// 返回 (capabilities, skipped)：skipped 是无法展开而被丢弃的权限原文，
/// 供宿主日志透出，避免静默降权。
pub(crate) fn capabilities_from_permissions(
    permissions: &[String],
    extension_root: &std::path::Path,
) -> (Capabilities, Vec<String>) {
    let mut read_roots = Vec::new();
    let mut write_roots = Vec::new();
    let mut hosts = Vec::new();
    let mut commands = Vec::new();
    let mut skipped = Vec::new();

    let Ok(validated) = validate_permissions(permissions) else {
        return (Capabilities::new(), Vec::new());
    };

    for permission in &validated {
        match permission.kind {
            PermissionKind::FileSystem => {
                let (path, read) = if let Some(path) = permission.raw.strip_prefix("fs:read:") {
                    (path, true)
                } else if let Some(path) = permission.raw.strip_prefix("fs:write:") {
                    (path, false)
                } else {
                    continue;
                };
                match expand_grant_path(path, extension_root) {
                    Some(root) => {
                        let root = canonicalize_grant_root(root);
                        if read {
                            read_roots.push(root);
                        } else {
                            write_roots.push(root);
                        }
                    }
                    None => skipped.push(permission.raw.clone()),
                }
            }
            PermissionKind::Network => {
                let Some(host) = split_net_host(&permission.raw) else {
                    continue;
                };
                hosts.push(host.to_string());
            }
            PermissionKind::Spawn => {
                let Some(path) = permission.raw.strip_prefix("spawn:") else {
                    continue;
                };
                commands.push(command_name(path).to_string());
            }
            _ => {}
        }
    }

    let capabilities = Capabilities::new()
        .read_roots(read_roots)
        .write_roots(write_roots)
        .network_hosts(hosts)
        .execute(ExecuteGrant::Allowed(commands));
    (capabilities, skipped)
}

fn canonicalize_grant_root(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

/// 展开 `${pluginDir}`、`~`、`%VAR%` 前缀；无法展开返回 None。
fn expand_grant_path(path: &str, extension_root: &std::path::Path) -> Option<PathBuf> {
    if let Some(rest) = path.strip_prefix("${pluginDir}") {
        if rest.is_empty() {
            return Some(extension_root.to_path_buf());
        }
        let rest = rest.trim_start_matches('/');
        let mut root = extension_root.to_path_buf();
        root.push(rest);
        return Some(root);
    }
    let home = dirs::home_dir();
    if let Some(rest) = path.strip_prefix("~/") {
        return home.map(|home| home.join(rest));
    }
    if path == "~" {
        return home;
    }
    if let Some((name, rest)) = windows_env_prefix(path) {
        return std::env::var_os(name).map(|value| join_expanded_path(value, rest));
    }
    if path.starts_with('/') || path.starts_with('\\') || std::path::Path::new(path).is_absolute() {
        return Some(PathBuf::from(path));
    }
    None
}

fn windows_env_prefix(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix('%')?;
    let end = rest.find('%')?;
    let name = &rest[..end];
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
    {
        return None;
    }
    let tail = &rest[end + 1..];
    if !tail.is_empty() && !tail.starts_with('/') && !tail.starts_with('\\') {
        return None;
    }
    Some((name, tail))
}

fn join_expanded_path(base: std::ffi::OsString, rest: &str) -> PathBuf {
    let mut path = PathBuf::from(base);
    let rest = rest.trim_start_matches(['/', '\\']);
    if !rest.is_empty() {
        path.push(rest);
    }
    path
}

/// `net:tcp:<host>:<port>` → host。
fn split_net_host(raw: &str) -> Option<&str> {
    let mut parts = raw.splitn(4, ':');
    let _net = parts.next()?;
    let _proto = parts.next()?;
    parts.next()
}

/// spawn 权限路径 → `Capabilities::may_run` 匹配的命令名。
///
/// allowlist 只允许 `./...`（扩展内）或 `/usr/bin/...`，basename 即命令名；
/// `process.run` 调用侧传命令名（如 `my-tool`），与 allowlist 同粒度。
fn command_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn permissions(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn fs_read_write_become_roots() {
        let (capabilities, skipped) = capabilities_from_permissions(
            &permissions(&["fs:read:/etc/hosts", "fs:write:/etc/hosts"]),
            std::path::Path::new("/tmp/ext"),
        );
        assert!(skipped.is_empty());
        assert!(capabilities.has_read_access());
        assert!(capabilities.has_write_access());
    }

    #[test]
    fn net_permission_grants_host() {
        let (capabilities, _) = capabilities_from_permissions(
            &permissions(&["net:tcp:api.example.com:443"]),
            std::path::Path::new("/tmp/ext"),
        );
        assert!(capabilities.may_reach("api.example.com"));
        assert!(!capabilities.may_reach("other.example.com"));
    }

    #[test]
    fn spawn_permission_grants_command() {
        let (capabilities, _) = capabilities_from_permissions(
            &permissions(&["spawn:./bin/tool"]),
            std::path::Path::new("/tmp/ext"),
        );
        assert!(capabilities.may_run("tool"));
        assert!(!capabilities.may_run("other"));
    }

    #[test]
    fn unknown_permissions_grant_nothing() {
        let (capabilities, skipped) = capabilities_from_permissions(
            &permissions(&["shell:exec", "secrets:read:self.*"]),
            std::path::Path::new("/tmp/ext"),
        );
        assert!(skipped.is_empty());
        assert!(!capabilities.has_read_access());
        assert!(!capabilities.has_write_access());
        assert!(!capabilities.may_reach("api.example.com"));
        assert!(!capabilities.may_run("tool"));
    }

    #[test]
    fn plugin_dir_expands_to_extension_root() {
        let (capabilities, skipped) = capabilities_from_permissions(
            &permissions(&["fs:read:${pluginDir}/assets"]),
            std::path::Path::new("/tmp/ext"),
        );
        assert!(skipped.is_empty());
        assert!(capabilities.has_read_access());
    }

    #[test]
    fn tilde_expands_with_home() {
        let (capabilities, skipped) = capabilities_from_permissions(
            &permissions(&["fs:read:~/Documents"]),
            std::path::Path::new("/tmp/ext"),
        );
        assert!(skipped.is_empty());
        assert!(capabilities.has_read_access());
    }

    #[test]
    fn relative_path_outside_allowed_forms_grants_nothing() {
        // security_rules 不允许相对路径（除 ${pluginDir}），整表 Invalid →
        // fail-closed；无部分授予。
        let (capabilities, skipped) = capabilities_from_permissions(
            &permissions(&["fs:read:relative/path"]),
            std::path::Path::new("/tmp/ext"),
        );
        assert!(skipped.is_empty());
        assert!(!capabilities.has_read_access());
    }

    #[test]
    fn unexpandable_windows_env_is_skipped_not_granted() {
        // 合法形式但 env 缺失：跳过并上报，不把字面量当 root。
        let (capabilities, skipped) = capabilities_from_permissions(
            &permissions(&["fs:read:%NAVOP_MISSING_VAR%/data"]),
            std::path::Path::new("/tmp/ext"),
        );
        assert_eq!(
            skipped,
            vec!["fs:read:%NAVOP_MISSING_VAR%/data".to_string()]
        );
        assert!(!capabilities.has_read_access());
    }

    #[test]
    fn invalid_permission_list_fails_closed() {
        let (capabilities, skipped) = capabilities_from_permissions(
            &permissions(&["not:a:permission"]),
            std::path::Path::new("/tmp/ext"),
        );
        assert!(skipped.is_empty());
        assert!(!capabilities.has_read_access());
    }
}
