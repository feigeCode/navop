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
        let Some(claim) = self.terminal.read(cx).claim_zmodem_picker(request_id) else {
            return;
        };
        self.zmodem_picker_request_id = Some(request_id);

        let future = cx.prompt_for_paths(zmodem_prompt_options(request.kind()));
        cx.spawn(async move |this, cx| {
            let response = match future.await {
                Ok(Ok(Some(paths))) => picker_response(request.kind(), paths),
                _ => ZmodemPickerResponse::Cancel,
            };
            let _ = this.update(cx, |this, _cx| {
                if this.zmodem_picker_request_id != Some(request_id) {
                    return;
                }
                if claim.submit(response) {
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
