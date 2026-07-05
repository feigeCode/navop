use gpui::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, PathPromptOptions,
    SharedString, StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, button::Button, h_flex, v_flex};

use crate::theme::{AgentChatTheme, active_agent_chat_theme};

use super::agent_input::{AgentInput, AgentInputEvent};

const SKILL_PANEL_WIDTH: f32 = 380.0;
const SKILL_LIST_MAX_HEIGHT: f32 = 320.0;

/// Skill 面板摘要。`selected_skills` 表示本轮会注入 prompt 的技能数量。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposerSkillSummary {
    pub total_skills: usize,
    pub selected_skills: usize,
}

impl ComposerSkillSummary {
    pub fn new(total_skills: usize, selected_skills: usize) -> Self {
        Self {
            total_skills,
            selected_skills,
        }
    }
}

/// Skill 管理列表项。`selected` 表示本会话是否启用该 skill。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerSkillItem {
    pub id: SharedString,
    pub name: SharedString,
    pub description: SharedString,
    pub path: SharedString,
    pub enabled: bool,
    pub selected: bool,
}

impl ComposerSkillItem {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        description: impl Into<SharedString>,
        path: impl Into<SharedString>,
        enabled: bool,
        selected: bool,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            path: path.into(),
            enabled,
            selected,
        }
    }

    pub fn element_id(&self) -> SharedString {
        SharedString::from(format!("skill-item-{}", self.id))
    }
}

pub(super) fn skill_trigger_label(summary: &ComposerSkillSummary) -> SharedString {
    if summary.total_skills == 0 {
        return SharedString::from("Skill");
    }
    if summary.selected_skills == 0 {
        return SharedString::from(format!("Skill · {}", summary.total_skills));
    }
    SharedString::from(format!(
        "Skill · {}/{}",
        summary.selected_skills, summary.total_skills
    ))
}

pub(super) fn render_skill_mode_content(
    view: Entity<AgentInput>,
    summary: ComposerSkillSummary,
    items: Vec<ComposerSkillItem>,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let theme = active_agent_chat_theme(cx);
    let muted = theme.muted_foreground;
    let mut col = v_flex()
        .p_1()
        .gap(px(2.0))
        .w(px(SKILL_PANEL_WIDTH))
        .max_w(px(SKILL_PANEL_WIDTH));
    col = col.child(
        h_flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(skill_group_label("Skill", &theme))
            .child(import_skill_button(view.clone(), &theme)),
    );
    col = col.child(
        div()
            .px_2()
            .text_xs()
            .text_color(muted)
            .child(skill_summary_label(&summary)),
    );
    let mut list = v_flex()
        .id("agent-skill-list")
        .w_full()
        .min_w_0()
        .px_1()
        .pb_1()
        .gap(px(2.0))
        .max_h(px(SKILL_LIST_MAX_HEIGHT))
        .overflow_x_hidden()
        .overflow_y_scroll();
    if items.is_empty() {
        list = list.child(
            div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(muted)
                .child("暂无 Skill"),
        );
    }
    for item in items {
        list = list.child(skill_item_row(view.clone(), item, &theme, cx));
    }
    col = col.child(list);
    col.into_any_element()
}

fn import_skill_button(view: Entity<AgentInput>, theme: &AgentChatTheme) -> Button {
    Button::new("agent-import-skill")
        .debug_selector(|| "agent-import-skill".to_string())
        .icon(IconName::Plus)
        .small()
        .label("导入技能")
        .bg(theme.panel)
        .border_color(theme.border)
        .text_color(theme.foreground)
        .on_click(move |_, _window, cx| {
            view.update(cx, |_this, cx| prompt_import_skill(cx));
        })
}

fn prompt_import_skill(cx: &mut Context<AgentInput>) {
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("选择 Skill 目录".into()),
    });
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(paths))) = rx.await else {
            return;
        };
        let Some(path) = paths.first().cloned() else {
            return;
        };
        let _ = this.update(cx, |this, cx| {
            if !this.is_running() {
                cx.emit(AgentInputEvent::ImportSkill { path });
            }
        });
    })
    .detach();
}

fn skill_summary_label(summary: &ComposerSkillSummary) -> SharedString {
    SharedString::from(format!(
        "{} available · {} selected",
        summary.total_skills, summary.selected_skills
    ))
}

fn skill_item_row(
    view: Entity<AgentInput>,
    item: ComposerSkillItem,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let id = item.id.clone();
    let muted = theme.muted_foreground;
    let hover_bg = theme.hover_background();
    h_flex()
        .id(item.element_id())
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(cx.theme().radius)
        .when(item.selected, |this| this.bg(theme.selection_background()))
        .when(item.enabled, |this| {
            this.cursor_pointer().hover(move |this| this.bg(hover_bg))
        })
        .when(!item.enabled, |this| this.opacity(0.5))
        .on_click(move |_, _window, cx| {
            let id = id.clone();
            view.update(cx, |this, cx| this.toggle_skill(id, cx));
        })
        .child(Icon::new(IconName::BookOpen).xsmall().text_color(muted))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.0))
                .child(div().text_sm().truncate().child(item.name))
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .truncate()
                        .child(item.description),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .truncate()
                        .child(item.path),
                ),
        )
        .when(item.selected, |this| {
            this.child(Icon::new(IconName::Check).xsmall().text_color(theme.accent))
        })
        .into_any_element()
}

fn skill_group_label(label: &'static str, theme: &AgentChatTheme) -> gpui::AnyElement {
    div()
        .px_2()
        .pt_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(label)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_item_exposes_stable_element_id_and_selection_state() {
        let item = ComposerSkillItem::new(
            "ops",
            "ops",
            "Run operational playbooks",
            "/tmp/skills/ops/SKILL.md",
            true,
            false,
        );

        assert_eq!(item.element_id().as_ref(), "skill-item-ops");
        assert_eq!(item.name.as_ref(), "ops");
        assert_eq!(item.description.as_ref(), "Run operational playbooks");
        assert!(item.enabled);
        assert!(!item.selected);
    }

    #[test]
    fn skill_summary_tracks_available_and_selected_counts() {
        let summary = ComposerSkillSummary::new(4, 2);

        assert_eq!(4, summary.total_skills);
        assert_eq!(2, summary.selected_skills);
        assert_eq!(ComposerSkillSummary::default().total_skills, 0);
    }

    #[test]
    fn skill_trigger_label_reflects_counts() {
        assert_eq!(
            skill_trigger_label(&ComposerSkillSummary::default()).as_ref(),
            "Skill"
        );
        assert_eq!(
            skill_trigger_label(&ComposerSkillSummary::new(3, 0)).as_ref(),
            "Skill · 3"
        );
        assert_eq!(
            skill_trigger_label(&ComposerSkillSummary::new(3, 2)).as_ref(),
            "Skill · 2/3"
        );
    }
}
