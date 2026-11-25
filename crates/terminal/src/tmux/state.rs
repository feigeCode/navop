use super::TmuxVersion;

pub const LIST_WINDOWS_FORMAT: &str =
    "#{q:session_id}|#{q:window_id}|#{q:window_index}|#{q:window_name}|#{window_active}";
pub const LIST_PANES_FORMAT: &str = "#{q:session_id}|#{q:window_id}|#{q:window_index}|#{q:pane_id}|#{pane_width}|#{pane_height}|#{window_active}|#{pane_active}|#{q:pane_current_path}|#{q:pane_current_command}|#{pane_in_mode}|#{q:pane_title}|#{pane_pid}";
pub const LIST_PANES_COMMAND_FORMAT: &str = "#{q:session_id}|#{q:window_id}|#{q:window_index}|#{q:pane_id}|#{pane_width}|#{pane_height}|#{window_active}|#{pane_active}|#{q:pane_current_path}|#{q:pane_current_command}|#{pane_in_mode}|#{q:pane_title}|#{pane_pid}|#{pane_command_running}|#{pane_command_status}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TmuxWindow {
    pub id: String,
    pub index: String,
    pub name: String,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TmuxPane {
    pub id: String,
    pub window_id: String,
    pub width: u16,
    pub height: u16,
    pub window_active: bool,
    pub active_in_window: bool,
    pub cwd: Option<String>,
    pub command: String,
    pub command_running: Option<bool>,
    pub command_status: Option<i32>,
    pub in_mode: bool,
    pub title: String,
    pub pid: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TmuxSessionState {
    pub session_id: String,
    pub session_name: String,
    pub version: Option<TmuxVersion>,
    pub windows: Vec<TmuxWindow>,
    pub panes: Vec<TmuxPane>,
    pub attached: bool,
}

impl TmuxSessionState {
    pub fn active_window(&self) -> Option<&TmuxWindow> {
        self.windows.iter().find(|window| window.active)
    }

    pub fn active_pane(&self) -> Option<&TmuxPane> {
        let window = self.active_window()?;
        self.panes
            .iter()
            .find(|pane| pane.window_id == window.id && pane.active_in_window)
    }

    pub fn active_cwd(&self) -> Option<&str> {
        self.active_pane()?.cwd.as_deref()
    }

    pub fn pane(&self, pane_id: &str) -> Option<&TmuxPane> {
        self.panes.iter().find(|pane| pane.id == pane_id)
    }
}

pub fn parse_list_windows(output: &str) -> Vec<TmuxWindow> {
    output
        .lines()
        .filter_map(|line| {
            let fields = parse_fields(line, 5)?;
            Some(TmuxWindow {
                id: fields[1].clone(),
                index: fields[2].clone(),
                name: fields[3].clone(),
                active: fields[4] == "1",
            })
        })
        .collect()
}

pub fn parse_list_panes(output: &str) -> Vec<TmuxPane> {
    parse_list_panes_with_command_state(output, false)
}

pub fn parse_list_panes_with_command_state(
    output: &str,
    include_command_state: bool,
) -> Vec<TmuxPane> {
    let field_count = if include_command_state { 15 } else { 13 };
    output
        .lines()
        .filter_map(|line| {
            let fields = parse_fields(line, field_count)?;
            Some(TmuxPane {
                id: fields[3].clone(),
                window_id: fields[1].clone(),
                width: fields[4].parse().ok()?,
                height: fields[5].parse().ok()?,
                window_active: fields[6] == "1",
                active_in_window: fields[7] == "1",
                cwd: non_empty(&fields[8]),
                command: fields[9].clone(),
                command_running: include_command_state
                    .then(|| fields.get(13).map(|value| value == "1"))
                    .flatten(),
                command_status: include_command_state
                    .then(|| fields.get(14).and_then(|value| value.parse().ok()))
                    .flatten(),
                in_mode: fields[10] == "1",
                title: fields[11].clone(),
                pid: fields[12].parse().ok()?,
            })
        })
        .collect()
}

fn parse_fields(line: &str, count: usize) -> Option<Vec<String>> {
    let mut fields = Vec::with_capacity(count);
    let mut current = String::new();
    let mut escaped = false;
    for character in line.chars() {
        match character {
            '|' if !escaped => fields.push(std::mem::take(&mut current)),
            '\\' if !escaped => escaped = true,
            _ => {
                current.push(character);
                escaped = false;
            }
        }
    }
    if escaped {
        current.push('\\');
    }
    fields.push(current);
    if fields.len() != count {
        return None;
    }
    Some(fields)
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        TmuxSessionState, parse_list_panes, parse_list_panes_with_command_state, parse_list_windows,
    };

    #[test]
    fn parses_windows_and_panes_with_escaped_pipe() {
        let windows = parse_list_windows("$0|@1|0|editor|1\n");
        let panes = parse_list_panes("$0|@1|0|%2|120|40|1|1|/srv/a\\|b|vim|0|api \\| logs|123\n");
        assert_eq!(windows[0].name, "editor");
        assert!(windows[0].active);
        assert_eq!(panes[0].cwd.as_deref(), Some("/srv/a|b"));
        assert_eq!(panes[0].title, "api | logs");
    }

    #[test]
    fn active_pane_uses_active_window_then_pane() {
        let windows = parse_list_windows("$0|@1|0|one|0\n$0|@2|1|two|1\n");
        let panes = parse_list_panes(
            "$0|@1|0|%1|80|24|0|1|/one|sh|0|one|1\n\
             $0|@2|1|%2|80|24|1|0|/two|sh|0|two|2\n\
             $0|@2|1|%3|80|24|1|1|/two-active|bash|0|two|3\n",
        );
        let state = TmuxSessionState {
            windows,
            panes,
            ..TmuxSessionState::default()
        };
        assert_eq!(state.active_pane().map(|pane| pane.id.as_str()), Some("%3"));
        assert_eq!(state.active_cwd(), Some("/two-active"));
    }

    #[test]
    fn parses_optional_command_state_for_tmux_38() {
        let panes = parse_list_panes_with_command_state(
            "$0|@1|0|%2|80|24|1|1|/tmp|vim|0|editor|42|1|0\n",
            true,
        );
        assert_eq!(panes[0].command_running, Some(true));
        assert_eq!(panes[0].command_status, Some(0));
    }
}
