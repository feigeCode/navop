//! 转录里的用户图片。
//!
//! 三处来源归一到这里：
//!
//! - 实时提交：手上就有 [`ImageAttachment`]（已经是 `Arc<Image>`，零解码）；
//! - 本地会话恢复：快照历史里的 [`InputImage`] 只有 base64，要解开；
//! - ACP 会话恢复：agent 把图片原样重放回来时是一条 `![](data:image/png;base64,…)`
//!   markdown（见 `acp/translate.rs`），得从文本里抠出来——否则整段 base64 会
//!   以纯文本形式铺在气泡里。
//!
//! 只做「解开成可渲染的图」，不做压缩：历史里的图是模型当时收到的原样，
//! 重新编码一遍纯属浪费，而且用户回看的就是他当时发出去的东西。

use std::sync::Arc;

use agent_runtime::InputImage;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use gpui::{Image, ImageFormat};

use crate::input::ImageAttachment;

/// 一条用户消息附带的图片。
#[derive(Clone, Debug)]
pub struct MessageImage {
    /// 展示名；恢复回来的历史没有文件名，留空。
    pub name: Option<String>,
    /// 底层图片（格式 + 原始字节），`img()` 直接可渲染。
    pub image: Arc<Image>,
}

/// 相等 = 同一个 `Arc` 指向同一份解码结果，外加同名。
///
/// 刻意不比较像素：`Image` 的相等语义是逐像素比对，在这里既贵又没意义——
/// 转录里两张「同一张图」（乐观插入一张、重放回来一张）本来就是同一个
/// `Arc`。这个实现只为让它能进带 `PartialEq` 的动作枚举。
impl PartialEq for MessageImage {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && Arc::ptr_eq(&self.image, &other.image)
    }
}

impl Eq for MessageImage {}

impl MessageImage {
    /// 由 composer 附件构造（实时提交路径）。
    pub(crate) fn from_attachment(attachment: &ImageAttachment) -> Self {
        Self {
            name: Some(attachment.name.clone()),
            image: attachment.image.clone(),
        }
    }

    /// 由 runtime 的多模态输入构造（本地快照恢复路径）。
    ///
    /// base64 损坏或 MIME 不认识时返回 `None`：一张画不出来的图不该让整条
    /// 历史恢复失败。
    pub(crate) fn from_input(input: &InputImage) -> Option<Self> {
        Self::from_base64(&input.mime, &input.data_base64)
    }

    fn from_base64(mime: &str, data_base64: &str) -> Option<Self> {
        let format = ImageFormat::from_mime_type(mime)?;
        let bytes = BASE64.decode(data_base64.trim()).ok()?;
        if bytes.is_empty() {
            return None;
        }
        Some(Self {
            name: None,
            image: Arc::new(Image::from_bytes(format, bytes)),
        })
    }
}

/// composer 附件 → 消息图片。
pub(crate) fn images_from_attachments(attachments: &[ImageAttachment]) -> Vec<MessageImage> {
    attachments
        .iter()
        .map(MessageImage::from_attachment)
        .collect()
}

/// runtime 图片输入 → 消息图片，逐张容错。
pub(crate) fn images_from_inputs(inputs: &[InputImage]) -> Vec<MessageImage> {
    inputs
        .iter()
        .filter_map(|input| {
            let image = MessageImage::from_input(input);
            if image.is_none() {
                tracing::warn!(mime = %input.mime, "跳过一张无法解码的历史图片");
            }
            image
        })
        .collect()
}

/// 把一段文本里的 `![alt](data:image/…;base64,…)` 抠成图片，返回剩余文本与图片。
///
/// 只认 data URL：外链图片仍留在文本里交给 markdown 渲染器，转录里不回抓网络。
/// 抠干净之后按行去重空行——否则留下的一串空行会把气泡撑高一截。
pub(crate) fn split_data_url_images(text: &str) -> (String, Vec<MessageImage>) {
    let mut images = Vec::new();
    let mut remaining = String::with_capacity(text.len());
    let mut cursor = 0;

    while let Some(start) = text[cursor..].find("![") {
        let start = cursor + start;
        let Some(end_offset) = text[start..].find(')') else {
            break;
        };
        let end = start + end_offset + 1;
        let candidate = &text[start..end];
        match data_url_image(candidate) {
            Some(image) => {
                remaining.push_str(&text[cursor..start]);
                images.push(image);
                cursor = end;
            }
            None => {
                // 不是 data URL（普通外链图 / 只是碰巧有 `![`）：原样留着，
                // 从它后面接着找。
                remaining.push_str(&text[cursor..end]);
                cursor = end;
            }
        }
    }
    remaining.push_str(&text[cursor..]);

    let remaining = collapse_blank_lines(&remaining);
    (remaining, images)
}

