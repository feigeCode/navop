//! 从二维码图片里识别一次性验证码（TOTP）密钥。
//!
//! 用户可以把认证器里的二维码截图/图片直接丢给表单，省去手抄 base32 密钥。

use otp::{OtpError, OtpSecret};

/// 二维码识别的失败原因。
#[derive(Debug)]
pub enum OtpQrError {
    /// 图片本身无法解码（不是图片，或格式不受支持）。
    ImageDecode,
    /// 图片里没有二维码。
    NoQrCodeFound,
    /// 二维码内容不是 TOTP 密钥（例如普通网址、HOTP 配置）。
    NotOtpSecret(OtpError),
}

/// 从图片字节（PNG/JPEG 等常见格式）中识别 TOTP 密钥。
pub fn decode_otp_secret(bytes: &[u8]) -> Result<OtpSecret, OtpQrError> {
    let image = image::load_from_memory(bytes).map_err(|_| OtpQrError::ImageDecode)?;
    decode_otp_secret_from_image(&image)
}

/// 从已解码的图片中识别 TOTP 密钥。
pub fn decode_otp_secret_from_image(image: &image::DynamicImage) -> Result<OtpSecret, OtpQrError> {
    let gray = image.to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(gray);
    let grids = prepared.detect_grids();
    if grids.is_empty() {
        return Err(OtpQrError::NoQrCodeFound);
    }

    let mut content_error = None;
    for grid in grids {
        let Ok((_metadata, content)) = grid.decode() else {
            continue;
        };
        match OtpSecret::parse(content.trim()) {
            Ok(secret) => return Ok(secret),
            Err(error) => content_error = Some(error),
        }
    }

    Err(match content_error {
        Some(error) => OtpQrError::NotOtpSecret(error),
        None => OtpQrError::NoQrCodeFound,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qr_png(content: &str) -> Vec<u8> {
        let code = qrcode::QrCode::new(content.as_bytes()).expect("valid qr payload");
        let image = code
            .render::<image::Luma<u8>>()
            .quiet_zone(true)
            .module_dimensions(8, 8)
            .build();
        let mut bytes = Vec::new();
        image::DynamicImage::ImageLuma8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("png encode");
        bytes
    }

    #[test]
    fn decodes_otpauth_uri_from_qr_image() {
        let secret = decode_otp_secret(&qr_png(
            "otpauth://totp/ACME:alice?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=ACME&digits=6&period=30&algorithm=SHA1",
        ))
        .expect("should decode");
        assert_eq!(secret.secret, "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(secret.issuer.as_deref(), Some("ACME"));
        assert_eq!(secret.account.as_deref(), Some("alice"));
    }

    #[test]
    fn decodes_bare_base32_from_qr_image() {
        let secret = decode_otp_secret(&qr_png("GEZDGNBVGY3TQOJQ")).expect("should decode");
        assert_eq!(secret.secret, "GEZDGNBVGY3TQOJQ");
    }

    #[test]
    fn reports_missing_qr_code() {
        let white = image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(
            64,
            64,
            image::Luma([255u8]),
        ));
        assert!(matches!(
            decode_otp_secret_from_image(&white),
            Err(OtpQrError::NoQrCodeFound)
        ));
    }

    #[test]
    fn reports_non_otp_qr_code() {
        let error = decode_otp_secret(&qr_png("https://example.com/login")).expect_err("not otp");
        assert!(matches!(
            error,
            OtpQrError::NotOtpSecret(OtpError::UnsupportedScheme(_))
        ));
    }

    #[test]
    fn reports_undecodable_image() {
        assert!(matches!(
            decode_otp_secret(b"not an image"),
            Err(OtpQrError::ImageDecode)
        ));
    }
}
