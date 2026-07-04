use super::security::{
    PermissionError, PermissionKind, PermissionRisk, ValidatedPermission, invalid, is_identifier,
    path_has_escape, permission_error, valid,
};

const MAX_NET_PORT_RANGE_SPAN: u16 = 100;

pub(super) fn validate_permission(
    permission: &str,
) -> Result<ValidatedPermission, PermissionError> {
    if permission == "shell:exec" {
        return Ok(valid(
            permission,
            PermissionKind::Shell,
            PermissionRisk::High,
        ));
    }
    if permission.starts_with("fs:") {
        return validate_fs_permission(permission);
    }
    if permission.starts_with("net:") {
        return validate_net_permission(permission);
    }
    if permission.starts_with("spawn:") {
        return validate_spawn_permission(permission);
    }
    if permission.starts_with("secrets:") {
        return validate_secret_permission(permission);
    }
    if permission.starts_with("db:") {
        return validate_db_permission(permission);
    }
    if permission.starts_with("ui:") {
        return validate_ui_permission(permission);
    }
    validate_host_permission(permission)
}

fn validate_fs_permission(permission: &str) -> Result<ValidatedPermission, PermissionError> {
    let Some(path) = permission
        .strip_prefix("fs:read:")
        .or_else(|| permission.strip_prefix("fs:write:"))
    else {
        return invalid(
            permission,
            "文件权限必须是 fs:read:<path> 或 fs:write:<path>",
        );
    };
    if path == "*" || path == "/" {
        return invalid(permission, "文件权限不能使用通配符或整盘根目录");
    }
    if path_has_escape(path) {
        return invalid(permission, "文件路径不能包含相对逃逸");
    }
    let allowed = path == "~"
        || path.starts_with("~/")
        || windows_env_path_prefix(path).is_some()
        || path.starts_with('/')
        || path == "${task.workspace}"
        || path == "${user_pick}";
    if !allowed {
        return invalid(permission, "文件路径必须是绝对路径、~/ 或允许的变量");
    }
    let risk = if path == "~" {
        PermissionRisk::High
    } else {
        PermissionRisk::Normal
    };
    Ok(valid(permission, PermissionKind::FileSystem, risk))
}

fn windows_env_path_prefix(path: &str) -> Option<&str> {
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
    if tail.starts_with('/') || tail.starts_with('\\') {
        Some(name)
    } else {
        None
    }
}

fn validate_net_permission(permission: &str) -> Result<ValidatedPermission, PermissionError> {
    let parts = permission.split(':').collect::<Vec<_>>();
    if parts.len() != 4 || !matches!(parts[1], "tcp" | "udp") {
        return invalid(permission, "网络权限必须是 net:tcp:<host>:<port-range>");
    }
    let host = parts[2];
    let port_range = parts[3];
    if host.is_empty() || port_range == "*" {
        return invalid(permission, "网络 host 不能为空且端口不能使用通配符");
    }
    let span = parse_port_range(port_range)
        .ok_or_else(|| permission_error(permission, "端口范围不合法"))?;
    if span > MAX_NET_PORT_RANGE_SPAN {
        return invalid(permission, "端口范围跨度不能超过 100");
    }
    let risk = if host == "*" {
        PermissionRisk::High
    } else {
        PermissionRisk::Normal
    };
    Ok(valid(permission, PermissionKind::Network, risk))
}

fn validate_spawn_permission(permission: &str) -> Result<ValidatedPermission, PermissionError> {
    let Some(path) = permission.strip_prefix("spawn:") else {
        return invalid(permission, "进程权限必须是 spawn:<path>");
    };
    if path == "/" || path == "*" || path_has_escape(path) {
        return invalid(permission, "spawn 路径不能是根目录、通配符或相对逃逸");
    }
    let allowed = path.starts_with("./") || path.starts_with("/usr/bin/");
    if !allowed {
        return invalid(
            permission,
            "spawn 路径必须在扩展目录内或 /usr/bin allowlist 内",
        );
    }
    Ok(valid(
        permission,
        PermissionKind::Spawn,
        PermissionRisk::Normal,
    ))
}

