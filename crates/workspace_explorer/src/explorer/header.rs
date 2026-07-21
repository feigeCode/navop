use super::WorkspaceExplorer;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Icon, IconName, Sizable as _, Size, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use rust_i18n::t;

#[derive(Clone, Copy)]
pub(super) enum ExplorerSection {
    Changes,
    Files,
}

impl WorkspaceExplorer {
    pub(super) fn render_root_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string());
        let branch = self
            .repository
            .as_ref()
            .and_then(|repository| repository.branch.clone());
        h_flex()
            .items_center()
            .gap_2()
            .h(px(38.0))
            .px_2()
            .border_b_1()
            .border_color(self.theme.border)
            .bg(self.theme.muted)
            .child(
                Icon::new(IconName::FolderOpen)
                    .with_size(Size::Small)
                    .text_color(self.theme.foreground),
            )
            .child(self.render_root_identity(label, branch))
            .child(
                Button::new("workspace-collapse-all")
                    .icon(IconName::ChevronsUpDown)
                    .ghost()
                    .compact()
                    .tooltip(t!("WorkspaceExplorer.tooltip.collapse_folders"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded.clear();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("workspace-refresh")
                    .icon(IconName::Refresh)
                    .ghost()
                    .compact()
                    .tooltip(t!("WorkspaceExplorer.tooltip.refresh"))
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
    }

    fn render_root_identity(&self, label: String, branch: Option<String>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_w_0()
            .child(div().truncate().text_sm().child(label))
            .when_some(branch, |this, branch| {
                this.child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(self.theme.muted_foreground)
                        .child(branch),
                )
            })
    }

    pub(super) fn render_section_header(
        &self,
        section: ExplorerSection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (id, label, expanded) = self.section_header_details(section);
        h_flex()
            .id(id)
            .items_center()
            .gap_1()
            .h(px(28.0))
            .px_2()
            .cursor_pointer()
            .bg(self.theme.muted.opacity(0.55))
            .hover(|style| style.bg(self.theme.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_section(section, cx);
            }))
            .child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .with_size(Size::XSmall)
                .text_color(self.theme.muted_foreground),
            )
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(self.theme.foreground)
                    .child(label),
            )
    }

    fn section_header_details(&self, section: ExplorerSection) -> (&'static str, String, bool) {
        match section {
            ExplorerSection::Changes => (
                "workspace-changes-header",
                t!(
                    "WorkspaceExplorer.section.changes",
                    count = self.changes.len()
                )
                .to_string(),
                self.changes_expanded,
            ),
            ExplorerSection::Files => (
                "workspace-files-header",
                t!("WorkspaceExplorer.section.files").to_string(),
                self.files_expanded,
            ),
        }
    }

    fn toggle_section(&mut self, section: ExplorerSection, cx: &mut Context<Self>) {
        match section {
            ExplorerSection::Changes => self.changes_expanded = !self.changes_expanded,
            ExplorerSection::Files => self.files_expanded = !self.files_expanded,
        }
        cx.notify();
    }
}
