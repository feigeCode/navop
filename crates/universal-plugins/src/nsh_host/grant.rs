//! manifest 权限 + 模块声明 → nsh `Capabilities`（宿主的授予上限）。
//!
//! 这是**上限**，不是授予本身：nsh 会把页面的 `capabilities.json`（声明）
//! 与这里的 granted 取交集，页面声明不了超出上限的东西。
//!
//! 为什么必须有这一层：`Bundle::policy.declared` 缺失时，nsh 的
//! `Policy::access()` 会退回 `Capabilities::all()` —— 页面只要不写
//! capabilities.json 就拿到了全部能力。所以宿主必须显式给 granted。
//!
//! 映射与 gpui-shell 版一致（fail-closed）：只认清单里已通过静态校验的
//! `fs:read:` / `fs:write:` / `net:tcp|udp:` / `spawn:`，无法展开的路径与
//! `*` 通配记入 skipped，交给调用方记日志。

use std::path::{Path, PathBuf};

use extension_runtime::RegisteredShellAppContribution;
use extension_runtime::extension::manifest::security::{PermissionKind, validate_permissions};
use nsh_core::access::Capabilities;
use serde_json::json;

/// 组装宿主授予上限。
pub(crate) fn granted(
    contribution: &RegisteredShellAppContribution,
) -> (Capabilities, Vec<String>) {
    let (read, write, hosts, sockets, commands, mut skipped) =
        grants_from_permissions(&contribution.permissions, &contribution.extension_root);
    // 页面能调的宿主函数 = 它声明的模块对应的命名空间。裸名字和 `.*` 都要给：
    // `navop.log` 模块唯一的宿主函数就叫裸的 `navop.log`（host.post），而
    // `navop.log.*` 只按前缀 `navop.log.` 匹配，匹配不到它本身。
    // 只认 `is_supported()` 的模块：validate_modules 已经拒绝 v2 删除的旧值，
    // 这里再兜一层，保证旧枚举值永远不会派生任何 host 能力。
    let mut host = contribution
        .modules
        .iter()
        .filter(|module| module.is_supported())
        .flat_map(|module| {
            let name = module.module_specifier();
            [name.to_string(), format!("{name}.*")]
        })
        .collect::<Vec<_>>();
    if cfg!(test) {
        // 自检页用一个假宿主函数回报装载结果；只在测试构建里放开，
        // 生产构建永远不授予 `navop.` 之外的宿主名。
        host.push("spike.mounted".into());
    }
    let mut value = json!({
        "fs": { "read": read, "write": write },
        "network": { "hosts": hosts, "sockets": sockets },
        "process": { "execute": commands },
        "host": host,
        // clipboard 上限恒放行：nsh 侧它对应真实可用的 `nsh/clipboard`（读写
        // 剪贴板），spec §7 也明确把 clipboard 计入派生集合；页面仍要自己在
        // nsh.json 里声明并与上限取交集。等 `navop.clipboard.*` 宿主能力落地后
        // 再决定是否按声明收紧。
        "clipboard": true,
    });
    if value["network"]["hosts"]
        .as_array()
        .is_some_and(Vec::is_empty)
    {
        value["network"]
            .as_object_mut()
            .map(|network| network.remove("hosts"));
    }
    if value["network"]["sockets"]
        .as_array()
        .is_some_and(Vec::is_empty)
    {
        value["network"]
            .as_object_mut()
            .map(|network| network.remove("sockets"));
    }
    match Capabilities::from_json_lenient(&value) {
        Ok((capabilities, dropped)) => {
            skipped.extend(dropped);
            (capabilities, skipped)
        }
        Err(error) => {
            // 组装失败就退回最小集：宁可供不上，也不能悄悄放权。
            tracing::warn!(%error, "扩展能力上限组装失败，退回最小集");
            (Minimum::capabilities(), skipped)
        }
    }
}

/// 组装失败时的兜底：只有空的宿主列表（= 任何 `navop.*` 都调不到）。
struct Minimum;

impl Minimum {
    fn capabilities() -> Capabilities {
        Capabilities::from_json_lenient(&json!({"host": []}))
            .map(|(capabilities, _)| capabilities)
            .expect("empty capabilities must parse")
    }
}

