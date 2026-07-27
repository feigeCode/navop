use super::*;

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
