//! Message Content Block Definition
//!
//! Supports multi-modal content including text, images, etc.

use serde::{Deserialize, Serialize};

/// Message Content Block
///
/// A message can contain multiple content blocks, supporting multi-modal content like text, images, etc.
///
/// # Example
///
/// ```rust
/// use llm_connector::types::MessageBlock;
///
/// // Text block
/// let text = MessageBlock::text("Hello, world!");
///
/// // Image block (Base64)
/// let image = MessageBlock::image_base64("image/jpeg", "base64_data...");
///
/// // Image block (URL)
/// let image_url = MessageBlock::image_url("https://example.com/image.jpg");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageBlock {
    /// Text block
    Text { text: String },

    /// Extended-thinking block (Anthropic native shape).
    ///
    /// `signature` is an Anthropic-issued continuation credential when present.
    /// `None` means unsigned or degraded content (for example OpenAI-style reasoning);
    /// adapters must not invent placeholder signatures.
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },

    /// Image block (Anthropic format)
    Image { source: ImageSource },

    /// Image URL block (OpenAI format)
    ImageUrl { image_url: ImageUrl },

    /// Document block (Base64 PDF, etc. - Anthropic format / extending others)
    Document { source: DocumentSource },

    /// Document URL block
    DocumentUrl { document_url: DocumentUrl },
}

impl MessageBlock {
    /// Create text block
    ///
    /// # Example
    ///
    /// ```rust
    /// use llm_connector::types::MessageBlock;
    ///
    /// let block = MessageBlock::text("Hello, world!");
    /// ```
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Create an Anthropic-style thinking block.
    pub fn thinking_unsigned(thinking: impl Into<String>) -> Self {
        Self::Thinking {
            thinking: thinking.into(),
            signature: None,
        }
    }

    /// Create an Anthropic-style thinking block with a real `signature`.
    pub fn thinking_signed(thinking: impl Into<String>, signature: impl Into<String>) -> Self {
        Self::Thinking {
            thinking: thinking.into(),
            signature: Some(signature.into()),
        }
    }