type Grants = (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
);

fn grants_from_permissions(permissions: &[String], extension_root: &Path) -> Grants {
    let mut read = Vec::new();
    let mut write = Vec::new();
    let mut hosts = Vec::new();
    let mut sockets = Vec::new();
    let mut commands = Vec::new();
    let mut skipped = Vec::new();

    let Ok(validated) = validate_permissions(permissions) else {
        return (read, write, hosts, sockets, commands, skipped);
    };
    for permission in &validated {
        match permission.kind {
            PermissionKind::FileSystem => {
                let (path, is_read) = if let Some(path) = permission.raw.strip_prefix("fs:read:") {
                    (path, true)
                } else if let Some(path) = permission.raw.strip_prefix("fs:write:") {
                    (path, false)
                } else {
                    continue;
                };
                match expand_path(path, extension_root) {
                    Some(root) => {
                        let root = root.canonicalize().unwrap_or(root);
                        let text = root.display().to_string();
                        if is_read {
                            read.push(text);
                        } else {
                            write.push(text);
                        }
                    }
                    None => skipped.push(permission.raw.clone()),
                }
            }
            PermissionKind::Network => {
                // `net:unix:` 在扩展清单里是合法声明（docker 扩展就声明了两条），
                // 但 nsh 的 Capabilities 没有 Unix socket 字段，也没有对应的宿主
                // 函数入口。这里**显式 warn + 丢弃**，比静默 continue 好：作者能
                // 从日志看出这条权限没生效，而不是等到页面调不通才发现。
                if let Some(path) = permission.raw.strip_prefix("net:unix:") {
                    tracing::warn!(
                        permission = %permission.raw,
                        path = %path,
                        "nsh 能力模型不支持 Unix socket，权限丢弃（fail-closed）"
                    );
                    skipped.push(permission.raw.clone());
                    continue;
                }
                let Some(rest) = permission
                    .raw
                    .strip_prefix("net:tcp:")
                    .or_else(|| permission.raw.strip_prefix("net:udp:"))
                else {
                    continue;
                };
                let Some((host, port)) = split_host_port(rest) else {
                    skipped.push(permission.raw.clone());
                    continue;
                };
                if host == "*" || host.starts_with("*.") {
                    // nsh 的 host 粒度通配要写成 `*.example.com`，但 `*`
                    // 本身表达不了"任意主机"——fail-closed 跳过并上报。
                    skipped.push(permission.raw.clone());
                    continue;
                }
                match port {
                    Some(port) => sockets.push(format!("{host}:{port}")),
                    None => hosts.push(host),
                }
            }
            PermissionKind::Spawn => {
                let Some(path) = permission.raw.strip_prefix("spawn:") else {
                    continue;
                };
                if let Some(name) = command_name(path) {
                    commands.push(name);
                }
            }
            _ => {}
        }
    }
    (read, write, hosts, sockets, commands, skipped)
}

/// `host:port` 拆两段；IPv6 走 `[::1]:port`。
fn split_host_port(rest: &str) -> Option<(String, Option<String>)> {
    if let Some(stripped) = rest.strip_prefix('[') {
        let end = stripped.find(']')?;
        let host = stripped[..end].to_string();
        let port = stripped[end + 1..].strip_prefix(':').map(str::to_owned);
        return Some((host, port));
    }
    match rest.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|ch| ch.is_ascii_digit()) => {
            Some((host.to_ascii_lowercase(), Some(port.to_owned())))
        }
        _ => Some((rest.to_ascii_lowercase(), None)),
    }
}

/// 展开 `${pluginDir}`、`~`；无法展开返回 `None`（该权限作废）。
fn expand_path(path: &str, extension_root: &Path) -> Option<PathBuf> {
    if let Some(rest) = path.strip_prefix("${pluginDir}") {
        let rest = rest.trim_start_matches('/');
        return Some(if rest.is_empty() {
            extension_root.to_path_buf()
        } else {
            extension_root.join(rest)
        });
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return dirs::home_dir().map(|home| home.join(rest));
    }
    if path == "~" {
        return dirs::home_dir();
    }
    let path = Path::new(path);
    path.is_absolute().then(|| path.to_path_buf())
}

