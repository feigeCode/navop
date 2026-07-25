//! 图片附件:粘贴 / 选择的图片,既用于渲染缩略图,也用于发送给视觉模型。
//!
//! 持有 GPUI 的 [`gpui::Image`](Image)(含格式与原始字节),因此既能直接 `img()`
//! 渲染缩略图,又能按需编码为 base64 交给 [`agent_runtime::InputImage`] 发送给模型。

use std::path::Path;
use std::sync::Arc;

use agent_runtime::InputImage;
use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use gpui::{App, ClipboardEntry, Image, ImageFormat};
use image::imageops::FilterType;
use rust_i18n::t;
use uuid::Uuid;

/// 为完整模型请求预留文本、工具定义与 JSON 协议开销后的图片 base64 总预算。
///
/// OpenAI 兼容入口的请求体上限可能只有 16 MiB；控制在 10 MiB 可以避免图片编码
/// 加上其余消息后再次触发 `request body too large`。
pub(crate) const MODEL_IMAGES_BASE64_BUDGET: usize = 10 * 1024 * 1024;
/// 视觉模型输入图片的最长边。超出后先等比缩小，再转为 JPEG。
pub(crate) const MODEL_IMAGE_MAX_DIMENSION: u32 = 2048;
const MIN_REENCODE_DIMENSION: u32 = 512;
const JPEG_QUALITIES: [u8; 3] = [85, 70, 55];

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
                    out.push(Self::new(t!("AgentUi.pasted_image"), image));
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

/// 把一批 UI 图片附件转换为受请求体预算约束的模型输入。
///
/// 小图保留原始编码；大图或超大尺寸图片会等比缩放并转为 JPEG。预算按当前批次
/// 图片数均分，确保单次用户输入不会仅因 base64 膨胀就越过常见的 16 MiB 请求限制。
pub(crate) fn prepare_input_images(images: &[ImageAttachment]) -> Result<Vec<InputImage>> {
    if images.is_empty() {
        return Ok(Vec::new());
    }

    let raw_budget = base64_budget_to_raw_bytes(MODEL_IMAGES_BASE64_BUDGET) / images.len();
    let mut prepared = Vec::with_capacity(images.len());
    for image in images {
        prepared.push(
            prepare_input_image(image, raw_budget)
                .with_context(|| format!("无法处理图片附件“{}”", image.name))?,
        );
    }

    let encoded_bytes = prepared
        .iter()
        .map(|image| image.data_base64.len())
        .sum::<usize>();
    if encoded_bytes > MODEL_IMAGES_BASE64_BUDGET {
        bail!(
            "图片编码后仍有 {:.1} MiB，超过模型请求的安全预算 {:.1} MiB",
            encoded_bytes as f64 / 1024.0 / 1024.0,
            MODEL_IMAGES_BASE64_BUDGET as f64 / 1024.0 / 1024.0
        );
    }
    Ok(prepared)
}

fn prepare_input_image(attachment: &ImageAttachment, raw_budget: usize) -> Result<InputImage> {
    let decoded = match image::load_from_memory(&attachment.image.bytes) {
        Ok(decoded) => decoded,
        Err(_error) if attachment.byte_len() <= raw_budget => {
            // SVG 等 GPUI 可显示但 image crate 不负责解码的格式，只要体积安全就原样发送。
            return Ok(attachment.to_input_image());
        }
        Err(error) => {
            return Err(error).context("图片过大且无法解码压缩");
        }
    };

    let longest_edge = decoded.width().max(decoded.height());
    if attachment.byte_len() <= raw_budget && longest_edge <= MODEL_IMAGE_MAX_DIMENSION {
        return Ok(attachment.to_input_image());
    }

    let mut target_dimension = longest_edge.min(MODEL_IMAGE_MAX_DIMENSION);
    loop {
        let resized = if longest_edge > target_dimension {
            decoded.resize(target_dimension, target_dimension, FilterType::Lanczos3)
        } else {
            decoded.clone()
        };
        let rgb = flatten_onto_white(&resized);
        for quality in JPEG_QUALITIES {
            let mut bytes = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
                .encode_image(&image::DynamicImage::ImageRgb8(rgb.clone()))
                .context("JPEG 编码失败")?;
            if bytes.len() <= raw_budget {
                return Ok(InputImage::new("image/jpeg", BASE64.encode(bytes)));
            }
        }

        if target_dimension <= MIN_REENCODE_DIMENSION {
            break;
        }
        target_dimension = (target_dimension * 3 / 4).max(MIN_REENCODE_DIMENSION);
    }

    bail!(
        "压缩后仍超过单图安全预算 {:.1} MiB，请减少图片数量或裁剪图片",
        raw_budget as f64 / 1024.0 / 1024.0
    )
}

fn flatten_onto_white(image: &image::DynamicImage) -> image::RgbImage {
    let rgba = image.to_rgba8();
    image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let pixel = rgba.get_pixel(x, y);
        let alpha = u16::from(pixel[3]);
        image::Rgb([
            blend_over_white(pixel[0], alpha),
            blend_over_white(pixel[1], alpha),
            blend_over_white(pixel[2], alpha),
        ])
    })
}

fn blend_over_white(channel: u8, alpha: u16) -> u8 {
    ((u16::from(channel) * alpha + 255 * (255 - alpha)) / 255) as u8
}

fn base64_budget_to_raw_bytes(base64_bytes: usize) -> usize {
    base64_bytes / 4 * 3
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
    use image::{DynamicImage, ImageBuffer, Rgb};
    use std::io::Cursor;

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

    #[test]
    fn oversized_bitmap_is_resized_and_reencoded_for_model() {
        let pixels = ImageBuffer::from_fn(2400, 2400, |x, y| {
            Rgb([
                (x.wrapping_mul(31) ^ y) as u8,
                (y.wrapping_mul(17) ^ x) as u8,
                (x.wrapping_add(y).wrapping_mul(13)) as u8,
            ])
        });
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(pixels)
            .write_to(&mut encoded, image::ImageFormat::Bmp)
            .expect("test bitmap should encode");
        let attachment = ImageAttachment::new(
            "large.bmp",
            Image::from_bytes(ImageFormat::Bmp, encoded.into_inner()),
        );
        assert!(
            attachment.byte_len() > 16 * 1024 * 1024,
            "fixture must reproduce an oversized request attachment"
        );

        let inputs = prepare_input_images(&[attachment]).expect("image should be prepared");
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].mime, "image/jpeg");
        assert!(
            inputs[0].data_base64.len() <= MODEL_IMAGES_BASE64_BUDGET,
            "prepared image must leave room below the provider's 16 MB request limit"
        );

        let prepared_bytes = BASE64
            .decode(inputs[0].data_base64.as_bytes())
            .expect("prepared image should contain valid base64");
        let prepared =
            image::load_from_memory(&prepared_bytes).expect("prepared image should still decode");
        assert!(
            prepared.width() <= MODEL_IMAGE_MAX_DIMENSION
                && prepared.height() <= MODEL_IMAGE_MAX_DIMENSION,
            "prepared image should be bounded: {}x{}",
            prepared.width(),
            prepared.height()
        );
    }
}
