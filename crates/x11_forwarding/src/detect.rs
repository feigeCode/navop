//! 探测本机 X11 运行环境，构造连接级 [`X11Proxy`]。
//!
//! 探测顺序：DISPLAY（macOS 下 GUI 进程通常没有该环境变量，回退到
//! `launchctl getenv DISPLAY`；XQuartz 注册的是 launchd 套接字，首个
//! 连接会自动拉起服务）→ 解析出本机端点 → 读取 Xauthority 文件并
//! 挑出匹配 DISPLAY 的 MIT-MAGIC-COOKIE-1。

use std::path::PathBuf;

use crate::xauthority::{self, HostHints};
use crate::{DisplayAddress, X11Error, X11Proxy, X11Result};

pub fn detect_local_server() -> X11Result<X11Proxy> {
    let display_text = discover_display_string().ok_or(X11Error::DisplayNotFound)?;
    let address = DisplayAddress::parse(&display_text)?;

    let authority_path = authority_file().ok_or(X11Error::AuthorityUnreadable(
        "cannot locate ~/.Xauthority".into(),
    ))?;
    let raw = std::fs::read(&authority_path).map_err(|error| {
        X11Error::AuthorityUnreadable(format!("{}: {error}", authority_path.display()))
    })?;

    let hints = HostHints {
        hostname: query_hostname(),
        ips: Vec::new(),
    };
    let cookie = xauthority::best_cookie(xauthority::records_of(&raw), &address, &hints)?;

    Ok(X11Proxy::new(
        address.endpoint().clone(),
        address.screen(),
        cookie,
    ))
}

fn discover_display_string() -> Option<String> {
    env_non_empty("DISPLAY").or_else(launchd_display)
}

#[cfg(target_os = "macos")]
fn launchd_display() -> Option<String> {
    command_output("launchctl", &["getenv", "DISPLAY"])
}

#[cfg(not(target_os = "macos"))]
fn launchd_display() -> Option<String> {
    None
}

fn authority_file() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XAUTHORITY").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(path));
    }
    std::env::home_dir().map(|home| home.join(".Xauthority"))
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
