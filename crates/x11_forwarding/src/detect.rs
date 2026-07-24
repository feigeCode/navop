//! 探测本机 X11 运行环境，构造连接级 [`X11Proxy`]。
//!
//! 探测顺序：DISPLAY（macOS 下 GUI 进程通常没有该环境变量，回退到
//! `launchctl getenv DISPLAY`；部分 XQuartz 版本不会把 DISPLAY 写入
//! launchd 环境，此时再扫描 `/tmp/.X11-unix/X*`）→ 解析出本机端点
//! → 读取 Xauthority 文件并挑出匹配 DISPLAY 的 MIT-MAGIC-COOKIE-1。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

use crate::xauthority::{self, HostHints};
use crate::{DisplayAddress, MagicCookie, ServerEndpoint, X11Error, X11Proxy, X11Result};

pub fn detect_local_server() -> X11Result<X11Proxy> {
    let display_text = discover_display_string().ok_or(X11Error::DisplayNotFound)?;
    let address = DisplayAddress::parse(&display_text)?;

    let hints = HostHints {
        hostname: query_hostname(),
        ips: Vec::new(),
    };
    let cookie = match find_authority_cookie(&address, &hints) {
        Ok(cookie) => cookie,
        Err(initial_error) => {
            retry_xquartz_authority(&display_text, &address, &hints).map_err(|retry_error| {
                if matches!(retry_error, X11Error::AuthorityNoMatch) {
                    initial_error
                } else {
                    retry_error
                }
            })?
        }
    };

    Ok(X11Proxy::new(
        address.endpoint().clone(),
        address.screen(),
        cookie,
    ))
}

fn find_authority_cookie(address: &DisplayAddress, hints: &HostHints) -> X11Result<MagicCookie> {
    let authority_paths = authority_files();
    if authority_paths.is_empty() {
        return Err(X11Error::AuthorityUnreadable(
            "cannot locate an Xauthority file".into(),
        ));
    }

    let mut readable = false;
    let mut failures = Vec::new();
    let mut cookie = None;
    for authority_path in authority_paths {
        let raw = match std::fs::read(&authority_path) {
            Ok(raw) => {
                readable = true;
                raw
            }
            Err(error) => {
                failures.push(format!("{}: {error}", authority_path.display()));
                continue;
            }
        };
        match xauthority::best_cookie(xauthority::records_of(&raw), &address, &hints) {
            Ok(found) => {
                cookie = Some(found);
                break;
            }
            Err(error) => failures.push(format!("{}: {error}", authority_path.display())),
        }
    }
    cookie.ok_or_else(|| {
        if readable {
            X11Error::AuthorityNoMatch
        } else {
            X11Error::AuthorityUnreadable(failures.join("; "))
        }
    })
}

