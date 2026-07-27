use std::path::PathBuf;

use gpui::{ClipboardEntry, ClipboardItem, ExternalPaths};
use remote_desktop::RemoteDesktopProtocol;

use super::{
    FIRST_LOCAL_CLIPBOARD_TRANSFER_ID, LocalClipboardContent, REMOTE_CLIPBOARD_TRANSFER_BIT,
    allocate_local_clipboard_transfer_id, classify_local_clipboard, clipboard_files_supported,
    clipboard_text_supported, is_remote_clipboard_transfer_id,
    validate_remote_clipboard_paths_in_root,
};

#[test]
fn clipboard_protocol_policy_keeps_vnc_to_ascii_text_only() {
    assert!(clipboard_text_supported(
        RemoteDesktopProtocol::Rdp,
        "中文 clipboard"
    ));
    assert!(clipboard_text_supported(
        RemoteDesktopProtocol::Vnc,
        "ASCII clipboard"
    ));
    assert!(!clipboard_text_supported(
        RemoteDesktopProtocol::Vnc,
        "中文 clipboard"
    ));
    assert!(clipboard_files_supported(RemoteDesktopProtocol::Rdp));
    assert!(!clipboard_files_supported(RemoteDesktopProtocol::Vnc));
}

#[test]
fn local_clipboard_classification_prioritizes_files_over_path_text_fallback() {
    let files = ExternalPaths(
        [
            PathBuf::from("/tmp/report.txt"),
            PathBuf::from("/tmp/data.csv"),
        ]
        .into_iter()
        .collect(),
    );
    let item = ClipboardItem {
        entries: vec![
            ClipboardEntry::String(gpui::ClipboardString::new(
                "platform path fallback".to_string(),
            )),
            ClipboardEntry::ExternalPaths(files),
        ],
    };

    assert_eq!(
        LocalClipboardContent::Files(vec![
            "/tmp/report.txt".to_string(),
            "/tmp/data.csv".to_string(),
        ]),
        classify_local_clipboard(&item)
    );
    assert_eq!(
        LocalClipboardContent::Text("ASCII clipboard".to_string()),
        classify_local_clipboard(&ClipboardItem::new_string("ASCII clipboard".to_string()))
    );
    assert_eq!(
        LocalClipboardContent::Other,
        classify_local_clipboard(&ClipboardItem { entries: vec![] })
    );
}

#[test]
fn local_clipboard_transfer_ids_never_enter_the_remote_namespace() {
    let mut next_id = FIRST_LOCAL_CLIPBOARD_TRANSFER_ID;

    assert_eq!(1, allocate_local_clipboard_transfer_id(&mut next_id));
    assert_eq!(2, allocate_local_clipboard_transfer_id(&mut next_id));

    next_id = REMOTE_CLIPBOARD_TRANSFER_BIT - 1;
    assert_eq!(
        REMOTE_CLIPBOARD_TRANSFER_BIT - 1,
        allocate_local_clipboard_transfer_id(&mut next_id)
    );
    assert_eq!(1, allocate_local_clipboard_transfer_id(&mut next_id));
    assert!(!is_remote_clipboard_transfer_id(1));
    assert!(is_remote_clipboard_transfer_id(
        REMOTE_CLIPBOARD_TRANSFER_BIT | 1
    ));
}

#[test]
fn remote_clipboard_paths_must_resolve_inside_the_staging_root() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let root = temp.path().join("navop-rdp-clipboard");
    let transfer = root.join("transfer-remote");
    std::fs::create_dir_all(&transfer).expect("create transfer directory");
    let received = transfer.join("report.txt");
    std::fs::write(&received, b"report").expect("write received file");
    let received_string = received.to_string_lossy().into_owned();

    assert_eq!(
        vec![std::fs::canonicalize(&received).unwrap()],
        validate_remote_clipboard_paths_in_root(&root, &[received_string]).unwrap()
    );
    assert!(
        validate_remote_clipboard_paths_in_root(
            &root,
            &[temp
                .path()
                .join("outside.txt")
                .to_string_lossy()
                .into_owned()]
        )
        .is_err()
    );
    assert!(validate_remote_clipboard_paths_in_root(&root, &[]).is_err());
    assert!(validate_remote_clipboard_paths_in_root(&root, &["relative.txt".to_string()]).is_err());
}

#[cfg(unix)]
#[test]
fn remote_clipboard_paths_reject_symlink_escape_from_staging_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("temporary directory");
    let root = temp.path().join("navop-rdp-clipboard");
    let transfer = root.join("transfer-remote");
    std::fs::create_dir_all(&transfer).expect("create transfer directory");
    let outside = temp.path().join("outside.txt");
    std::fs::write(&outside, b"outside").expect("write outside file");
    let escaped = transfer.join("escaped.txt");
    symlink(&outside, &escaped).expect("create symlink");

    assert!(
        validate_remote_clipboard_paths_in_root(&root, &[escaped.to_string_lossy().into_owned()])
            .is_err()
    );
}
