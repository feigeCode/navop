//! 图片附件:粘贴 / 选择的图片,既用于渲染缩略图,也用于发送给视觉模型。
//!
//! 持有 GPUI 的 [`gpui::Image`](Image)(含格式与原始字节),因此既能直接 `img()`
//! 渲染缩略图,又能按需编码为 base64 交给 [`agent_runtime::InputImage`] 发送给模型。

use std::path::Path;
use std::sync::Arc;

use agent_runtime::InputImage;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use gpui::{App, ClipboardEntry, Image, ImageFormat};
use uuid::Uuid;

/// 一张图片附件。
#[derive(Clone, Debug)]
pub struct ImageAttachment {
    /// 唯一标识(用于列表 key 与删除)。
    pub id: String,
    /// 展示名称(文件名或 "粘贴的图片")。
    pub name: String,
    /// 底层图片(格式 + 原始字节)。
    pub image: Arc<Image>,
}

impl ImageAttachment {
    fn new(name: impl Into<String>, image: Image) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            image: Arc::new(image),
        }
    }

    /// MIME 类型(如 `image/png`)。
    pub fn mime(&self) -> &'static str {
        self.image.format.mime_type()
    }

    /// 原始字节大小。
    pub fn byte_len(&self) -> usize {
        self.image.bytes.len()
    }

    /// 编码为 base64(发送给模型时调用)。
    pub fn data_base64(&self) -> String {
        BASE64.encode(&self.image.bytes)
    }

    /// 转换为 runtime 的多模态输入图片。
    pub fn to_input_image(&self) -> InputImage {
        InputImage::new(self.mime(), self.data_base64())
    }

    /// 从剪贴板读取全部图片附件(粘贴图片)。
    ///
    /// 同时处理 [`ClipboardEntry::Image`](直接图片) 与 [`ClipboardEntry::ExternalPaths`]
    /// (复制的图片文件)。无图片时返回空 `Vec`。
    pub fn from_clipboard(cx: &App) -> Vec<Self> {
        let Some(item) = cx.read_from_clipboard() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in item.into_entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    out.push(Self::new("粘贴的图片", image));
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    for path in paths.paths() {
                        if let Some(att) = Self::from_path(path) {
                            out.push(att);
                        }
                    }
                }
                ClipboardEntry::String(_) => {}
            }
        }
        out
    }

    /// 从文件路径读取图片附件;非图片或读取失败返回 `None`。
    pub fn from_path(path: &Path) -> Option<Self> {
        let format = format_from_path(path)?;
        let bytes = std::fs::read(path).ok()?;
        if bytes.is_empty() {
            return None;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("image")
            .to_string();
        Some(Self::new(name, Image::from_bytes(format, bytes)))
    }
}

/// 由扩展名推断 GPUI 图片格式。
fn format_from_path(path: &Path) -> Option<ImageFormat> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "ico" => "image/ico",
        "svg" => "image/svg+xml",
        _ => return None,
    };
    ImageFormat::from_mime_type(mime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_and_base64_roundtrip() {
        let att = ImageAttachment::new("t.png", Image::from_bytes(ImageFormat::Png, vec![1, 2, 3]));
        assert_eq!(att.mime(), "image/png");
        assert_eq!(att.byte_len(), 3);
        assert_eq!(att.data_base64(), BASE64.encode([1u8, 2, 3]));
        let input = att.to_input_image();
        assert_eq!(input.mime, "image/png");
    }

    #[test]
    fn unknown_extension_is_rejected() {
        assert!(format_from_path(Path::new("/tmp/file.txt")).is_none());
        assert!(format_from_path(Path::new("/tmp/pic.PNG")).is_some());
    }
}
