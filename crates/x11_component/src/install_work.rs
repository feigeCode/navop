//! 下载后的 CPU 与目录 I/O 集中在 blocking 阶段执行。
use std::path::{Path, PathBuf};

use rand::{RngCore as _, rngs::OsRng};
use sha2::{Digest as _, Sha256};

use crate::X11ComponentError;
use crate::manifest::Artifact;

pub async fn unpack_blocking(
    payload: Vec<u8>,
    artifact: Artifact,
    public_key_base64: String,
    staging: PathBuf,
) -> Result<(), X11ComponentError> {
    tokio::task::spawn_blocking(move || {
        unpack_sync(&payload, &artifact, &public_key_base64, &staging)
    })
    .await
    .map_err(join_error)?
}

pub async fn unpack_embedded_blocking(
    payload: &'static [u8],
    artifact: Artifact,
    public_key_base64: String,
    staging: PathBuf,
) -> Result<(), X11ComponentError> {
    tokio::task::spawn_blocking(move || {
        unpack_sync(payload, &artifact, &public_key_base64, &staging)
    })
    .await
    .map_err(join_error)?
}

pub async fn commit_blocking(
    staging: PathBuf,
    root: PathBuf,
    version: String,
) -> Result<(), X11ComponentError> {
    tokio::task::spawn_blocking(move || crate::installer::commit_sync(&staging, &root, &version))
        .await
        .map_err(join_error)?
}

fn unpack_sync(
    payload: &[u8],
    artifact: &Artifact,
    public_key_base64: &str,
    staging: &Path,
) -> Result<(), X11ComponentError> {
    let actual = hex::encode(Sha256::digest(payload));
    if actual != artifact.sha256 {
        return Err(X11ComponentError::Sha256Mismatch {
            expected: artifact.sha256.clone(),
            actual,
        });
    }
    crate::verify_ed25519(payload, &artifact.signature, public_key_base64)?;
    crate::archive::unpack(payload, artifact, staging)?;
    Ok(())
}

pub async fn cleanup_blocking(path: PathBuf) {
    let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(path)).await;
}

pub fn schedule_cleanup(path: PathBuf) {
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(cleanup_blocking(path));
    } else {
        let _ = std::thread::Builder::new()
            .name("x11-staging-cleanup".into())
            .spawn(move || {
                let _ = std::fs::remove_dir_all(path);
            });
    }
}

pub fn replace_destination_sync(
    staging: &Path,
    destination: &Path,
) -> Result<(), X11ComponentError> {
    if !destination.exists() {
        return std::fs::rename(staging, destination).map_err(install_error);
    }
    let backup = backup_path(destination);
    std::fs::rename(destination, &backup).map_err(install_error)?;
    if let Err(error) = std::fs::rename(staging, destination) {
        let _ = std::fs::rename(&backup, destination);
        return Err(install_error(error));
    }
    std::fs::remove_dir_all(backup).map_err(install_error)
}

fn backup_path(destination: &Path) -> PathBuf {
    let mut random = [0; 8];
    OsRng.fill_bytes(&mut random);
    destination.with_extension(format!("replaced-{}", hex::encode(random)))
}

fn join_error(error: tokio::task::JoinError) -> X11ComponentError {
    X11ComponentError::Install(format!("安装 blocking 阶段失败：{error}"))
}

fn install_error(error: std::io::Error) -> X11ComponentError {
    X11ComponentError::Install(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::X11ComponentError;
    use crate::manifest::{Artifact, ArtifactFormat};

    #[tokio::test(flavor = "current_thread")]
    async fn sha256_does_not_block_the_async_worker() {
        let artifact = Artifact {
            version: "1.9.0".into(),
            url: String::new(),
            sha256: "00".repeat(32),
            signature: String::new(),
            min_vulkan: "1.3.0".into(),
            format: ArtifactFormat::Binary,
            executable_path: "yserver".into(),
        };
        let staging = std::env::temp_dir().join("x11-blocking-test");
        let work = tokio::spawn(super::unpack_blocking(
            vec![7; 64 * 1024 * 1024],
            artifact,
            String::new(),
            staging,
        ));
        let timer = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(5)).await;
        });
        tokio::time::timeout(Duration::from_millis(20), timer)
            .await
            .expect("同步哈希会阻塞 current_thread worker")
            .unwrap();
        assert!(matches!(
            work.await.unwrap(),
            Err(X11ComponentError::Sha256Mismatch { .. })
        ));
    }
}
