use nsh_base::theme;
use nsh_core::prelude::*;

use crate::{route, services::NavopNshServices, ui};

pub(crate) fn home() -> impl IntoElement {
    let services = NavopNshServices::current();
    let connection_count = services
        .as_ref()
        .and_then(|services| services.connections.list_personal().ok())
        .map_or(0, |connections| connections.len());
    let note_count = services
        .as_ref()
        .and_then(|services| services.notes.as_ref())
        .and_then(|notes| notes.scan_tree().ok())
        .map_or(0, |nodes| count_documents(&nodes));
    let host_count = services
        .as_ref()
        .and_then(|services| services.known_hosts.list_known_hosts().ok())
        .map_or(0, |hosts| hosts.len());

    view()
        .size_full()
        .child(ui::page_header(
            "Workspace",
            route::Route::Home.description(),
        ))
        .child(
            scroll().flex_1().child(
                view()
                    .p_6()
                    .gap_5()
                    .child(
                        view()
                            .flex_row()
                            .gap_3()
                            .children([
                                ui::stat("Saved connections", connection_count.to_string())
                                    .into_element(),
                                ui::stat("Notes", note_count.to_string()).into_element(),
                                ui::stat("Trusted hosts", host_count.to_string()).into_element(),
                            ]),
                    )
                    .child(
                        ui::card()
                            .child(
                                text("nsh is now the UI runtime")
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().foreground),
                            )
                            .child(
                                text(
                                    "This surface runs as an AppSource::Rust app inside the same \
                                     process as the existing domain services. Pages use nsh-core \
                                     primitives and nsh-base behaviors, not gpui-component views.",
                                )
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                            )
                            .child(
                                view()
                                    .flex_row()
                                    .gap_2()
                                    .child(
                                        ui::primary_button("Open connections")
                                            .on_press(|| route::navigate(route::Route::Connections)),
                                    )
                                    .child(
                                        ui::secondary_button("Open notes")
                                            .on_press(|| route::navigate(route::Route::Notes)),
                                    ),
                            ),
                    )
                    .child(ui::card().child(
                        text("Migration order")
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().foreground),
                    ).child(
                        text(
                            "Shared shell → connection inventory → notes/JSON/Known Hosts → \
                             data grids and trees → terminal/SFTP → AI → desktop integrations.",
                        )
                        .text_sm()
                        .text_color(move || theme().muted_foreground),
                    )),
            ),
        )
}

fn count_documents(nodes: &[notes::FileNode]) -> usize {
    nodes
        .iter()
        .map(|node| match node.kind {
            notes::NodeKind::Document => 1,
            notes::NodeKind::Directory => count_documents(&node.children),
        })
        .sum()
}