fn command_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use extension_runtime::extension::manifest::ShellHostModule;

    use super::*;

    fn view(permissions: &[&str], modules: &[ShellHostModule]) -> RegisteredShellAppContribution {
        RegisteredShellAppContribution {
            extension_id: "dev.navop.grant-test".into(),
            extension_version: "0.1.0".into(),
            title: "页".into(),
            description: None,
            icon_path: None,
            entry_path: PathBuf::from("ui/bundle.js"),
            extension_root: PathBuf::from("."),
            category: None,
            keywords: Vec::new(),
            backends: BTreeMap::new(),
            modules: modules.iter().copied().collect::<BTreeSet<_>>(),
            permissions: permissions.iter().map(|p| (*p).to_string()).collect(),
            shell_api_version: "1.0".into(),
            required_nsh_version: Some("0.3.0".into()),
        }
    }

    #[test]
    fn host_capability_follows_declared_modules() {
        let (capabilities, _) = granted(&view(
            &["shell:exec"],
            &[ShellHostModule::Resource, ShellHostModule::Log],
        ));
        // 裸名字和 `.*` 都在：`navop.log` 是 log 模块唯一宿主函数的真名，
        // `navop.resource.*` 才是它命名空间下函数的前缀匹配。
        for expected in [
            "navop.resource",
            "navop.resource.*",
            "navop.log",
            "navop.log.*",
        ] {
            assert!(
                capabilities.host.iter().any(|host| host == expected),
                "缺少 `{expected}`: {:?}",
                capabilities.host
            );
        }
        // 模块只放出命名空间，不预先放出具体函数名。
        assert!(
            !capabilities
                .host
                .iter()
                .any(|host| host == "navop.resource.read")
        );
    }

    /// v2 删除的旧模块即使残留在清单里（安装期会被 shell_validation 拒掉），
    /// 也不能靠这里派生能力——fail-closed 的第二道闸。
    #[test]
    fn legacy_modules_never_grant_host_capabilities() {
        let (capabilities, _) = granted(&view(
            &["shell:exec"],
            &[
                ShellHostModule::Context,
                ShellHostModule::I18n,
                ShellHostModule::Theme,
            ],
        ));
        for legacy in ["navop.context", "navop.i18n", "navop.theme"] {
            let wildcard = format!("{legacy}.*");
            assert!(
                !capabilities
                    .host
                    .iter()
                    .any(|host| host == legacy || host == &wildcard),
                "旧模块 `{legacy}` 不应派生宿主能力: {:?}",
                capabilities.host
            );
        }
    }

    /// nsh 的 Capabilities 没有 Unix socket 字段：这类权限必须显式丢弃并计入
    /// skipped（调用方据此告警），绝不静默变成"看起来授予了"。
    #[test]
    fn unix_socket_permission_is_dropped_with_explicit_skip() {
        let raw = "net:unix:/var/run/docker.sock";
        let (capabilities, skipped) = granted(&view(&["shell:exec", raw], &[]));
        assert!(
            capabilities.network.sockets.is_empty(),
            "{:?}",
            capabilities.network.sockets
        );
        assert!(
            capabilities.network.hosts.is_empty(),
            "{:?}",
            capabilities.network.hosts
        );
        assert_eq!(skipped, vec![raw.to_string()]);
    }

    #[test]
    fn spawn_permission_grants_execute_by_basename() {
        let (capabilities, skipped) =
            granted(&view(&["shell:exec", "spawn:./bin/docker-provider"], &[]));
        assert!(
            capabilities
                .process
                .execute
                .iter()
                .any(|name| name == "docker-provider")
        );
        assert!(skipped.is_empty(), "{skipped:?}");
    }

    #[test]
    fn wildcard_network_permission_is_skipped_not_granted() {
        let (capabilities, skipped) = granted(&view(&["shell:exec", "net:tcp:*:1"], &[]));
        assert!(capabilities.network.sockets.is_empty());
        assert_eq!(skipped, vec!["net:tcp:*:1".to_string()]);
    }
}
