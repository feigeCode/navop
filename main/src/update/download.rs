use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request, http};
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

pub(crate) async fn download_update_file<F>(
    http_client: Arc<dyn HttpClient>,
    download_url: &str,
    download_path: &Path,
    mut on_progress: F,
) -> Result<(), String>
where
    F: FnMut(u64, Option<u64>),
{
    if let Some(parent) = download_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|err| format!("创建下载目录失败: {}", err))?;

        // 设置目录权限为仅当前用户可访问，防止 TOCTOU 攻击
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::Permissions::from_mode(0o700);
            std::fs::set_permissions(parent, permissions)
                .map_err(|err| format!("设置下载目录权限失败: {}", err))?;
        }

        // 清理目录中超过 7 天的旧下载文件
        cleanup_old_downloads(parent).await;
    }

    let request = Request::builder()
        .method(Method::GET)
        .uri(download_url)
        .header("Accept", "application/octet-stream")
        .body(AsyncBody::empty())
        .map_err(|err| format!("构建下载请求失败: {}", err))?;

    let response = http_client
        .send(request)
        .await
        .map_err(|err| format!("发送下载请求失败: {}", err))?;

    if !response.status().is_success() {
        return Err(format!("更新包下载失败: {}", response.status()));
    }

    let total_bytes = response
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());

    let mut body = response.into_body();
    let mut file = fs::File::create(download_path)
        .await
        .map_err(|err| format!("创建更新文件失败: {}", err))?;

    let mut downloaded = 0;
    let mut buffer = vec![0u8; 8192];

    loop {
        let read = body
            .read(&mut buffer)
            .await
            .map_err(|err| format!("读取更新数据失败: {}", err))?;
        if read == 0 {
            break;
        }

        file.write_all(&buffer[..read])
            .await
            .map_err(|err| format!("写入更新文件失败: {}", err))?;

        downloaded += read as u64;
        on_progress(downloaded, total_bytes);
    }

    file.flush()
        .await
        .map_err(|err| format!("刷新更新文件失败: {}", err))?;
    file.sync_all()
        .await
        .map_err(|err| format!("同步更新文件失败: {}", err))?;

    #[cfg(unix)]
    super::install::set_executable_permission(download_path)?;

    Ok(())
}

pub(crate) fn build_download_path(version: &str, download_url: &str) -> Result<PathBuf, String> {
    let file_name = download_file_name(version, download_url);
    let dir = std::env::temp_dir().join("onetcli-update");
    Ok(dir.join(file_name))
}

fn download_file_name(version: &str, download_url: &str) -> String {
    let parsed = http::Uri::try_from(download_url).ok();
    let extension = parsed
        .and_then(|uri| uri.path().rsplit('/').next().map(|path| path.to_string()))
        .and_then(|name| {
            Path::new(&name)
                .extension()
                .map(|extension| extension.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| {
            #[cfg(target_os = "windows")]
            {
                return "exe".to_string();
            }
            #[allow(unreachable_code)]
            String::new()
        });

    let base_name = format!("onetcli-update-{}", version.replace('/', "-"));
    if extension.is_empty() {
        base_name
    } else {
        format!("{}.{}", base_name, extension)
    }
}

/// 校验下载文件的 SHA256 哈希值。
/// 使用同步文件读取——下载文件为本地文件且体积有限，无需异步。
pub(crate) fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|err| format!("读取下载文件失败: {}", err))?;

    let hash = Sha256::digest(&data);
    let actual = format!("{:x}", hash);
    let expected_lower = expected.trim().to_lowercase();

    if actual != expected_lower {
        return Err(format!(
            "SHA256 校验失败: 期望 {}，实际 {}",
            expected_lower, actual
        ));
    }

    Ok(())
}

async fn cleanup_old_downloads(dir: &Path) {
    let Ok(mut entries) = fs::read_dir(dir).await else {
        return;
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.is_file() {
            if let Ok(metadata) = fs::metadata(&path).await {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = modified.elapsed() {
                        if age > std::time::Duration::from_secs(7 * 24 * 3600) {
                            let _ = fs::remove_file(&path).await;
                        }
                    }
                }
            }
        }
    }
}