#[cfg(target_os = "macos")]
fn retry_xquartz_authority(
    display_text: &str,
    address: &DisplayAddress,
    hints: &HostHints,
) -> X11Result<MagicCookie> {
    let ServerEndpoint::Unix(endpoint) = address.endpoint() else {
        return Err(X11Error::AuthorityNoMatch);
    };
    let endpoint_text = endpoint.to_string_lossy();
    if !endpoint_text.contains("org.xquartz") && !endpoint_text.contains(".X11-unix/X") {
        return Err(X11Error::AuthorityNoMatch);
    }

    // XQuartz 的 launchd DISPLAY 套接字只有在首次连接后才真正拉起 X server，
    // 随后才会生成 ~/.serverauth.<pid>。先用 xdpyinfo 完成一次合法握手，
    // 再短暂轮询动态 authority 文件；命令放在调用方的 spawn_blocking 中执行。
    let mut wake = std::process::Command::new("/opt/X11/bin/xdpyinfo")
        .args(["-display", display_text])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok();
    let wake_deadline = Instant::now() + Duration::from_secs(5);
    while let Some(child) = wake.as_mut() {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        if Instant::now() >= wake_deadline {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match find_authority_cookie(address, hints) {
            Ok(cookie) => return Ok(cookie),
            Err(error) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
                tracing::trace!(
                    target: "x11.detect",
                    error = %error,
                    "等待 XQuartz 生成 authority cookie"
                );
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn retry_xquartz_authority(
    _display_text: &str,
    _address: &DisplayAddress,
    _hints: &HostHints,
) -> X11Result<MagicCookie> {
    Err(X11Error::AuthorityNoMatch)
}

fn discover_display_string() -> Option<String> {
    env_non_empty("DISPLAY")
        .or_else(launchd_display)
        .or_else(local_xquartz_display)
}

#[cfg(target_os = "macos")]
fn launchd_display() -> Option<String> {
    command_output("launchctl", &["getenv", "DISPLAY"])
}

#[cfg(not(target_os = "macos"))]
fn launchd_display() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn local_xquartz_display() -> Option<String> {
    use std::os::unix::fs::FileTypeExt as _;

    let entries = std::fs::read_dir("/tmp/.X11-unix").ok()?;
    let display = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .map(|file_type| file_type.is_socket())
                .unwrap_or(false)
        })
        .filter_map(|entry| display_number_from_socket_name(&entry.file_name()))
        .min()?;
    Some(format!(":{display}"))
}

#[cfg(not(target_os = "macos"))]
fn local_xquartz_display() -> Option<String> {
    None
}

fn display_number_from_socket_name(name: &std::ffi::OsStr) -> Option<u16> {
    name.to_str()?.strip_prefix('X')?.parse().ok()
}

fn authority_files() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = std::env::var_os("XAUTHORITY").filter(|value| !value.is_empty()) {
        paths.push(PathBuf::from(path));
    }
    if let Some(home) = std::env::home_dir() {
        paths.push(home.join(".Xauthority"));
        paths.extend(xquartz_server_authority_files(&home));
    }

    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
    paths
}

#[cfg(target_os = "macos")]
fn xquartz_server_authority_files(home: &Path) -> Vec<PathBuf> {
    let mut paths = std::fs::read_dir(home)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            name.to_str()
                .is_some_and(|name| name.starts_with(".serverauth."))
                .then(|| {
                    let modified = entry
                        .metadata()
                        .and_then(|metadata| metadata.modified())
                        .ok();
                    (entry.path(), modified)
                })
        })
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| right.1.cmp(&left.1));
    paths.into_iter().map(|(path, _)| path).collect()
}

#[cfg(not(target_os = "macos"))]
fn xquartz_server_authority_files(_home: &Path) -> Vec<PathBuf> {
    Vec::new()
}

fn query_hostname() -> Option<String> {
    command_output("hostname", &[])
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use super::display_number_from_socket_name;
    #[cfg(target_os = "macos")]
    use super::{detect_local_server, xquartz_server_authority_files};
    use std::ffi::OsStr;

    #[test]
    fn parses_x11_unix_socket_names() {
        assert_eq!(display_number_from_socket_name(OsStr::new("X0")), Some(0));
        assert_eq!(display_number_from_socket_name(OsStr::new("X12")), Some(12));
        assert_eq!(display_number_from_socket_name(OsStr::new("X")), None);
        assert_eq!(display_number_from_socket_name(OsStr::new("not-x11")), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn discovers_xquartz_server_authority_files() {
        let home = std::env::temp_dir().join(format!(
            "navop-x11-authority-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".serverauth.123"), b"authority").unwrap();
        std::fs::write(home.join(".Xauthority"), b"default").unwrap();
        std::fs::write(home.join(".serverauth.invalid"), b"authority").unwrap();

        let paths = xquartz_server_authority_files(&home);

        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".serverauth.")
        }));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires a locally installed and running XQuartz"]
    fn detects_running_xquartz() {
        detect_local_server().expect("running XQuartz should be detected");
    }
}
