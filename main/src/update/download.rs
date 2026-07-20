use std::path::Path;
use std::sync::Arc;

use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request, http};
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

const STALE_DOWNLOAD_AGE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);

#[path = "download_path.rs"]
mod download_path;

pub(crate) use download_path::build_download_path;
use download_path::{partial_download_path, update_root_for_download};

pub(crate) async fn download_update_file<F>(
    http_client: Arc<dyn HttpClient>,
    download_url: &str,
    download_path: &Path,
    mut on_progress: F,
) -> Result<(), String>
where
    F: FnMut(u64, Option<u64>),
{
    prepare_download_directory(download_path).await?;
    let partial_path = partial_download_path(download_path)?;
    let result = download_to_file(http_client, download_url, &partial_path, &mut on_progress).await;

    if let Err(err) = result {
        let _ = fs::remove_file(&partial_path).await;
        return Err(err);
    }

    if let Err(err) = fs::rename(&partial_path, download_path).await {
        let _ = fs::remove_file(&partial_path).await;
        return Err(format!("提交更新文件失败: {err}"));
    }

    Ok(())
}

async fn download_to_file<F>(
    http_client: Arc<dyn HttpClient>,
    download_url: &str,
    destination: &Path,
    on_progress: &mut F,
) -> Result<(), String>
where
    F: FnMut(u64, Option<u64>),
{
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
    let mut file = fs::File::create(destination)
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

    Ok(())
}

pub(crate) async fn download_update_file_from_sources<F>(
    http_client: Arc<dyn HttpClient>,
    download_urls: &[String],
    download_path: &Path,
    mut on_progress: F,
) -> Result<(), String>
where
    F: FnMut(u64, Option<u64>),
{
    let mut last_error = None;
    for download_url in download_urls {
        match download_update_file(
            http_client.clone(),
            download_url,
            download_path,
            |done, total| {
                on_progress(done, total);
            },
        )
        .await
        {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| "缺少可用的更新下载源".to_string()))
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
        if !is_stale_download(&path).await {
            continue;
        }

        if path.is_dir() {
            let _ = fs::remove_dir_all(&path).await;
        } else {
            let _ = fs::remove_file(&path).await;
        }
    }
}

async fn prepare_download_directory(download_path: &Path) -> Result<(), String> {
    let parent = download_path
        .parent()
        .ok_or_else(|| format!("更新文件缺少父目录: {}", download_path.display()))?;
    fs::create_dir_all(parent)
        .await
        .map_err(|err| format!("创建下载目录失败: {err}"))?;
    set_private_directory_permissions(parent)?;

    if let Some(root) = update_root_for_download(download_path) {
        set_private_directory_permissions(root)?;
        cleanup_old_downloads(root).await;
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = std::fs::Permissions::from_mode(0o700);
    std::fs::set_permissions(path, permissions)
        .map_err(|err| format!("设置下载目录权限失败: {err}"))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

async fn is_stale_download(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path).await else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    modified.elapsed().is_ok_and(|age| age > STALE_DOWNLOAD_AGE)
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;
