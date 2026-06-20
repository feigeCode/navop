use std::sync::{Arc, Mutex};

use ironrdp::cliprdr::backend::{ClipboardMessage, CliprdrBackend, CliprdrBackendFactory};
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp::core::{AsAny, IntoOwned};
use ironrdp_client::rdp::RdpInputEvent;
use tokio::sync::mpsc;

use crate::protocol::HelperEvent;

#[derive(Clone, Debug)]
pub struct TextClipboardController {
    shared: Arc<Mutex<TextClipboardState>>,
    input_tx: mpsc::UnboundedSender<RdpInputEvent>,
}

impl TextClipboardController {
    pub fn set_local_text(&self, text: String) -> anyhow::Result<()> {
        self.shared.lock().expect("clipboard mutex").local_text = Some(text);
        self.send_clipboard(ClipboardMessage::SendInitiateCopy(text_formats()))
    }

    fn send_clipboard(&self, message: ClipboardMessage) -> anyhow::Result<()> {
        self.input_tx
            .send(RdpInputEvent::Clipboard(message))
            .map_err(|_| anyhow::anyhow!("RDP input channel closed"))
    }
}

#[derive(Debug, Default)]
struct TextClipboardState {
    local_text: Option<String>,
}

#[derive(Clone, Debug)]
struct TextClipboardBackendFactory {
    shared: Arc<Mutex<TextClipboardState>>,
    input_tx: mpsc::UnboundedSender<RdpInputEvent>,
    output_tx: std::sync::mpsc::Sender<HelperEvent>,
}

impl CliprdrBackendFactory for TextClipboardBackendFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(TextClipboardBackend {
            shared: self.shared.clone(),
            input_tx: self.input_tx.clone(),
            output_tx: self.output_tx.clone(),
            temporary_directory: std::env::temp_dir().to_string_lossy().to_string(),
        })
    }
}

#[derive(Debug)]
struct TextClipboardBackend {
    shared: Arc<Mutex<TextClipboardState>>,
    input_tx: mpsc::UnboundedSender<RdpInputEvent>,
    output_tx: std::sync::mpsc::Sender<HelperEvent>,
    temporary_directory: String,
}

impl AsAny for TextClipboardBackend {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

impl TextClipboardBackend {
    fn send_clipboard(&self, message: ClipboardMessage) {
        let _ = self.input_tx.send(RdpInputEvent::Clipboard(message));
    }

    fn send_local_text_response(&self, request: FormatDataRequest) {
        let response = if request.format == ClipboardFormatId::CF_UNICODETEXT {
            self.shared
                .lock()
                .expect("clipboard mutex")
                .local_text
                .as_deref()
                .map(FormatDataResponse::new_unicode_string)
                .unwrap_or_else(FormatDataResponse::new_error)
        } else {
            FormatDataResponse::new_error()
        };
        self.send_clipboard(ClipboardMessage::SendFormatData(response.into_owned()));
    }
}

impl CliprdrBackend for TextClipboardBackend {
    fn temporary_directory(&self) -> &str {
        &self.temporary_directory
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        if self
            .shared
            .lock()
            .expect("clipboard mutex")
            .local_text
            .is_some()
        {
            self.send_clipboard(ClipboardMessage::SendInitiateCopy(text_formats()));
        }
    }

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        if available_formats
            .iter()
            .any(|format| format.id() == ClipboardFormatId::CF_UNICODETEXT)
        {
            self.send_clipboard(ClipboardMessage::SendInitiatePaste(
                ClipboardFormatId::CF_UNICODETEXT,
            ));
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.send_local_text_response(request);
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            return;
        }
        if let Ok(text) = response.to_unicode_string() {
            let _ = self.output_tx.send(HelperEvent::ClipboardText { text });
        }
    }

    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}

    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _: LockDataId) {}

    fn on_unlock(&mut self, _: LockDataId) {}
}

