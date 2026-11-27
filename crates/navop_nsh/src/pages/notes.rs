use std::path::PathBuf;

use notes::{FileNode, NodeKind, NotesStorage};
use nsh_base::{input, theme};
use nsh_core::prelude::*;

use crate::{services::NavopNshServices, ui};

#[derive(Clone, PartialEq)]
struct NoteRow {
    path: PathBuf,
    name: String,
    depth: usize,
    document: bool,
}

pub(crate) fn notes() -> impl IntoElement {
    let storage = NavopNshServices::current().and_then(|services| services.notes.clone());
    let rows = signal(load_rows(storage.as_deref()));
    let selected = signal(None::<PathBuf>);
    let source = signal(String::new());
    let status = signal(String::from("Select a document"));

    let select_document = {
        let storage = storage.clone();
        move |path: PathBuf| {
            let Some(storage) = storage.as_ref() else {
                status.set("Notes storage is unavailable".to_owned());
                return;
            };
            match storage.read_document(&path) {
                Ok((_, text)) => {
                    selected.set(Some(path));
                    source.set(text);
                    status.set("Loaded".to_owned());
                }
                Err(error) => status.set(error.to_string()),
            }
        }
    };
    let save = {
        let storage = storage.clone();
        move || {
            let (Some(storage), Some(path)) = (storage.as_ref(), selected.get_untracked()) else {
                status.set("Select a document first".to_owned());
                return;
            };
            match storage.write_document(&path, &source.get_untracked()) {
                Ok(_) => status.set("Saved".to_owned()),
                Err(error) => status.set(error.to_string()),
            }
        }
    };
    let create_note = {
        let storage = storage.clone();
        move || {
            let Some(storage) = storage.as_ref() else {
                status.set("Notes storage is unavailable".to_owned());
                return;
            };
            let mut name = String::from("Untitled");
            let mut suffix = 2;
            while storage
                .descriptor(std::path::Path::new(&format!("{name}.md")))
                .is_ok()
            {
                name = format!("Untitled {suffix}");
                suffix += 1;
            }
            match storage.create_document(std::path::Path::new(""), &name) {
                Ok(document) => {
                    rows.set(load_rows(Some(storage)));
                    selected.set(Some(document.relative_path));
                    source.set(String::new());
                    status.set("Created".to_owned());
                }
                Err(error) => status.set(error.to_string()),
            }
        }
    };

    view()
        .size_full()
        .child(ui::page_header(
            "Notes",
            "Local notebook storage with an nsh-native editor surface",
        ))
        .child(
            view()
                .flex_1()
                .min_h_0()
                .flex_row()
                .child(
                    view()
                        .w(px(270.0))
                        .h_full()
                        .min_h_0()
                        .flex_none()
                        .border_r_1()
                        .border_r_color(move || theme().border)
                        .p_3()
                        .gap_2()
                        .child(
                            view()
                                .flex_row()
                                .gap_2()
                                .child(ui::primary_button("New").on_press(create_note))
                                .child(ui::secondary_button("Reload").on_press({
                                    let storage = storage.clone();
                                    move || {
                                        rows.set(load_rows(storage.as_deref()));
                                        status.set("Reloaded".to_owned());
                                    }
                                })),
                        )
                        .child(scroll().flex_1().min_h_0().gap_1().children(
                            rows.with(|rows| rows.clone()).into_iter().map({
                                let select_document = select_document.clone();
                                move |row| note_row(row, select_document.clone())
                            }),
                        )),
                )
                .child(
                    view()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .p_4()
                        .gap_2()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .child(
                                    text(move || {
                                        selected
                                            .get()
                                            .and_then(|path| {
                                                path.file_name()
                                                    .map(|name| name.to_string_lossy().into_owned())
                                            })
                                            .unwrap_or_else(|| "No document selected".to_owned())
                                    })
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().foreground),
                                )
                                .child(
                                    text(move || status.get())
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(view().flex_1())
                                .child(ui::primary_button("Save").on_press(save)),
                        )
                        .child(
                            input()
                                .multiline(true)
                                .value(move || source.get())
                                .on_change(move |value: &str| source.set(value.to_owned()))
                                .placeholder("Select or create a note")
                                .flex_1()
                                .min_h_0()
                                .p_4()
                                .rounded(move || theme().radius)
                                .border_1()
                                .border_color(move || theme().input)
                                .bg(move || theme().card)
                                .text_color(move || theme().foreground)
                                .text_size(px(14.0)),
                        ),
                ),
        )
}

fn load_rows(storage: Option<&NotesStorage>) -> Vec<NoteRow> {
    let Some(storage) = storage else {
        return Vec::new();
    };
    let Ok(nodes) = storage.scan_tree() else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    flatten_tree(&nodes, 0, &mut rows);
    rows
}

fn flatten_tree(nodes: &[FileNode], depth: usize, rows: &mut Vec<NoteRow>) {
    for node in nodes {
        rows.push(NoteRow {
            path: node.relative_path.clone(),
            name: node.display_name.clone(),
            depth,
            document: node.kind == NodeKind::Document,
        });
        if node.kind == NodeKind::Directory {
            flatten_tree(&node.children, depth + 1, rows);
        }
    }
}

fn note_row<F>(row: NoteRow, select_document: F) -> impl IntoElement
where
    F: Fn(PathBuf) + Clone + 'static,
{
    let path = row.path.clone();
    view()
        .h(px(30.0))
        .flex_none()
        .flex_row()
        .items_center()
        .gap_2()
        .pl(px(8.0 + row.depth as f32 * 14.0))
        .pr_2()
        .rounded(move || theme().radius)
        .hover(|s| s.bg(move || theme().muted))
        .active(|s| s.opacity(0.7))
        .when(row.document, |row| {
            row.role(AccessibilityRole::Button)
                .on_press(move || select_document(path.clone()))
        })
        .child(
            text(if row.document { "·" } else { "▾" })
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(text(row.name).text_sm().text_color(move || {
            if row.document {
                theme().foreground
            } else {
                theme().muted_foreground
            }
        }))
}