    /// Create Base64 Image block (Anthropic format)
    ///
    /// # Parameters
    ///
    /// - `media_type`: Media type, such as "image/jpeg", "image/png"
    /// - `data`: Base64 encoded image data
    ///
    /// # Example
    ///
    /// ```rust
    /// use llm_connector::types::MessageBlock;
    ///
    /// let block = MessageBlock::image_base64(
    ///     "image/jpeg",
    ///     "iVBORw0KGgoAAAANSUhEUgA..."
    /// );
    /// ```
    pub fn image_base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self::Image {
            source: ImageSource::Base64 {
                media_type: media_type.into(),
                data: data.into(),
            },
        }
    }

    /// Create document block (Base64)
    pub fn document_base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self::Document {
            source: DocumentSource::Base64 {
                media_type: media_type.into(),
                data: data.into(),
            },
        }
    }

    /// Create image URL block (Anthropic format)
    ///
    /// # Example
    ///
    /// ```rust
    /// use llm_connector::types::MessageBlock;
    ///
    /// let block = MessageBlock::image_url_anthropic("https://example.com/image.jpg");
    /// ```
    pub fn image_url_anthropic(url: impl Into<String>) -> Self {
        Self::Image {
            source: ImageSource::Url { url: url.into() },
        }
    }

    /// Create image URL block (OpenAI format)
    ///
    /// # Example
    ///
    /// ```rust
    /// use llm_connector::types::MessageBlock;
    ///
    /// let block = MessageBlock::image_url("https://example.com/image.jpg");
    /// ```
    pub fn image_url(url: impl Into<String>) -> Self {
        Self::ImageUrl {
            image_url: ImageUrl {
                url: url.into(),
                detail: None,
            },
        }
    }

    /// Create image URL block (OpenAI format, with detail parameter)
    ///
    /// # Parameters
    ///
    /// - `url`: Image URL
    /// - `detail`: Image detail level, optional values: "auto", "low", "high"
    ///
    /// # Example
    ///
    /// ```rust
    /// use llm_connector::types::MessageBlock;
    ///
    /// let block = MessageBlock::image_url_with_detail(
    ///     "https://example.com/image.jpg",
    ///     "high"
    /// );
    /// ```
    pub fn image_url_with_detail(url: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::ImageUrl {
            image_url: ImageUrl {
                url: url.into(),
                detail: Some(detail.into()),
            },
        }
    }

    /// Create message block from file path (Image or PDF)
    pub fn from_file_path(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();

        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let b64 = STANDARD.encode(bytes);

        if mime.starts_with("image/") {
            Ok(Self::image_base64(mime, b64))
        } else {
            Ok(Self::document_base64(mime, b64))
        }
    }

    /// Get text content (if is text block)
    ///
    /// # Example
    ///
    /// ```rust
    /// use llm_connector::types::MessageBlock;
    ///
    /// let block = MessageBlock::text("Hello");
    /// assert_eq!(block.as_text(), Some("Hello"));
    ///
    /// let image = MessageBlock::image_url("https://...");
    /// assert_eq!(image.as_text(), None);
    /// ```
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }

    /// Thinking body when this block is [`MessageBlock::Thinking`].
    pub fn as_thinking_body(&self) -> Option<&str> {
        match self {
            Self::Thinking { thinking, .. } => Some(thinking),
            _ => None,
        }
    }

    /// Anthropic signature when this block is signed thinking.
    pub fn thinking_signature(&self) -> Option<&str> {
        match self {
            Self::Thinking { signature, .. } => signature.as_deref(),
            _ => None,
        }
    }

    /// Extract Base64 encoded image data from the block (if it contains an image)
    ///
    /// Supports both `Image` (Anthropic format) and `ImageUrl` blocks.
    /// For URL sources, extracts the Base64 portion from `data:` URIs.
    ///
    /// # Returns
    ///
    /// - `Some(String)` containing the Base64 data if the block contains an image
    /// - `None` if the block is not an image type
    ///
    /// # Example
    ///
    /// ```rust
    /// use llm_connector::types::MessageBlock;
    ///
    /// let image = MessageBlock::image_base64("image/png", "abc123");
    /// assert_eq!(image.as_image_base64(), Some("abc123".to_string()));
    ///
    /// let text = MessageBlock::text("Hello");
    /// assert_eq!(text.as_image_base64(), None);
    /// ```
    pub fn as_image_base64(&self) -> Option<String> {
        match self {
            Self::Image { source } => match source {
                ImageSource::Base64 { data, .. } => Some(data.to_owned()),
                ImageSource::Url { url } => Some(
                    if url.starts_with("data:")
                        && let Some(pos) = url.find(";base64,")
                    {
                        &url[pos + 8..]
                    } else {
                        url
                    }
                    .to_owned(),
                ),
            },
            MessageBlock::ImageUrl {
                image_url: ImageUrl { url, .. },
            } => Some(
                if url.starts_with("data:")
                    && let Some(pos) = url.find(";base64,")
                {
                    &url[pos + 8..]
                } else {
                    url
                }
                .to_owned(),
            ),
            _ => None,
        }
    }

    /// CheckisifasText block
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. })
    }

    pub fn is_thinking(&self) -> bool {
        matches!(self, Self::Thinking { .. })
    }

    /// Check if is image block
    pub fn is_image(&self) -> bool {
        matches!(self, Self::Image { .. } | Self::ImageUrl { .. })
    }

    /// Check if is document block
    pub fn is_document(&self) -> bool {
        matches!(self, Self::Document { .. } | Self::DocumentUrl { .. })
    }

    /// Create Document URL block
    pub fn document_url(url: impl Into<String>) -> Self {
        Self::DocumentUrl {
            document_url: DocumentUrl { url: url.into() },
        }
    }
}

/// Image source (Anthropic format)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    /// Base64 encoded image
    Base64 {
        /// Media type, such as "image/jpeg", "image/png"
        media_type: String,
        /// Base64 encoded image data
        data: String,
    },

    /// Image URL
    Url {
        /// Image URL
        url: String,
    },
}

