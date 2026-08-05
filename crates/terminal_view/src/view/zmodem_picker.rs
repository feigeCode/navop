use super::*;
use terminal::zmodem::{ZmodemPickerKind, ZmodemPickerResponse};

impl TerminalView {
    pub(super) fn sync_zmodem_picker(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.terminal.read(cx).zmodem_picker_request() else {
            self.zmodem_picker_request_id = None;
            return;
        };
        let request_id = request.id();
        if self.zmodem_picker_request_id == Some(request_id) {
            return;
        }
        self.zmodem_picker_request_id = Some(request_id);

        let future = cx.prompt_for_paths(zmodem_prompt_options(request.kind()));
        cx.spawn(async move |this, cx| {
            let response = match future.await {
                Ok(Ok(Some(paths))) => picker_response(request.kind(), paths),
                _ => ZmodemPickerResponse::Cancel,
            };
            let _ = this.update(cx, |this, cx| {
                let pending_id = this
                    .terminal
                    .read(cx)
                    .zmodem_picker_request()
                    .map(|pending| pending.id());
                if !active_request_matches(this.zmodem_picker_request_id, pending_id, request_id) {
                    return;
                }
                if this.terminal.read(cx).submit_zmodem_picker(response) {
                    this.zmodem_picker_request_id = None;
                }
            });
        })
        .detach();
    }
}

fn zmodem_prompt_options(kind: ZmodemPickerKind) -> PathPromptOptions {
    match kind {
        ZmodemPickerKind::UploadFiles => PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("TerminalZmodem.select_upload_files").to_string().into()),
        },
        ZmodemPickerKind::DownloadDirectory => PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(
                t!("TerminalZmodem.select_download_directory")
                    .to_string()
                    .into(),
            ),
        },
    }
}

fn picker_response(kind: ZmodemPickerKind, mut paths: Vec<PathBuf>) -> ZmodemPickerResponse {
    match kind {
        ZmodemPickerKind::UploadFiles if !paths.is_empty() => {
            ZmodemPickerResponse::UploadFiles(paths)
        }
        ZmodemPickerKind::DownloadDirectory => paths
            .drain(..)
            .next()
            .map(ZmodemPickerResponse::DownloadDirectory)
            .unwrap_or(ZmodemPickerResponse::Cancel),
        ZmodemPickerKind::UploadFiles => ZmodemPickerResponse::Cancel,
    }
}

fn active_request_matches(active: Option<u64>, pending: Option<u64>, completed: u64) -> bool {
    completed != 0 && active == Some(completed) && pending == Some(completed)
}

#[cfg(test)]
mod tests {
    use super::active_request_matches;

    #[test]
    fn picker_result_only_matches_the_active_pending_request() {
        assert!(active_request_matches(Some(7), Some(7), 7));
        assert!(!active_request_matches(Some(8), Some(7), 7));
        assert!(!active_request_matches(Some(7), Some(8), 7));
        assert!(!active_request_matches(Some(7), None, 7));
        assert!(!active_request_matches(Some(0), Some(0), 0));
    }
}
