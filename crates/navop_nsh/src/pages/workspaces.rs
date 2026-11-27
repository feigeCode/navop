use nsh_base::{input, theme};
use nsh_core::prelude::*;
use one_core::storage::traits::Repository as _;
use one_core::storage::{ConnectionRepository, Workspace, WorkspaceRepository};

use crate::{route, route::Route, services::NavopNshServices, ui};

#[derive(Clone, PartialEq)]
struct WorkspaceRow {
    id: Option<i64>,
    name: String,
    color: Option<String>,
    connection_count: usize,
}

pub(crate) fn workspaces() -> impl IntoElement {
    let services = NavopNshServices::current();
    let repository = services
        .as_ref()
        .and_then(|services| services.workspaces.clone());
    let connections = services.map(|services| services.connections.clone());
    let rows = signal(load_workspaces(repository.as_ref(), connections.as_ref()));
    let status = signal(String::new());
    let name = signal(String::new());
    let editing_id = signal(None::<i64>);

    let save = {
        let repository = repository.clone();
        let connections = connections.clone();
        move || {
            let Some(repository) = repository.as_ref() else {
                status.set("Workspace storage is unavailable".to_owned());
                return;
            };
            let workspace_name = name.get_untracked().trim().to_owned();
            if workspace_name.is_empty() {
                status.set("Workspace name is required".to_owned());
                return;
            }
            let result = if let Some(id) = editing_id.get_untracked() {
                let Some(mut workspace) = repository.get(id).ok().flatten() else {
                    status.set(format!("Workspace #{id} was not found"));
                    return;
                };
                workspace.name = workspace_name;
                repository.update(&workspace).map(|_| id)
            } else {
                let mut workspace = Workspace::new(workspace_name);
                repository.insert(&mut workspace)
            };
            match result {
                Ok(id) => {
                    rows.set(load_workspaces(Some(repository), connections.as_ref()));
                    status.set(format!("Saved workspace #{id}"));
                    name.set(String::new());
                    editing_id.set(None);
                }
                Err(error) => status.set(format!("Failed to save workspace: {error}")),
            }
        }
    };
    let delete = {
        let repository = repository.clone();
        let connections = connections.clone();
        move |id: i64| {
            let Some(repository) = repository.as_ref() else {
                status.set("Workspace storage is unavailable".to_owned());
                return;
            };
            match repository.delete(id) {
                Ok(()) => {
                    rows.set(load_workspaces(Some(repository), connections.as_ref()));
                    status.set(format!("Deleted workspace #{id}"));
                }
                Err(error) => status.set(format!("Delete failed: {error}")),
            }
        }
    };
    let edit = move |workspace: WorkspaceRow| {
        if let Some(id) = workspace.id {
            editing_id.set(Some(id));
            name.set(workspace.name);
        }
    };
    let total = move || rows.with(Vec::len);

    view()
        .size_full()
        .child(ui::page_header(
            "Workspaces",
            format!("{} local workspace(s)", total()),
        ))
        .child(
            scroll().flex_1().child(
                view()
                    .p_6()
                    .gap_3()
                    .child(
                        ui::card()
                            .gap_3()
                            .child(
                                text(move || {
                                    if editing_id.get().is_some() {
                                        "Rename workspace"
                                    } else {
                                        "New workspace"
                                    }
                                })
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                            )
                            .child(
                                view()
                                    .flex_row()
                                    .gap_3()
                                    .child(
                                        input()
                                            .value(move || name.get())
                                            .on_change(move |next: &str| {
                                                name.set(next.to_owned())
                                            })
                                            .placeholder("Workspace name")
                                            .flex_1()
                                            .h(px(36.0))
                                            .px_3()
                                            .rounded(move || theme().radius)
                                            .border_1()
                                            .border_color(move || theme().input)
                                            .bg(move || theme().background)
                                            .text_sm()
                                            .text_color(move || theme().foreground),
                                    )
                                    .child(ui::primary_button("Save").on_press(save))
                                    .when(editing_id.get().is_some(), |actions| {
                                        actions.child(ui::secondary_button("Cancel").on_press(
                                            move || {
                                                editing_id.set(None);
                                                name.set(String::new());
                                            },
                                        ))
                                    }),
                            ),
                    )
                    .child(
                        text(move || status.get())
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                    )
                    .when(rows.with(Vec::is_empty), |page| {
                        page.child(ui::card().child(
                            text("No workspaces yet. Save connections to Personal or create one here.")
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        ))
                    })
                    .children(
                        rows.with(|rows| rows.clone()).into_iter().map({
                            let delete = delete.clone();
                            move |workspace| workspace_row(workspace, edit, delete.clone())
                        }),
                    ),
            ),
        )
}

fn load_workspaces(
    repository: Option<&WorkspaceRepository>,
    connections: Option<&ConnectionRepository>,
) -> Vec<WorkspaceRow> {
    repository
        .and_then(|repository| repository.list().ok())
        .unwrap_or_default()
        .into_iter()
        .map(|workspace| WorkspaceRow {
            id: workspace.id,
            name: workspace.name,
            color: workspace.color,
            connection_count: workspace.id.map_or(0, |id| {
                connections
                    .and_then(|repository| repository.list_by_workspace(Some(id)).ok())
                    .map_or(0, |connections| connections.len())
            }),
        })
        .collect()
}

fn workspace_row(
    workspace: WorkspaceRow,
    edit: impl Fn(WorkspaceRow) + Clone + 'static,
    delete: impl Fn(i64) + Clone + 'static,
) -> impl IntoElement {
    let edit_row = workspace.clone();
    let id = workspace.id;
    ui::card()
        .flex_row()
        .items_center()
        .gap_4()
        .child(
            view().size(px(12.0)).rounded_full().bg(workspace
                .color
                .as_deref()
                .and_then(parse_hex_color)
                .unwrap_or_else(|| theme().primary)),
        )
        .child(
            view()
                .flex_1()
                .min_w_0()
                .child(
                    text(workspace.name)
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(format!("{} connection(s)", workspace.connection_count))
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            ui::secondary_button("Connections")
                .on_press(move || route::navigate(Route::Connections)),
        )
        .when_some(id, |row, id| {
            row.child(ui::secondary_button("Rename").on_press({
                let edit = edit.clone();
                move || edit(edit_row.clone())
            }))
            .child(ui::secondary_button("Delete").on_press(move || delete(id)))
        })
}

fn parse_hex_color(value: &str) -> Option<AdaptiveColor> {
    let value = value.trim().trim_start_matches('#');
    let (red, green, blue) = match value.len() {
        6 => (
            u8::from_str_radix(&value[0..2], 16).ok()?,
            u8::from_str_radix(&value[2..4], 16).ok()?,
            u8::from_str_radix(&value[4..6], 16).ok()?,
        ),
        3 => {
            let expand = |digit: &str| {
                let value = u8::from_str_radix(digit, 16).ok()? * 17;
                Some(value)
            };
            (
                expand(&value[0..1])?,
                expand(&value[1..2])?,
                expand(&value[2..3])?,
            )
        }
        _ => return None,
    };
    let rgb = (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue);
    Some(AdaptiveColor::rgb(rgb, rgb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_color_parser_accepts_hex_and_rejects_invalid_values() {
        assert!(parse_hex_color("#336699").is_some());
        assert!(parse_hex_color("369").is_some());
        assert!(parse_hex_color("not-a-color").is_none());
    }
}