/// Image URL (OpenAI format)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageUrl {
    /// Image URL
    pub url: String,

    /// Image detail level
    ///
    /// Optional values: "auto", "low", "high"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Document source
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocumentSource {
    /// Base64 encoded document
    Base64 {
        /// Media type, such as "application/pdf"
        media_type: String,
        /// Base64 encoded document data
        data: String,
    },
}

/// Document URL
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentUrl {
    /// Document URL
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_block() {
        let block = MessageBlock::text("Hello");
        assert!(block.is_text());
        assert!(!block.is_image());
        assert_eq!(block.as_text(), Some("Hello"));
    }

    #[test]
    fn test_image_base64_block() {
        let block = MessageBlock::image_base64("image/jpeg", "base64data");
        assert!(!block.is_text());
        assert!(block.is_image());
        assert_eq!(block.as_text(), None);

        match block {
            MessageBlock::Image { source } => match source {
                ImageSource::Base64 { media_type, data } => {
                    assert_eq!(media_type, "image/jpeg");
                    assert_eq!(data, "base64data");
                }
                _ => panic!("Expected Base64 source"),
            },
            _ => panic!("Expected Image block"),
        }
    }

    #[test]
    fn test_image_url_block() {
        let block = MessageBlock::image_url("https://example.com/image.jpg");
        assert!(!block.is_text());
        assert!(block.is_image());

        match block {
            MessageBlock::ImageUrl { image_url } => {
                assert_eq!(image_url.url, "https://example.com/image.jpg");
                assert_eq!(image_url.detail, None);
            }
            _ => panic!("Expected ImageUrl block"),
        }
    }

    #[test]
    fn test_image_url_with_detail() {
        let block = MessageBlock::image_url_with_detail("https://example.com/image.jpg", "high");

        match block {
            MessageBlock::ImageUrl { image_url } => {
                assert_eq!(image_url.url, "https://example.com/image.jpg");
                assert_eq!(image_url.detail, Some("high".to_string()));
            }
            _ => panic!("Expected ImageUrl block"),
        }
    }

    #[test]
    fn test_serialize_text_block() {
        let block = MessageBlock::text("Hello");
        let json = serde_json::to_string(&block).unwrap();
        assert_eq!(json, r#"{"type":"text","text":"Hello"}"#);
    }

    #[test]
    fn test_serialize_image_base64() {
        let block = MessageBlock::image_base64("image/jpeg", "data");
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains(r#""type":"image""#));
        assert!(json.contains(r#""type":"base64""#));
        assert!(json.contains(r#""media_type":"image/jpeg""#));
    }

    #[test]
    fn test_serialize_image_url() {
        let block = MessageBlock::image_url("https://example.com/image.jpg");
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains(r#""type":"image_url""#));
        assert!(json.contains(r#""url":"https://example.com/image.jpg""#));
    }

    #[test]
    fn test_thinking_block_serde_unsigned() {
        let block = MessageBlock::thinking_unsigned("chain");
        assert!(block.is_thinking());
        assert_eq!(block.thinking_signature(), None);
        let json = serde_json::to_string(&block).unwrap();
        assert_eq!(json, r#"{"type":"thinking","thinking":"chain"}"#);
    }

    #[test]
    fn test_thinking_block_serde_signed() {
        let block = MessageBlock::thinking_signed("chain", "sig_real");
        assert_eq!(block.thinking_signature(), Some("sig_real"));
        let json = serde_json::to_string(&block).unwrap();
        assert!(
            json.contains(r#""type":"thinking""#)
                && json.contains(r#""thinking":"chain""#)
                && json.contains(r#""signature":"sig_real""#)
        );
        let back: MessageBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(back, block);
    }

    #[test]
    fn test_deserialize_text_block() {
        let json = r#"{"type":"text","text":"Hello"}"#;
        let block: MessageBlock = serde_json::from_str(json).unwrap();
        assert_eq!(block, MessageBlock::text("Hello"));
    }
}
