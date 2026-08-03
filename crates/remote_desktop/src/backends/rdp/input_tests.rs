use super::*;

#[cfg(unix)]
#[test]
fn close_helper_kills_a_process_that_ignores_the_close_request() {
    let mut helper = std::process::Command::new("sleep")
        .arg("60")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("spawn test helper");
    let mut stdin = helper.stdin.take().expect("helper stdin");
    let (output_tx, _output_rx) = crate::output_mailbox::output_mailbox();

    close_helper_with_grace_period(
        &mut helper,
        &mut stdin,
        &output_tx,
        RemoteDesktopProtocol::Rdp,
        std::time::Duration::from_millis(20),
    );

    assert!(
        helper.try_wait().expect("query helper status").is_some(),
        "close must reap a helper that ignores the request"
    );
}

#[test]
fn coalesces_consecutive_mouse_moves_without_reordering_actions() {
    let inputs = vec![
        RemoteDesktopInput::MouseMove { x: 10, y: 10 },
        RemoteDesktopInput::MouseMove { x: 20, y: 20 },
        RemoteDesktopInput::MouseButton {
            button: crate::RemoteMouseButton::Left,
            pressed: true,
        },
        RemoteDesktopInput::MouseMove { x: 30, y: 30 },
        RemoteDesktopInput::MouseMove { x: 40, y: 40 },
        RemoteDesktopInput::Key {
            key: crate::RemoteKey::Named(crate::RemoteNamedKey::Enter),
            pressed: true,
        },
    ];

    assert_eq!(
        vec![
            RemoteDesktopInput::MouseMove { x: 20, y: 20 },
            RemoteDesktopInput::MouseButton {
                button: crate::RemoteMouseButton::Left,
                pressed: true,
            },
            RemoteDesktopInput::MouseMove { x: 40, y: 40 },
            RemoteDesktopInput::Key {
                key: crate::RemoteKey::Named(crate::RemoteNamedKey::Enter),
                pressed: true,
            },
        ],
        coalesce_remote_inputs(inputs)
    );
}
