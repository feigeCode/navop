use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{HtmlPreviewDocument, resolve_extension_asset_url};

const EXTENSION_ASSET_SCHEME: &str = "onet-extension://";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserOpenCommand {
    pub program: String,
    pub args: Vec<String>,
}

pub fn download_html_preview_document(document: &HtmlPreviewDocument) -> io::Result<PathBuf> {
    let dir = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or(std::env::current_dir()?);
    write_html_preview_document_to_dir(document, dir)
}

pub fn write_html_preview_document_to_dir(
    document: &HtmlPreviewDocument,
    dir: impl AsRef<Path>,
) -> io::Result<PathBuf> {
    write_browser_html_preview_document_to_dir(document, dir)
}

pub fn write_browser_html_preview_document_to_dir(
    document: &HtmlPreviewDocument,
    dir: impl AsRef<Path>,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir.as_ref())?;
    let path = available_html_preview_path(dir.as_ref());
    std::fs::write(&path, browser_ready_html_preview_document(document))?;
    Ok(path)
}

pub fn browser_ready_html_preview_document(document: &HtmlPreviewDocument) -> String {
    rewrite_extension_asset_urls(document.render_html())
}

pub fn open_html_preview_document_in_browser(
    document: &HtmlPreviewDocument,
) -> io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("onetcli-html-preview");
    let path = write_browser_html_preview_document_to_dir(document, dir)?;
    let command = system_browser_open_command(&path);
    Command::new(&command.program).args(&command.args).spawn()?;
    Ok(path)
}

pub fn system_browser_open_command(path: &Path) -> BrowserOpenCommand {
    let path = path.display().to_string();
    if cfg!(target_os = "macos") {
        BrowserOpenCommand {
            program: "open".to_string(),
            args: vec![path],
        }
    } else if cfg!(target_os = "windows") {
        BrowserOpenCommand {
            program: "cmd".to_string(),
            args: vec!["/C".into(), "start".into(), "".into(), path],
        }
    } else {
        BrowserOpenCommand {
            program: "xdg-open".to_string(),
            args: vec![path],
        }
    }
}

fn rewrite_extension_asset_urls(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(index) = rest.find(EXTENSION_ASSET_SCHEME) {
        output.push_str(&rest[..index]);
        let url_and_tail = &rest[index..];
        let url_end = url_and_tail
            .find(is_url_delimiter)
            .unwrap_or(url_and_tail.len());
        let url = &url_and_tail[..url_end];
        output.push_str(&file_url_for_extension_asset(url).unwrap_or_else(|| url.to_string()));
        rest = &url_and_tail[url_end..];
    }
    output.push_str(rest);
    output
}

fn is_url_delimiter(ch: char) -> bool {
    matches!(
        ch,
        '"' | '\'' | '<' | '>' | '(' | ')' | '`' | ',' | ';' | ' ' | '\n' | '\r' | '\t'
    )
}

fn file_url_for_extension_asset(url: &str) -> Option<String> {
    let path = resolve_extension_asset_url(url)?;
    Some(file_url_for_path(&path))
}

fn file_url_for_path(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    if cfg!(target_os = "windows") {
        format!("file:///{}", percent_encode_path(&path))
    } else {
        format!("file://{}", percent_encode_path(&path))
    }
}

fn percent_encode_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.as_bytes() {
        let ch = *byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '/' | ':' | '-' | '_' | '.' | '~') {
            encoded.push(ch);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn available_html_preview_path(dir: &Path) -> PathBuf {
    let first = dir.join("onetcli-html-preview.html");
    if !first.exists() {
        return first;
    }
    for index in 1.. {
        let candidate = dir.join(format!("onetcli-html-preview-{index}.html"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}
