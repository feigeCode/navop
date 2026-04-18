use anyhow::{Result, anyhow};

pub const LARGE_FILE_PLAIN_TEXT_THRESHOLD: usize = 2 * 1024 * 1024;
pub const MAX_EDITABLE_FILE_SIZE: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Code,
    PlainText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilePolicy {
    pub mode: EditorMode,
    pub is_large_file: bool,
}

pub fn determine_file_policy(file_size: usize) -> Result<FilePolicy> {
    if file_size > MAX_EDITABLE_FILE_SIZE {
        return Err(anyhow!(
            "File too large to edit: {} bytes exceeds limit {} bytes",
            file_size,
            MAX_EDITABLE_FILE_SIZE
        ));
    }

    Ok(FilePolicy {
        mode: if file_size > LARGE_FILE_PLAIN_TEXT_THRESHOLD {
            EditorMode::PlainText
        } else {
            EditorMode::Code
        },
        is_large_file: file_size > LARGE_FILE_PLAIN_TEXT_THRESHOLD,
    })
}

pub fn decode_text_content(bytes: &[u8]) -> Result<String> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    Ok(String::from_utf8(bytes.to_vec())?)
}

#[cfg(test)]
mod tests {
    use super::{EditorMode, MAX_EDITABLE_FILE_SIZE, decode_text_content, determine_file_policy};

    #[test]
    fn determine_file_policy_uses_code_mode_for_small_files() {
        let policy = determine_file_policy(128 * 1024).expect("小文件应允许编辑");
        assert_eq!(policy.mode, EditorMode::Code);
        assert!(!policy.is_large_file);
    }

    #[test]
    fn determine_file_policy_uses_plain_text_mode_for_large_files() {
        let policy = determine_file_policy(3 * 1024 * 1024).expect("大文件仍应允许编辑");
        assert_eq!(policy.mode, EditorMode::PlainText);
        assert!(policy.is_large_file);
    }

    #[test]
    fn determine_file_policy_rejects_files_over_max_limit() {
        let error = determine_file_policy(MAX_EDITABLE_FILE_SIZE + 1)
            .expect_err("超出 10 MiB 的文件应被拒绝");
        assert!(error.to_string().contains("too large"));
    }

    #[test]
    fn decode_text_content_strips_utf8_bom() {
        let decoded = decode_text_content(b"\xEF\xBB\xBFhello").expect("UTF-8 BOM 应被剥离");
        assert_eq!(decoded, "hello");
    }

    #[test]
    fn decode_text_content_rejects_invalid_utf8() {
        let error = decode_text_content(&[0xff, 0xfe]).expect_err("非法 UTF-8 应报错");
        assert!(error.to_string().contains("utf-8") || error.to_string().contains("UTF-8"));
    }
}
