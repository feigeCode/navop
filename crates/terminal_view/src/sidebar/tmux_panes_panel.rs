use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Sizable, Size, h_flex, v_flex};
use one_assets::IconName;
use rust_i18n::t;
use terminal::tmux::{TmuxPane, TmuxSessionState, TmuxWindow};

use crate::theme::TerminalColors;

#[derive(Clone, Debug)]
pub enum TmuxPanesPanelEvent {
    Close,
    SelectWindow { window_id: String },
    SelectPane { window_id: String, pane_id: String },
}

pub struct TmuxPanesPanel {
    state: Option<TmuxSessionState>,
    colors: TerminalColors,
    focus_handle: FocusHandle,
}

fn panes_for_window<'a>(panes: &'a [TmuxPane], window_id: &str) -> Vec<&'a TmuxPane> {
    panes
        .iter()
        .filter(|pane| pane.window_id == window_id)
        .collect()
}

impl EventEmitter<TmuxPanesPanelEvent> for TmuxPanesPanel {}

impl Focusable for TmuxPanesPanel {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TmuxPanesPanel {
    pub fn new(colors: TerminalColors, cx: &mut Context<Self>) -> Self {
        Self {
            state: None,
            colors,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_colors(&mut self, colors: TerminalColors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
    }

    pub fn set_state(&mut self, state: Option<TmuxSessionState>, cx: &mut Context<Self>) {
        self.state = state;
        cx.notify();
    }

    pub fn state(&self) -> Option<&TmuxSessionState> {
        self.state.as_ref()
    }

    fn render_window(
        &self,
        window: &TmuxWindow,
        panes: &[&TmuxPane],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors.clone();
        let active = window.active;
        let window_id = window.id.clone();
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .id(SharedString::from(format!("tmux-window-{}", window.id)))
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .when(active, |this| this.bg(colors.muted))
                    .when(!active, |this| {
                        this.hover(|this| this.bg(colors.muted.opacity(0.6)))
                    })
                    .on_click(cx.listener(move |_this, _, _, cx| {
                        cx.emit(TmuxPanesPanelEvent::SelectWindow {
                            window_id: window_id.clone(),
                        });
                    }))
                    .child(
                        div()
                            .w_1()
                            .h_1()
                            .rounded_full()
                            .when(active, |this| this.bg(colors.accent))
                            .when(!active, |this| this.bg(colors.border)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(format!("{}: {}", window.index, window.name)),
                    ),
            )
            .children(panes.iter().map(|pane| self.render_pane(window, pane, cx)))
            .into_any_element()
    }

    fn render_pane(
        &self,
        window: &TmuxWindow,
        pane: &TmuxPane,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors.clone();
        let active = window.active && pane.active_in_window;
        let window_id = window.id.clone();
        let pane_id = pane.id.clone();
        let cwd = pane.cwd.clone().unwrap_or_default();
        let details = if cwd.is_empty() {
            pane.command.clone()
        } else {
            format!("{}  {}", pane.command, cwd)
        };
        h_flex()
            .id(SharedString::from(format!("tmux-pane-{}", pane.id)))
            .items_center()
            .gap_2()
            .pl_5()
            .pr_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .when(active, |this| {
                this.bg(colors.accent).text_color(colors.accent_foreground)
            })
            .when(!active, |this| {
                this.hover(|this| this.bg(colors.muted.opacity(0.6)))
            })
            .on_click(cx.listener(move |_this, _, _, cx| {
                cx.emit(TmuxPanesPanelEvent::SelectPane {
                    window_id: window_id.clone(),
                    pane_id: pane_id.clone(),
                });
            }))
            .child(
                gpui_component::Icon::new(if active {
                    IconName::Check
                } else {
                    IconName::Terminal
                })
                .with_size(Size::XSmall)
                .text_color(if active {
                    colors.accent_foreground
                } else {
                    colors.muted_foreground
                }),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(pane.id.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .when(!active, |this| this.text_color(colors.muted_foreground))
                    .child(details),
            )
            .when(pane.in_mode, |this| {
                this.child(
                    div()
                        .px_1()
                        .rounded_sm()
                        .bg(colors.accent.opacity(0.2))
                        .text_xs()
                        .text_color(colors.accent)
                        .child("COPY"),
                )
            })
            .into_any_element()
    }
}

impl Render for TmuxPanesPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors.clone();
        let Some(state) = self.state.as_ref() else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .p_4()
                .text_sm()
                .text_color(colors.muted_foreground)
                .child(t!("TerminalWorkspace.tmux_detached"))
                .into_any_element();
        };

        let windows = state.windows.clone();
        let panes = state.panes.clone();
        v_flex()
            .size_full()
            .min_h_0()
            .bg(colors.background)
            .text_color(colors.foreground)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(t!("TerminalWorkspace.tmux_pane_tree").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(format!("{} / {}", state.session_name, state.panes.len())),
                    ),
            )
            .child(
                div()
                    .id("tmux-panes-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_2()
                    .child(
                        v_flex()
                            .flex_shrink_0()
                            .gap_1()
                            .children(windows.iter().map(|window| {
                                let window_panes = panes_for_window(&panes, &window.id);
                                self.render_window(window, &window_panes, cx)
                            })),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::panes_for_window;
    use terminal::tmux::TmuxPane;

    fn pane(id: &str, window_id: &str) -> TmuxPane {
        TmuxPane {
            id: id.to_string(),
            window_id: window_id.to_string(),
            width: 80,
            height: 24,
            window_active: true,
            active_in_window: id == "%1",
            cwd: Some("/tmp".to_string()),
            command: "bash".to_string(),
            command_running: None,
            command_status: None,
            in_mode: false,
            title: String::new(),
            pid: 1,
        }
    }

    #[test]
    fn pane_tree_groups_only_panes_from_the_requested_window() {
        let panes = vec![pane("%1", "@1"), pane("%2", "@1"), pane("%3", "@2")];
        let first = panes_for_window(&panes, "@1");
        assert_eq!(
            first
                .iter()
                .map(|pane| pane.id.as_str())
                .collect::<Vec<_>>(),
            vec!["%1", "%2"]
        );
        assert_eq!(2, first.len());
    }
}
