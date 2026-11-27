use nsh_base::theme;
use nsh_core::prelude::*;

use crate::{route, route::Route, services::NavopNshServices, ui};

#[derive(Clone, Copy)]
struct ToolLink {
    route: Route,
    title: &'static str,
    description: &'static str,
    glyph: &'static str,
}

pub(crate) fn toolbox() -> impl IntoElement {
    let extensions = NavopNshServices::current()
        .map(|services| services.extensions.clone())
        .unwrap_or_default();
    let extension_count = extensions.len();
    let tools = [
        ToolLink {
            route: Route::Json,
            title: "JSON formatter",
            description: "Validate, format and copy JSON documents.",
            glyph: "{}",
        },
        ToolLink {
            route: Route::Notes,
            title: "Notes",
            description: "Browse and edit the local Markdown notebook.",
            glyph: "N",
        },
        ToolLink {
            route: Route::SessionLogs,
            title: "Session logs",
            description: "Inspect terminal history and recording metadata.",
            glyph: ">_",
        },
        ToolLink {
            route: Route::CredentialVault,
            title: "Credential vault",
            description: "Review shared passwords, SSH keys and agent settings.",
            glyph: "K",
        },
        ToolLink {
            route: Route::Extensions,
            title: "Extensions",
            description: "Open installed nsh apps and inspect capabilities.",
            glyph: "E",
        },
        ToolLink {
            route: Route::Monitor,
            title: "Host monitor",
            description: "Sample CPU, memory, disk and network for SSH hosts.",
            glyph: "M",
        },
    ];

    view()
        .size_full()
        .child(ui::page_header(
            "Toolbox",
            format!("{extension_count} extension app(s) available through the shared nsh host"),
        ))
        .child(
            scroll().flex_1().child(
                view()
                    .p_6()
                    .flex_row()
                    .flex_wrap()
                    .gap_3()
                    .children(tools.into_iter().map(tool_card))
                    .children(extensions.into_iter().map(extension_tool_card)),
            ),
        )
}

fn tool_card(tool: ToolLink) -> impl IntoElement {
    ui::card()
        .w(px(260.0))
        .min_h(px(150.0))
        .flex_none()
        .justify_between()
        .role(AccessibilityRole::Button)
        .label(tool.title)
        .hover(|style| style.border_color(move || theme().primary))
        .active(|style| style.opacity(0.75))
        .on_press(move || route::navigate(tool.route))
        .child(
            view()
                .size(px(34.0))
                .items_center()
                .justify_center()
                .rounded(px(9.0))
                .bg(move || theme().accent)
                .child(
                    text(tool.glyph)
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                ),
        )
        .child(
            view()
                .gap_1()
                .child(
                    text(tool.title)
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(tool.description)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
}

fn extension_tool_card(extension: crate::NavopExtensionInfo) -> impl IntoElement {
    let extension_id = extension.id.clone();
    let title = extension.title.clone();
    let description = extension
        .description
        .unwrap_or_else(|| "Open the installed extension app.".to_owned());
    ui::card()
        .w(px(260.0))
        .min_h(px(150.0))
        .flex_none()
        .justify_between()
        .role(AccessibilityRole::Button)
        .label(title.clone())
        .hover(|style| style.border_color(move || theme().primary))
        .active(|style| style.opacity(0.75))
        .on_press(move || route::open_extension(extension_id.clone(), None))
        .child(
            view()
                .size(px(34.0))
                .items_center()
                .justify_center()
                .rounded(px(9.0))
                .bg(move || theme().accent)
                .child(
                    text("EXT")
                        .text_size(px(9.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                ),
        )
        .child(
            view()
                .gap_1()
                .child(
                    text(title)
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(description)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
}
