use gpui::{
    AnyWindowHandle, App, AsyncApp, Context, Image, ImageFormat, IntoElement, ObjectFit,
    ParentElement, Render, Styled, Window, div, img, prelude::*, px, relative,
};
use gpui_component::{
    ActiveTheme, WindowExt, notification::Notification, scroll::ScrollableElement,
};
use one_core::gpui_tokio::Tokio;
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use sftp::{RusshSftpClient, SftpClient};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

const MAX_REMOTE_IMAGE_PREVIEW_BYTES: usize = 25 * 1024 * 1024;

struct RemoteImagePreview {
    image: Arc<Image>,
}

impl RemoteImagePreview {
    fn new(image: Image) -> Self {
        Self {
            image: Arc::new(image),
        }
    }
}

impl Render for RemoteImagePreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .size_full()
            .min_h(px(0.0))
            .bg(theme.muted.opacity(0.20))
            .overflow_scrollbar()
            .child(
                div()
                    .min_w(relative(1.0))
                    .min_h(relative(1.0))
                    .p_4()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        img(self.image.clone())
                            .max_w(relative(1.0))
                            .object_fit(ObjectFit::Contain),
                    ),
            )
    }
}

pub fn image_format_for_path(path: &str) -> Option<ImageFormat> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    image_format_for_extension(&ext)
}

pub fn image_format_for_local_path(path: &Path) -> Option<ImageFormat> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    image_format_for_extension(&ext)
}

pub fn image_from_local_path(path: &Path) -> Option<Image> {
    let format = image_format_for_local_path(path)?;
    let bytes = std::fs::read(path).ok()?;
    (!bytes.is_empty()).then(|| Image::from_bytes(format, bytes))
}

pub fn open_remote_image_preview<T: 'static>(
    remote_path: String,
    client: Arc<Mutex<RusshSftpClient>>,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let Some(format) = image_format_for_path(&remote_path) else {
        return;
    };

    let window_handle = window.window_handle();
    let task_path = remote_path.clone();
    let read_task = Tokio::spawn(cx, async move {
        let bytes = client
            .lock()
            .await
            .read_file(&task_path, MAX_REMOTE_IMAGE_PREVIEW_BYTES)
            .await?;
        Ok::<_, anyhow::Error>((task_path, bytes))
    });

    window.push_notification(
        Notification::info("正在读取远程图片...".to_string()).autohide(true),
        cx,
    );

    cx.spawn(async move |_this, cx| match read_task.await {
        Ok(Ok((path, bytes))) => {
            cx.update(|cx| {
                open_remote_image_preview_window(path, format, bytes, cx);
            });
        }
        Ok(Err(error)) => {
            notify_remote_image_preview_error(window_handle, error.to_string(), cx);
        }
        Err(error) => {
            notify_remote_image_preview_error(window_handle, error.to_string(), cx);
        }
    })
    .detach();
}

pub fn format_image_preview_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn image_format_for_extension(ext: &str) -> Option<ImageFormat> {
    match ext {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "webp" => Some(ImageFormat::Webp),
        "gif" => Some(ImageFormat::Gif),
        "bmp" => Some(ImageFormat::Bmp),
        "tif" | "tiff" => Some(ImageFormat::Tiff),
        "ico" => Some(ImageFormat::Ico),
        "svg" => Some(ImageFormat::Svg),
        "pnm" => Some(ImageFormat::Pnm),
        _ => None,
    }
}

fn open_remote_image_preview_window(
    remote_path: String,
    format: ImageFormat,
    bytes: Vec<u8>,
    cx: &mut App,
) {
    let title = remote_path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("Remote Image")
        .to_string();
    let title = format!("{title} · {}", format_image_preview_size(bytes.len()));
    let image = Image::from_bytes(format, bytes);

    open_popup_window(
        PopupWindowOptions::new(title)
            .size(960.0, 720.0)
            .min_width(480.0)
            .min_height(360.0),
        move |_window, cx| cx.new(|_| RemoteImagePreview::new(image)),
        cx,
    );
}

fn notify_remote_image_preview_error(
    window_handle: AnyWindowHandle,
    message: String,
    cx: &mut AsyncApp,
) {
    let _ = cx.update_window(window_handle, |_, window, cx| {
        window.push_notification(
            Notification::error(format!("远程图片预览失败：{message}")).autohide(true),
            cx,
        );
    });
}

#[cfg(test)]
mod tests {
    use gpui::ImageFormat;

    #[test]
    fn detects_supported_image_extensions() {
        assert_eq!(
            Some(ImageFormat::Png),
            super::image_format_for_path("/root/octops.PNG")
        );
        assert_eq!(
            Some(ImageFormat::Jpeg),
            super::image_format_for_path("/srv/photo.jpeg")
        );
        assert_eq!(
            Some(ImageFormat::Webp),
            super::image_format_for_path("/srv/thumb.webp")
        );
    }

    #[test]
    fn rejects_non_image_paths() {
        assert_eq!(None, super::image_format_for_path("/root/readme.md"));
        assert_eq!(None, super::image_format_for_path("/root/archive.tar.gz"));
        assert_eq!(None, super::image_format_for_path("/root/no-extension"));
    }

    #[test]
    fn formats_preview_size_for_header() {
        assert_eq!("512 B", super::format_image_preview_size(512));
        assert_eq!("2.0 KB", super::format_image_preview_size(2048));
        assert_eq!("2.5 MB", super::format_image_preview_size(2_621_440));
    }

    #[test]
    fn reads_local_image_file_for_clipboard_upload() {
        let path = std::env::temp_dir().join(format!(
            "onetcli-remote-image-preview-{}.png",
            std::process::id()
        ));
        std::fs::write(&path, [1u8, 2, 3]).expect("write temp image");

        let image = super::image_from_local_path(&path).expect("image should load");

        assert_eq!(ImageFormat::Png, image.format);
        assert_eq!(vec![1, 2, 3], image.bytes);
        let _ = std::fs::remove_file(path);
    }
}
