use std::ffi::c_void;
use std::path::PathBuf;

use cocoa::{
    appkit::{NSFilenamesPboardType, NSPasteboard, NSPasteboardTypeString},
    base::{NO, id, nil},
    foundation::{NSArray, NSAutoreleasePool, NSData, NSString},
};

/// Writes validated staging paths from a GPUI foreground callback.
///
/// Keep this on the UI thread: it calls AppKit's process-global pasteboard.
pub(super) fn write_files_to_system_clipboard(paths: &[PathBuf]) -> anyhow::Result<()> {
    unsafe {
        let pasteboard = NSPasteboard::generalPasteboard(nil);
        write_files_to_pasteboard(pasteboard, paths)
    }
}

unsafe fn write_files_to_pasteboard(pasteboard: id, paths: &[PathBuf]) -> anyhow::Result<()> {
    anyhow::ensure!(!paths.is_empty(), "clipboard file list is empty");

    let path_strings = paths
        .iter()
        .map(|path| {
            path.to_str()
                .ok_or_else(|| anyhow::anyhow!("clipboard file path is not valid UTF-8"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        let result = (|| {
            let ns_paths = path_strings
                .iter()
                .map(|path| NSString::alloc(nil).init_str(path).autorelease())
                .collect::<Vec<_>>();
            let filenames = NSArray::arrayWithObjects(nil, &ns_paths);
            let types =
                NSArray::arrayWithObjects(nil, &[NSFilenamesPboardType, NSPasteboardTypeString]);

            pasteboard.declareTypes_owner(types, nil);
            anyhow::ensure!(
                pasteboard.setPropertyList_forType(filenames, NSFilenamesPboardType) != NO,
                "macOS rejected the clipboard file list"
            );

            let joined_paths = path_strings.join("\n");
            let text = NSData::dataWithBytes_length_(
                nil,
                joined_paths.as_ptr() as *const c_void,
                joined_paths.len() as u64,
            );
            if pasteboard.setData_forType(text, NSPasteboardTypeString) == NO {
                tracing::debug!("macOS rejected the clipboard path text fallback");
            }

            Ok(())
        })();
        pool.drain();
        result
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::CStr;

    use super::*;

    #[test]
    fn native_file_clipboard_rejects_empty_file_lists() {
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            let pasteboard = NSPasteboard::pasteboardWithUniqueName(nil);

            assert!(write_files_to_pasteboard(pasteboard, &[]).is_err());

            pasteboard.releaseGlobally();
            pool.drain();
        }
    }

    #[test]
    fn native_file_clipboard_writes_finder_compatible_filenames() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let first = temp.path().join("报告 one.txt");
        let second = temp.path().join("data.csv");
        std::fs::write(&first, b"report").expect("write first clipboard file");
        std::fs::write(&second, b"data").expect("write second clipboard file");

        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            let pasteboard = NSPasteboard::pasteboardWithUniqueName(nil);

            write_files_to_pasteboard(pasteboard, &[first.clone(), second.clone()])
                .expect("write native file clipboard");

            let filenames = pasteboard.propertyListForType(NSFilenamesPboardType);
            assert_ne!(filenames, nil);
            assert_eq!(NSArray::count(filenames), 2);

            let first_path = NSArray::objectAtIndex(filenames, 0);
            let first_path = CStr::from_ptr(NSString::UTF8String(first_path))
                .to_string_lossy()
                .into_owned();
            assert_eq!(first.to_string_lossy(), first_path);

            let fallback = pasteboard.stringForType(NSPasteboardTypeString);
            assert_ne!(fallback, nil);
            let fallback = CStr::from_ptr(NSString::UTF8String(fallback))
                .to_string_lossy()
                .into_owned();
            assert_eq!(
                format!("{}\n{}", first.to_string_lossy(), second.to_string_lossy()),
                fallback
            );

            pasteboard.releaseGlobally();
            pool.drain();
        }
    }
}