pub fn text_clipboard(
    input_tx: mpsc::UnboundedSender<RdpInputEvent>,
    output_tx: std::sync::mpsc::Sender<HelperEvent>,
) -> (
    TextClipboardController,
    Box<dyn CliprdrBackendFactory + Send>,
) {
    let shared = Arc::new(Mutex::new(TextClipboardState::default()));
    let controller = TextClipboardController {
        shared: shared.clone(),
        input_tx: input_tx.clone(),
    };
    let factory = TextClipboardBackendFactory {
        shared,
        input_tx,
        output_tx,
    };
    (controller, Box::new(factory))
}

fn text_formats() -> Vec<ClipboardFormat> {
    vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp::cliprdr::backend::ClipboardMessage;
    use ironrdp::cliprdr::pdu::{ClipboardFormatId, FormatDataRequest, FormatDataResponse};
    use ironrdp_client::rdp::RdpInputEvent;

    use crate::protocol::HelperEvent;

    #[test]
    fn local_text_advertises_unicode_clipboard_format() {
        let (input_tx, mut input_rx) = RdpInputEvent::create_channel();
        let (output_tx, _output_rx) = std::sync::mpsc::channel::<HelperEvent>();
        let (controller, _factory) = text_clipboard(input_tx, output_tx);

        controller
            .set_local_text("hello 中文".to_string())
            .expect("local clipboard advertises");

        match input_rx.try_recv().expect("clipboard message") {
            RdpInputEvent::Clipboard(ClipboardMessage::SendInitiateCopy(formats)) => {
                assert_eq!(
                    vec![ClipboardFormatId::CF_UNICODETEXT],
                    format_ids(&formats)
                );
            }
            other => panic!("expected clipboard advertise, got {other:?}"),
        }
    }

    #[test]
    fn backend_replies_with_local_text_when_remote_requests_unicode_data() {
        let (input_tx, mut input_rx) = RdpInputEvent::create_channel();
        let (output_tx, _output_rx) = std::sync::mpsc::channel::<HelperEvent>();
        let (controller, factory) = text_clipboard(input_tx, output_tx);
        controller
            .set_local_text("hello 中文".to_string())
            .expect("local clipboard advertises");
        let _ = input_rx.try_recv();
        let mut backend = factory.build_cliprdr_backend();

        backend.on_format_data_request(FormatDataRequest {
            format: ClipboardFormatId::CF_UNICODETEXT,
        });

        match input_rx.try_recv().expect("format data response") {
            RdpInputEvent::Clipboard(ClipboardMessage::SendFormatData(response)) => {
                assert_eq!(
                    "hello 中文",
                    response.to_unicode_string().expect("unicode text decodes")
                );
            }
            other => panic!("expected clipboard data response, got {other:?}"),
        }
    }

    #[test]
    fn backend_fetches_and_emits_remote_unicode_clipboard_text() {
        let (input_tx, mut input_rx) = RdpInputEvent::create_channel();
        let (output_tx, output_rx) = std::sync::mpsc::channel::<HelperEvent>();
        let (_controller, factory) = text_clipboard(input_tx, output_tx);
        let mut backend = factory.build_cliprdr_backend();

        backend.on_remote_copy(&[ironrdp::cliprdr::pdu::ClipboardFormat::new(
            ClipboardFormatId::CF_UNICODETEXT,
        )]);

        match input_rx.try_recv().expect("paste request") {
            RdpInputEvent::Clipboard(ClipboardMessage::SendInitiatePaste(format)) => {
                assert_eq!(ClipboardFormatId::CF_UNICODETEXT, format);
            }
            other => panic!("expected paste request, got {other:?}"),
        }

        backend.on_format_data_response(FormatDataResponse::new_unicode_string("remote 中文"));

        assert_eq!(
            HelperEvent::ClipboardText {
                text: "remote 中文".to_string()
            },
            output_rx.try_recv().expect("clipboard event")
        );
    }

    fn format_ids(formats: &[ironrdp::cliprdr::pdu::ClipboardFormat]) -> Vec<ClipboardFormatId> {
        formats.iter().map(|format| format.id()).collect()
    }
}