fn validate_secret_permission(permission: &str) -> Result<ValidatedPermission, PermissionError> {
    let Some(glob) = permission
        .strip_prefix("secrets:read:")
        .or_else(|| permission.strip_prefix("secrets:write:"))
    else {
        return invalid(
            permission,
            "凭证权限必须是 secrets:read:<glob> 或 secrets:write:<glob>",
        );
    };
    if !is_valid_secret_glob(glob) {
        return invalid(permission, "凭证 namespace 必须形如 ext.key 或 ext.*");
    }
    Ok(valid(
        permission,
        PermissionKind::Secrets,
        PermissionRisk::Normal,
    ))
}

fn validate_db_permission(permission: &str) -> Result<ValidatedPermission, PermissionError> {
    if permission == "db:connections:list" {
        return Ok(valid(
            permission,
            PermissionKind::Database,
            PermissionRisk::Normal,
        ));
    }
    let parts = permission.split(':').collect::<Vec<_>>();
    if parts.len() != 3 || !matches!(parts[1], "read" | "write" | "schema" | "admin") {
        return invalid(
            permission,
            "数据库权限必须是 db:connections:list 或 db:<read|write|schema|admin>:<connection-id|*>",
        );
    }
    let scope = parts[2];
    if !is_valid_db_scope(scope) {
        return invalid(permission, "数据库权限 scope 不能为空、路径或相对逃逸");
    }
    let risk = if matches!(parts[1], "write" | "schema" | "admin") {
        PermissionRisk::High
    } else {
        PermissionRisk::Normal
    };
    Ok(valid(permission, PermissionKind::Database, risk))
}

fn validate_ui_permission(permission: &str) -> Result<ValidatedPermission, PermissionError> {
    const UI_PERMISSIONS: &[&str] = &[
        "ui:dialog",
        "ui:webview",
        "ui:tab",
        "ui:progress",
        "ui:notify",
        "ui:result_view",
        "ui:refresh_tree",
    ];
    if UI_PERMISSIONS.contains(&permission) {
        return Ok(valid(
            permission,
            PermissionKind::Ui,
            PermissionRisk::Normal,
        ));
    }
    invalid(permission, "未知 UI 权限")
}

fn validate_host_permission(permission: &str) -> Result<ValidatedPermission, PermissionError> {
    const HOST_PERMISSIONS: &[&str] = &[
        "notifications:show",
        "host:ssh_tunnel",
        "host:open_browser",
        "host:clipboard:read",
        "host:clipboard:write",
    ];
    if HOST_PERMISSIONS.contains(&permission) {
        return Ok(valid(
            permission,
            PermissionKind::Host,
            PermissionRisk::Normal,
        ));
    }
    invalid(permission, "未知权限")
}

fn parse_port_range(port_range: &str) -> Option<u16> {
    let (start, end) = match port_range.split_once('-') {
        Some((start, end)) => (start.parse::<u16>().ok()?, end.parse::<u16>().ok()?),
        None => {
            let port = port_range.parse::<u16>().ok()?;
            (port, port)
        }
    };
    if start == 0 || end == 0 || end < start {
        return None;
    }
    Some(end - start + 1)
}

fn is_valid_secret_glob(glob: &str) -> bool {
    let Some((namespace, key)) = glob.split_once('.') else {
        return false;
    };
    is_identifier(namespace) && (key == "*" || is_identifier(key))
}

fn is_valid_db_scope(scope: &str) -> bool {
    !(scope.is_empty()
        || scope == "."
        || scope == ".."
        || scope.contains('/')
        || scope.contains('\\')
        || path_has_escape(scope))
}

#[cfg(test)]
mod tests {
    use super::validate_permission;

    #[test]
    fn secret_permission_allows_termius_localkey_scope() {
        let permission = validate_permission("secrets:read:termius.localkey").unwrap();

        assert_eq!(permission.raw, "secrets:read:termius.localkey");
    }
}
