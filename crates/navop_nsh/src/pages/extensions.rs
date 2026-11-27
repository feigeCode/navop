use nsh_base::{input, theme};
use nsh_core::prelude::*;

use crate::{services::NavopExtensionInfo, services::NavopNshServices, ui};

pub(crate) fn extensions() -> impl IntoElement {
    let items = NavopNshServices::current()
        .map(|services| services.extensions.clone())
        .unwrap_or_default();
    let requested = crate::route::take_extension_request();
    let initial = requested
        .as_ref()
        .and_then(|(extension_id, _)| items.iter().find(|item| item.id == *extension_id).cloned());
    let query = signal(String::new());
    let extensions = signal(items);
    let filtered = computed(move || {
        let query = query.get().trim().to_lowercase();
        extensions.with(|items| {
            items
                .iter()
                .filter(|item| extension_matches(item, &query))
                .cloned()
                .collect::<Vec<_>>()
        })
    });
    let count = move || extensions.with(Vec::len);
    let visible = move || filtered.with(Vec::len);
    let selected = signal(initial);
    let extension_route = signal(requested.and_then(|(_, route)| route));
    let manager_open = signal(false);

    view()
        .size_full()
        .child(ui::page_header(
            "Extensions",
            "Installed nsh apps and their declared host capabilities",
        ))
        .child(
            view()
                .flex_none()
                .flex_row()
                .items_center()
                .gap_2()
                .px_6()
                .py_3()
                .border_b_1()
                .border_b_color(move || theme().border)
                .child(
                    view()
                        .flex_row()
                        .gap_1()
                        .child(extension_mode_button(
                            "Installed apps",
                            false,
                            manager_open,
                        ))
                        .child(extension_mode_button(
                            "Manage / Marketplace",
                            true,
                            manager_open,
                        )),
                )
                .child(view().flex_1())
                .child(
                    text(
                        "Install, update, reload and remove extensions through the existing extension runtime.",
                    )
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
                ),
        )
        .when(manager_open.get(), |page| {
            page.child(
                native("navop.extension_manager")
                    .flex_1()
                    .min_h_0(),
            )
        })
        .when(!manager_open.get(), |page| {
            page
        .child(
            view()
                .flex_1()
                .min_h_0()
                .flex_row()
                .child(
                    view()
                        .w(px(380.0))
                        .h_full()
                        .min_h_0()
                        .flex_none()
                        .p_4()
                        .gap_3()
                        .border_r_1()
                        .border_r_color(move || theme().border)
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .child(
                                    text(move || format!("{} installed", count()))
                                        .text_sm()
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(
                                    text(move || format!("{} shown", visible()))
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(view().flex_1())
                                .child(
                                    input()
                                        .value(move || query.get())
                                        .on_change(move |value: &str| query.set(value.to_owned()))
                                        .placeholder("Search extensions")
                                        .w(px(180.0))
                                        .h(px(34.0))
                                        .px_3()
                                        .rounded(move || theme().radius)
                                        .border_1()
                                        .border_color(move || theme().input)
                                        .bg(move || theme().card)
                                        .text_sm()
                                        .text_color(move || theme().foreground),
                                ),
                        )
                        .child(
                            scroll()
                                .flex_1()
                                .min_h_0()
                                .gap_2()
                                .children(
                                    filtered
                                        .with(|items| items.clone())
                                        .into_iter()
                                        .map(move |item| {
                                            extension_row(item, selected, extension_route)
                                        }),
                                )
                                .when(filtered.with(Vec::is_empty), |list| {
                                    list.child(
                                        ui::card().child(
                                            text(if extensions.with(Vec::is_empty) {
                                                "No nsh extensions are installed"
                                            } else {
                                                "No extensions match this search"
                                            })
                                            .text_sm()
                                            .text_color(move || theme().muted_foreground),
                                        ),
                                    )
                                }),
                        ),
                )
                .child(
                    view()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .when_some(selected.get(), |pane, item| {
                            pane.child(
                                view()
                                    .flex_1()
                                    .min_h_0()
                                    .gap_2()
                                    .p_4()
                                    .child(
                                        text(item.title)
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().foreground),
                                    )
                                    .child(
                                        native("navop.extension")
                                            .prop(
                                                "extension_id",
                                                serde_json::Value::String(item.id),
                                            )
                                            .prop(
                                                "route",
                                                extension_route
                                                    .get()
                                                    .map(serde_json::Value::String)
                                                    .unwrap_or(serde_json::Value::Null),
                                            )
                                            .flex_1()
                                            .min_h_0(),
                                    ),
                            )
                        })
                        .when(selected.with(Option::is_none), |pane| {
                            pane.child(
                                view().size_full().items_center().justify_center().child(
                                    text("Select an extension app to open it here")
                                        .text_sm()
                                        .text_color(move || theme().muted_foreground),
                                ),
                            )
                        }),
                ),
        )
        })
}

