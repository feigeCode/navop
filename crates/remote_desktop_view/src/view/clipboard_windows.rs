use std::path::{Path, PathBuf};

// DROPFILES contains pFiles, POINT, fNC, and fWide. Windows supports only
// little-endian targets, so the native structure can be emitted explicitly.
const DROPFILES_HEADER_SIZE: usize = 20;
const DROPFILES_WIDE_OFFSET: usize = 16;
const WIDE_PATH_FLAG: u32 = 1;

fn drop_files_payload(paths: &[PathBuf]) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(!paths.is_empty(), "clipboard file list is empty");

    let mut payload = Vec::new();
    payload.extend_from_slice(&(DROPFILES_HEADER_SIZE as u32).to_le_bytes());
    payload.resize(DROPFILES_HEADER_SIZE, 0);
    payload[DROPFILES_WIDE_OFFSET..DROPFILES_HEADER_SIZE]
        .copy_from_slice(&WIDE_PATH_FLAG.to_le_bytes());

    for path in paths {
        append_wide_path(&mut payload, path)?;
    }
    payload.extend_from_slice(&0u16.to_le_bytes());
    Ok(payload)
}

fn append_wide_path(payload: &mut Vec<u8>, path: &Path) -> anyhow::Result<()> {
    let path = path.to_string_lossy();
    anyhow::ensure!(
        !path.encode_utf16().any(|unit| unit == 0),
        "clipboard path contains a null character"
    );
    for unit in path.encode_utf16().chain(Some(0)) {
        payload.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn write_files_to_system_clipboard(paths: &[PathBuf]) -> anyhow::Result<()> {
    use windows::Win32::System::DataExchange::EmptyClipboard;

    let payload = drop_files_payload(paths)?;
    let _clipboard = ClipboardGuard::open()?;
    unsafe { EmptyClipboard()? };
    set_clipboard_bytes(&payload)
}

#[cfg(target_os = "windows")]
fn set_clipboard_bytes(payload: &[u8]) -> anyhow::Result<()> {
    use windows::Win32::{
        Foundation::HANDLE,
        System::{
            DataExchange::SetClipboardData,
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::CF_HDROP,
        },
    };
    use windows::core::Owned;

    unsafe {
        let global = Owned::new(GlobalAlloc(GMEM_MOVEABLE, payload.len())?);
        let pointer = GlobalLock(*global);
        anyhow::ensure!(!pointer.is_null(), "GlobalLock returned null");
        std::ptr::copy_nonoverlapping(payload.as_ptr(), pointer.cast(), payload.len());
        GlobalUnlock(*global).ok();
        SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(global.0)))?;
        std::mem::forget(global);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
struct ClipboardGuard;

#[cfg(target_os = "windows")]
impl ClipboardGuard {
    fn open() -> anyhow::Result<Self> {
        use windows::Win32::System::DataExchange::OpenClipboard;

        unsafe { OpenClipboard(None)? };
        Ok(Self)
    }
}

#[cfg(target_os = "windows")]
impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        use windows::Win32::System::DataExchange::CloseClipboard;

        if let Err(error) = unsafe { CloseClipboard() } {
            tracing::warn!(error = %error, "failed to close Windows clipboard");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{DROPFILES_HEADER_SIZE, DROPFILES_WIDE_OFFSET, WIDE_PATH_FLAG, drop_files_payload};

    #[test]
    fn drop_files_payload_contains_wide_double_null_terminated_paths() {
        let paths = vec![
            PathBuf::from(r"C:\Users\测试\report.txt"),
            PathBuf::from(r"D:\data\image.png"),
        ];

        let payload = drop_files_payload(&paths).expect("build CF_HDROP payload");
        assert_eq!(
            DROPFILES_HEADER_SIZE as u32,
            u32::from_le_bytes(payload[0..4].try_into().unwrap())
        );
        assert_eq!(
            WIDE_PATH_FLAG,
            u32::from_le_bytes(
                payload[DROPFILES_WIDE_OFFSET..DROPFILES_HEADER_SIZE]
                    .try_into()
                    .unwrap()
            )
        );

        let wide = payload[DROPFILES_HEADER_SIZE..]
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        let expected = paths
            .iter()
            .flat_map(|path| {
                let mut encoded = path.to_string_lossy().encode_utf16().collect::<Vec<_>>();
                encoded.push(0);
                encoded
            })
            .chain(Some(0))
            .collect::<Vec<_>>();
        assert_eq!(expected, wide);
        assert!(wide.ends_with(&[0, 0]));
    }

    #[test]
    fn drop_files_payload_rejects_empty_file_lists() {
        assert!(drop_files_payload(&[]).is_err());
    }
}