/// 从 `![alt](data:image/png;base64,XXXX)` 里解析出图片；不是 data URL 返回 `None`。
fn data_url_image(markdown: &str) -> Option<MessageImage> {
    let inside = markdown.split_once("](")?.1.strip_suffix(')')?.trim();
    let payload = inside.strip_prefix("data:")?;
    let (header, data) = payload.split_once(',')?;
    let mut header = header.split(';');
    let mime = header.next()?.trim();
    if !header.any(|part| part.trim().eq_ignore_ascii_case("base64")) {
        return None;
    }
    MessageImage::from_base64(mime, data)
}

/// 连续空行压成一个换行，并去掉首尾空行。
fn collapse_blank_lines(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() && out.last().is_none_or(|last| last.trim().is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|line| line.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_ONE_PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";

    /// 相等按「同一份解码结果」算：动作枚举靠它做 `PartialEq`，不能逐像素比。
    #[test]
    fn image_equality_follows_the_shared_decode() {
        let input = InputImage::new("image/png", PNG_ONE_PIXEL);
        let image = MessageImage::from_input(&input).expect("PNG should decode");
        let same = image.clone();
        let other = MessageImage::from_input(&input).expect("PNG should decode");
        let renamed = MessageImage {
            name: Some("screenshot.png".to_string()),
            ..image.clone()
        };

        assert_eq!(image, same, "克隆出来的是同一张图");
        assert_ne!(image, other, "各自解码出来的两份额不相等");
        assert_ne!(image, renamed, "换了文件名就不是同一条");
    }

    #[test]
    fn input_images_decode_into_renderable_ones() {
        let input = InputImage::new("image/png", PNG_ONE_PIXEL);
        let image = MessageImage::from_input(&input).expect("PNG should decode");
        assert!(image.name.is_none());
        assert!(!image.image.bytes.is_empty());
        assert_eq!(image.image.format, ImageFormat::Png);
    }

    #[test]
    fn broken_base64_or_unknown_mime_is_skipped_instead_of_failing() {
        assert!(MessageImage::from_input(&InputImage::new("image/png", "!!!")).is_none());
        assert!(MessageImage::from_input(&InputImage::new("image/png", "")).is_none());
        assert!(
            MessageImage::from_input(&InputImage::new("application/pdf", PNG_ONE_PIXEL)).is_none()
        );

        let inputs = vec![
            InputImage::new("application/pdf", PNG_ONE_PIXEL),
            InputImage::new("image/png", PNG_ONE_PIXEL),
        ];
        assert_eq!(images_from_inputs(&inputs).len(), 1);
    }

    #[test]
    fn data_url_markdown_becomes_an_image_without_leaking_base64_into_text() {
        let text =
            format!("看这张图\n\n![](data:image/png;base64,{PNG_ONE_PIXEL})\n\n另外注意这里",);
        let (remaining, images) = split_data_url_images(&text);
        assert_eq!(1, images.len());
        assert!(
            !remaining.contains("base64"),
            "base64 不该留在文本里: {remaining}"
        );
        assert_eq!("看这张图\n\n另外注意这里", remaining);
    }

    #[test]
    fn external_images_stay_in_the_text() {
        let text = "![示意图](https://example.com/a.png)";
        let (remaining, images) = split_data_url_images(text);
        assert!(images.is_empty());
        assert_eq!(
            text, remaining,
            "外链图交给 markdown 渲染器，转录不回抓网络"
        );
    }

    #[test]
    fn non_base64_data_url_is_left_alone() {
        let text = "![x](data:image/svg+xml,<svg/>)";
        let (remaining, images) = split_data_url_images(text);
        assert!(images.is_empty());
        assert_eq!(text, remaining);
    }

    #[test]
    fn text_without_images_is_returned_trimmed_of_blank_edges() {
        let (remaining, images) = split_data_url_images("\n\n只有文字\n\n");
        assert!(images.is_empty());
        assert_eq!("只有文字", remaining);
    }

    #[test]
    fn unterminated_image_syntax_does_not_panic() {
        let (remaining, images) = split_data_url_images("![未闭合(data:image/png;base64,AAAA");
        assert!(images.is_empty());
        assert_eq!("![未闭合(data:image/png;base64,AAAA", remaining);
    }
}
