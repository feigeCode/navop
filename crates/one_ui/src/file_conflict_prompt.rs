use std::{rc::Rc, sync::Arc, sync::atomic::AtomicBool, sync::atomic::Ordering};

use gpui::{App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use gpui_component::{
    ActiveTheme,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex, v_flex,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileConflictChoice {
    Skip,
    KeepBoth,
    Merge,
    Overwrite,
}

#[derive(Clone)]
pub struct FileConflictPromptLabels {
    pub exists: SharedString,
    pub progress: SharedString,
    pub choose_action: SharedString,
    pub apply_all: SharedString,
    pub skip: SharedString,
    pub keep_both: SharedString,
    pub merge: SharedString,
    pub overwrite: SharedString,
}

type ChoiceHandler = Rc<dyn Fn(FileConflictChoice, bool, &mut Window, &mut App)>;

pub struct FileConflictPromptSpec {
    pub name: SharedString,
    pub is_directory: bool,
    pub apply_all: Arc<AtomicBool>,
    pub labels: FileConflictPromptLabels,
}

#[derive(IntoElement)]
pub struct FileConflictPrompt {
    name: SharedString,
    is_directory: bool,
    apply_all: Arc<AtomicBool>,
    labels: FileConflictPromptLabels,
    on_choice: ChoiceHandler,
}

impl FileConflictPrompt {
    pub fn new(
        spec: FileConflictPromptSpec,
        on_choice: impl Fn(FileConflictChoice, bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            name: spec.name,
            is_directory: spec.is_directory,
            apply_all: spec.apply_all,
            labels: spec.labels,
            on_choice: Rc::new(on_choice),
        }
    }

    fn action_button(&self, choice: FileConflictChoice, label: SharedString) -> Button {
        let apply_all = self.apply_all.clone();
        let on_choice = self.on_choice.clone();
        Button::new(format!("file-conflict-{choice:?}"))
            .label(label)
            .on_click(move |_, window, cx| {
                on_choice(choice, apply_all.load(Ordering::Relaxed), window, cx);
            })
    }
}

impl RenderOnce for FileConflictPrompt {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let checked = self.apply_all.load(Ordering::Relaxed);
        let apply_all = self.apply_all.clone();
        let mut actions = vec![
            self.action_button(FileConflictChoice::Skip, self.labels.skip.clone())
                .ghost()
                .into_any_element(),
            self.action_button(FileConflictChoice::KeepBoth, self.labels.keep_both.clone())
                .ghost()
                .into_any_element(),
        ];
        if self.is_directory {
            actions.push(
                self.action_button(FileConflictChoice::Merge, self.labels.merge.clone())
                    .ghost()
                    .into_any_element(),
            );
        }
        actions.push(
            self.action_button(FileConflictChoice::Overwrite, self.labels.overwrite.clone())
                .primary()
                .into_any_element(),
        );

        v_flex()
            .gap_3()
            .child(self.labels.exists)
            .child(
                div()
                    .p_2()
                    .rounded_md()
                    .bg(cx.theme().secondary)
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(self.name),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.labels.progress),
                    ),
            )
            .child(self.labels.choose_action)
            .child(
                Checkbox::new("file-conflict-apply-all")
                    .checked(checked)
                    .label(self.labels.apply_all)
                    .on_click(move |checked, window, _cx| {
                        apply_all.store(*checked, Ordering::Relaxed);
                        window.refresh();
                    }),
            )
            .child(h_flex().justify_end().gap_2().children(actions))
    }
}
