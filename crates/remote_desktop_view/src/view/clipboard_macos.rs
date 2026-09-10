use std::path::PathBuf;

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSData, NSURL, NSString};

/// Writes validated staging paths from a GPUI foreground callback.
///
/// Keep this on the UI thread: it calls AppKit's process-global pasteboard.
pub(super) fn write_files_to_system_clipboard(paths: &[PathBuf]) -> anyhow::Result<()> {
    let pasteboard = NSPasteboard::generalPasteboard();
    write_files_to_pasteboard(&pasteboard, paths)
}

fn write_files_to_pasteboard(pasteboard: &NSPasteboard, paths: &[PathBuf]) -> anyhow::Result<()> {
    anyhow::ensure!(!paths.is_empty(), "clipboard file list is empty");

    let path_strings = paths
        .iter()
        .map(|path| {
            path.to_str()
                .ok_or_else(|| anyhow::anyhow!("clipboard file path is not valid UTF-8"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    autoreleasepool(|_| {
        let string_type = unsafe { NSPasteboardTypeString };

        // 现代多文件复制入口:writeObjects 由 NSURL 自己编码
        // NSPasteboardTypeFileURL 等全部表示,Finder 与大多数宿主优先读取。
        let file_urls = path_strings
            .iter()
            .zip(paths)
            .map(|(path, original)| {
                NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), original.is_dir())
            })
            .collect::<Vec<_>>();
        let objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = file_urls
            .into_iter()
            .map(ProtocolObject::from_retained)
            .collect();
        pasteboard.writeObjects(&NSArray::from_retained_slice(&objects));

        // 纯文本回退:以换行分隔的路径列表,供只认文本的接收方使用。
        let joined_paths = path_strings.join("\n");
        let text = NSData::with_bytes(joined_paths.as_bytes());
        if !pasteboard.setData_forType(Some(&text), string_type) {
            tracing::debug!("macOS rejected the clipboard path text fallback");
        }

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use objc2::ClassType as _;

    use super::*;

    #[test]
    fn native_file_clipboard_rejects_empty_file_lists() {
        let pasteboard = NSPasteboard::pasteboardWithUniqueName();

        assert!(write_files_to_pasteboard(&pasteboard, &[]).is_err());
    }

    #[test]
    fn native_file_clipboard_writes_finder_compatible_file_urls() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let first = temp.path().join("报告 one.txt");
        let second = temp.path().join("data.csv");
        std::fs::write(&first, b"report").expect("write first clipboard file");
        std::fs::write(&second, b"data").expect("write second clipboard file");

        let pasteboard = NSPasteboard::pasteboardWithUniqueName();

        write_files_to_pasteboard(&pasteboard, &[first.clone(), second.clone()])
            .expect("write native file clipboard");

        // writeObjects 为每个 URL 建独立 pasteboard item,规范读法是
        // readObjectsForClasses: 直接取回 NSURL 对象。
        let classes = NSArray::from_slice(&[NSURL::class()]);
        let urls = unsafe { pasteboard.readObjectsForClasses_options(&classes, None) }
            .expect("file URL objects readable from pasteboard");
        let urls = unsafe { urls.cast_unchecked::<NSURL>() };
        assert_eq!(urls.len(), 2);

        let first_url = unsafe { urls.objectAtIndex_unchecked(0) };
        let first_path = first_url.path().expect("file URL path");
        assert_eq!(first.to_string_lossy(), first_path.to_string());

        let string_type = unsafe { NSPasteboardTypeString };
        let fallback = pasteboard
            .stringForType(string_type)
            .expect("string fallback on pasteboard");
        assert_eq!(
            format!("{}\n{}", first.to_string_lossy(), second.to_string_lossy()),
            fallback.to_string()
        );
    }
}
