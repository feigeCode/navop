use crate::SharedState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasToolMode {
    Select,
    Pan,
}

#[derive(Debug, Clone, Copy)]
struct CanvasToolModeState(CanvasToolMode);

impl CanvasToolMode {
    pub fn is_pan(self) -> bool {
        matches!(self, Self::Pan)
    }
}

pub fn current_tool_mode(shared_state: &SharedState) -> CanvasToolMode {
    shared_state
        .get::<CanvasToolModeState>()
        .map(|state| state.0)
        .unwrap_or(CanvasToolMode::Select)
}

pub fn set_tool_mode(shared_state: &mut SharedState, mode: CanvasToolMode) {
    shared_state.insert(CanvasToolModeState(mode));
}

#[cfg(test)]
mod tests {
    use super::{CanvasToolMode, current_tool_mode, set_tool_mode};
    use crate::SharedState;

    #[test]
    fn tool_mode_defaults_to_select() {
        let shared_state = SharedState::new();

        assert_eq!(CanvasToolMode::Select, current_tool_mode(&shared_state));
    }

    #[test]
    fn tool_mode_can_switch_to_pan() {
        let mut shared_state = SharedState::new();

        set_tool_mode(&mut shared_state, CanvasToolMode::Pan);

        assert_eq!(CanvasToolMode::Pan, current_tool_mode(&shared_state));
    }
}
