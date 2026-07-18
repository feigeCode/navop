use super::{SenderProgress, parse_ready_port, python_command, target_root};
use crate::ServerCopyItem;
use crate::direct_copy_scripts::{RECEIVER_SCRIPT, SENDER_SCRIPT};
use base64::Engine as _;
use std::fs;
use std::io::{BufRead as _, BufReader};
use std::process::Command;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

#[test]
fn parses_listener_port_only_from_ready_marker() {
    assert_eq!(Some(1234), parse_ready_port(b"noise\nNAVOP_READY 1234\n"));
    assert_eq!(None, parse_ready_port(b"NAVOP_TOTAL 10\n"));
}

#[test]
fn python_command_contains_no_unescaped_path_data() {
    let command = python_command("print('ok')", &["L3RtcC9hJ2I="]);
    assert!(command.starts_with("python3 -c '"));
    assert!(command.contains("L3RtcC9hJ2I="));
}

#[test]
fn sender_progress_handles_fragmented_lines() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let callback_observed = observed.clone();
    let callback = move |progress: crate::TransferProgress| {
        callback_observed
            .lock()
            .expect("progress lock")
            .push((progress.transferred, progress.total));
    };
    let mut state = SenderProgress {
        total: 0,
        transferred: 0,
        progress: &callback,
        buffer: Vec::new(),
    };
    state.consume(b"NAVOP_TOTAL 10\nNAVOP_PRO");
    state.consume(b"GRESS 4\n");
    assert_eq!(vec![(4, 10)], *observed.lock().expect("progress lock"));
}

#[test]
fn target_root_handles_root_directory_destinations() {
    let items = vec![
        ServerCopyItem {
            source_path: "/src/a".to_string(),
            target_path: "/a".to_string(),
            is_dir: false,
            size: 1,
        },
        ServerCopyItem {
            source_path: "/src/b".to_string(),
            target_path: "/b".to_string(),
            is_dir: false,
            size: 1,
        },
    ];
    assert_eq!("/", target_root(&items).expect("shared root"));
}

#[test]
fn embedded_python_scripts_are_valid() {
    for script in [SENDER_SCRIPT, RECEIVER_SCRIPT] {
        let status = Command::new("python3")
            .args([
                "-c",
                "import sys; compile(sys.argv[1], '<navop-direct-copy>', 'exec')",
                script,
            ])
            .status();
        let Ok(status) = status else {
            return;
        };
        assert!(status.success(), "embedded Python script must compile");
    }
}

#[test]
fn direct_protocol_copies_nested_directory_bytes() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let temp = tempfile::tempdir().expect("temporary directory");
    let source = temp.path().join("source dir's");
    let nested = source.join("nested");
    fs::create_dir_all(&nested).expect("source directory");
    fs::write(nested.join("payload.bin"), [0, 1, 2, 3, 255]).expect("source file");
    let target = temp.path().join("target");
    let token = "00112233445566778899aabbccddeeff";
    let target_arg =
        base64::engine::general_purpose::STANDARD.encode(target.to_string_lossy().as_bytes());

    let mut receiver = Command::new("python3")
        .args(["-c", RECEIVER_SCRIPT, &target_arg, token])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("receiver process");
    let stdout = receiver.stdout.take().expect("receiver stdout");
    let mut lines = BufReader::new(stdout).lines();
    let ready = lines.next().expect("ready line").expect("ready output");
    let port = ready
        .strip_prefix("NAVOP_READY ")
        .expect("ready marker")
        .parse::<u16>()
        .expect("listener port");

    let host = base64::engine::general_purpose::STANDARD.encode("127.0.0.1");
    let paths =
        serde_json::to_vec(&vec![source.to_string_lossy().to_string()]).expect("source path JSON");
    let paths = base64::engine::general_purpose::STANDARD.encode(paths);
    let sender = Command::new("python3")
        .args(["-c", SENDER_SCRIPT, &host, &port.to_string(), token, &paths])
        .status()
        .expect("sender process");
    assert!(sender.success(), "sender must complete successfully");
    let receiver = receiver.wait().expect("receiver completion");
    assert!(receiver.success(), "receiver must complete successfully");

    let copied = target
        .join(source.file_name().expect("source name"))
        .join("nested")
        .join("payload.bin");
    assert_eq!(
        vec![0, 1, 2, 3, 255],
        fs::read(copied).expect("copied bytes")
    );
}
