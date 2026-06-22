use std::sync::Mutex;

use extension_runtime::extension_downloader::{DownloadProgress, DownloadProgressCallback};

static MCP_HELPER_INSTALL_PROGRESS: Mutex<McpHelperInstallProgressSnapshot> =
    Mutex::new(McpHelperInstallProgressSnapshot::idle());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum McpHelperInstallProgressStatus {
    Idle,
    Preparing,
    Connecting,
    Downloading { percent: Option<u8> },
    Retrying,
    Installing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct McpHelperInstallProgressSnapshot {
    status: McpHelperInstallProgressStatus,
}

impl McpHelperInstallProgressSnapshot {
    pub(crate) const fn idle() -> Self {
        Self {
            status: McpHelperInstallProgressStatus::Idle,
        }
    }

    pub(crate) const fn preparing() -> Self {
        Self {
            status: McpHelperInstallProgressStatus::Preparing,
        }
    }

    pub(crate) fn status(self) -> McpHelperInstallProgressStatus {
        self.status
    }

    pub(crate) fn apply_download_progress(&mut self, progress: DownloadProgress) {
        self.status = match progress {
            DownloadProgress::Started { .. } => McpHelperInstallProgressStatus::Connecting,
            DownloadProgress::Bytes { downloaded, total } => {
                McpHelperInstallProgressStatus::Downloading {
                    percent: download_percent(downloaded, total),
                }
            }
            DownloadProgress::Failed { retrying: true, .. } => {
                McpHelperInstallProgressStatus::Retrying
            }
            DownloadProgress::Failed {
                retrying: false, ..
            } => McpHelperInstallProgressStatus::Preparing,
            DownloadProgress::Finished => McpHelperInstallProgressStatus::Installing,
        };
    }
}

pub(crate) fn begin_helper_install_progress() {
    set_progress(McpHelperInstallProgressSnapshot::preparing());
}

pub(crate) fn clear_helper_install_progress() {
    set_progress(McpHelperInstallProgressSnapshot::idle());
}

pub(crate) fn helper_install_progress_snapshot() -> McpHelperInstallProgressSnapshot {
    MCP_HELPER_INSTALL_PROGRESS
        .lock()
        .map(|progress| *progress)
        .unwrap_or_else(|_| McpHelperInstallProgressSnapshot::idle())
}

pub(crate) fn helper_install_progress_callback() -> DownloadProgressCallback {
    std::sync::Arc::new(|progress| {
        if let Ok(mut snapshot) = MCP_HELPER_INSTALL_PROGRESS.lock() {
            snapshot.apply_download_progress(progress);
        }
    })
}

fn set_progress(progress: McpHelperInstallProgressSnapshot) {
    if let Ok(mut snapshot) = MCP_HELPER_INSTALL_PROGRESS.lock() {
        *snapshot = progress;
    }
}

fn download_percent(downloaded: u64, total: Option<u64>) -> Option<u8> {
    let total = total.filter(|total| *total > 0)?;
    let percent = ((downloaded as f64 / total as f64) * 100.0).round();
    Some(percent.clamp(0.0, 100.0) as u8)
}

#[cfg(test)]
mod tests {
    use extension_runtime::extension_downloader::DownloadProgress;

    #[test]
    fn helper_install_progress_tracks_download_and_installing_states() {
        let mut progress = super::McpHelperInstallProgressSnapshot::preparing();
        assert_eq!(
            super::McpHelperInstallProgressStatus::Preparing,
            progress.status()
        );

        progress.apply_download_progress(DownloadProgress::Started {
            url: "https://example.test/helper.tar.gz".to_string(),
        });
        assert_eq!(
            super::McpHelperInstallProgressStatus::Connecting,
            progress.status()
        );

        progress.apply_download_progress(DownloadProgress::Bytes {
            downloaded: 25,
            total: Some(100),
        });
        assert_eq!(
            super::McpHelperInstallProgressStatus::Downloading { percent: Some(25) },
            progress.status()
        );

        progress.apply_download_progress(DownloadProgress::Finished);
        assert_eq!(
            super::McpHelperInstallProgressStatus::Installing,
            progress.status()
        );
    }

    #[test]
    fn helper_install_progress_reports_retrying_failed_source() {
        let mut progress = super::McpHelperInstallProgressSnapshot::preparing();

        progress.apply_download_progress(DownloadProgress::Failed {
            url: "https://example.test/helper.tar.gz".to_string(),
            error: "HTTP 500".to_string(),
            retrying: true,
        });

        assert_eq!(
            super::McpHelperInstallProgressStatus::Retrying,
            progress.status()
        );
    }
}