fn extension_matches(item: &NavopExtensionInfo, query: &str) -> bool {
    query.is_empty()
        || item.title.to_lowercase().contains(query)
        || item.id.to_lowercase().contains(query)
        || item
            .description
            .as_deref()
            .is_some_and(|description| description.to_lowercase().contains(query))
        || item
            .category
            .as_deref()
            .is_some_and(|category| category.to_lowercase().contains(query))
        || item
            .keywords
            .iter()
            .any(|keyword| keyword.to_lowercase().contains(query))
}

fn extension_row(
    item: NavopExtensionInfo,
    selected: Signal<Option<NavopExtensionInfo>>,
    extension_route: Signal<Option<String>>,
) -> impl IntoElement {
    let open_item = item.clone();
    let description = item
        .description
        .unwrap_or_else(|| "No description provided".to_owned());
    let category = item.category.unwrap_or_else(|| "general".to_owned());
    let modules = if item.modules.is_empty() {
        "No host modules".to_owned()
    } else {
        item.modules.join(", ")
    };
    let permissions = if item.permissions.is_empty() {
        "No declared permissions".to_owned()
    } else {
        item.permissions.join(", ")
    };
    ui::card()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    view()
                        .size(px(36.0))
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded(px(9.0))
                        .bg(move || theme().muted)
                        .child(
                            text("EXT")
                                .text_size(px(9.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(
                    view()
                        .flex_1()
                        .min_w_0()
                        .child(
                            text(item.title)
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(format!("{} · v{} · {category}", item.id, item.version))
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                ),
        )
        .child(
            text(description)
                .text_sm()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(format!("Modules: {modules}"))
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(format!("Permissions: {permissions}"))
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(item.entry_path.display().to_string())
                        .flex_1()
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .child(ui::primary_button("Open").on_press(move || {
                    extension_route.set(None);
                    selected.set(Some(open_item.clone()));
                })),
        )
}

fn extension_mode_button(
    label: &'static str,
    manager: bool,
    open: Signal<bool>,
) -> impl IntoElement {
    view()
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if open.get() == manager {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if open.get() == manager {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: open.get() == manager,
            ..AccessibilityState::default()
        })
        .on_press(move || open.set(manager))
        .child(text(label).text_xs().text_color(move || theme().foreground))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn extension() -> NavopExtensionInfo {
        NavopExtensionInfo {
            id: "acme.dev-tools".to_owned(),
            title: "Dev Tools".to_owned(),
            version: "1.2.0".to_owned(),
            description: Some("Inspect application resources".to_owned()),
            category: Some("developer".to_owned()),
            keywords: vec!["debug".to_owned()],
            permissions: Vec::new(),
            modules: Vec::new(),
            entry_path: PathBuf::from("/tmp/dev-tools/bundle.js"),
        }
    }

    #[test]
    fn extension_search_matches_metadata() {
        let item = extension();
        assert!(extension_matches(&item, ""));
        assert!(extension_matches(&item, "dev"));
        assert!(extension_matches(&item, "acme"));
        assert!(extension_matches(&item, "debug"));
        assert!(!extension_matches(&item, "database"));
    }
}
