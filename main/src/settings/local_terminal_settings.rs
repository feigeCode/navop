use gpui::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div,
};
use gpui_component::{
    ActiveTheme, IconName, Selectable, Sizable,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    setting::{SettingField, SettingGroup, SettingItem},
    v_flex,
};
use one_core::settings::{
    AppSettings, LocalTerminalCustomProfile, LocalTerminalProfileKind,
    LocalTerminalProfileSettings,
};
use rust_i18n::t;

use crate::local_terminal_profiles::{effective_kind, setting_options};

struct TerminalProfileRow {
    id: String,
    name: Entity<InputState>,
    command: Entity<InputState>,
    _subscriptions: Vec<gpui::Subscription>,
}

struct TerminalProfilesEditor {
    rows: Vec<TerminalProfileRow>,
}

/// 本地终端设置分组：默认 profile 与自定义 profile 编辑。
pub fn local_terminal_setting_group(defaults: &LocalTerminalProfileSettings) -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.General.LocalTerminal.group_title"))
        .items(vec![
            local_terminal_profile_item(defaults.kind),
            SettingItem::render(move |_options, window, cx| render(window, cx)).keywords([
                t!("Settings.General.LocalTerminal.custom_profiles").to_string(),
                t!("Settings.General.LocalTerminal.custom_command").to_string(),
            ]),
        ])
}

fn local_terminal_profile_item(default: LocalTerminalProfileKind) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.LocalTerminal.profile"),
        SettingField::dropdown(
            setting_options(cfg!(target_os = "windows")),
            |cx: &App| {
                SharedString::from(
                    effective_kind(
                        AppSettings::global(cx).local_terminal_profile.kind,
                        cfg!(target_os = "windows"),
                    )
                    .as_str(),
                )
            },
            |value: SharedString, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.local_terminal_profile.kind =
                        LocalTerminalProfileKind::parse(value.as_ref());
                });
            },
        )
        .default_value(SharedString::from(default.as_str())),
    )
    .description(t!("Settings.General.LocalTerminal.profile_desc").to_string())
}

pub(crate) fn render(window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let profiles = AppSettings::global(cx)
        .local_terminal_profile
        .effective_custom_profiles();
    let editor = window.use_keyed_state("local-terminal-custom-profiles", cx, |window, cx| {
        TerminalProfilesEditor::new(profiles, window, cx)
    });
    editor.into_any_element()
}

impl TerminalProfilesEditor {
    fn new(
        profiles: Vec<LocalTerminalCustomProfile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let rows = profiles
            .into_iter()
            .map(|profile| Self::create_row(profile, window, cx))
            .collect();
        Self { rows }
    }

    fn create_row(
        profile: LocalTerminalCustomProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TerminalProfileRow {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(profile.name)
                .placeholder(t!("Settings.General.LocalTerminal.name_placeholder"))
        });
        let command = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(profile.command)
                .placeholder(t!("Settings.General.LocalTerminal.command_placeholder"))
        });
        let name_subscription = cx.subscribe(&name, |editor, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                editor.sync_settings(cx);
            }
        });
        let command_subscription = cx.subscribe(&command, |editor, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                editor.sync_settings(cx);
            }
        });
        TerminalProfileRow {
            id: profile.id,
            name,
            command,
            _subscriptions: vec![name_subscription, command_subscription],
        }
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile = LocalTerminalCustomProfile {
            id: format!("custom-{}", next_profile_id()),
            name: String::new(),
            command: String::new(),
        };
        self.rows.push(Self::create_row(profile, window, cx));
        cx.notify();
    }

    fn remove_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.rows.len() {
            self.rows.remove(index);
            self.sync_settings(cx);
            cx.notify();
        }
    }

    fn profiles(&self, cx: &App) -> Vec<LocalTerminalCustomProfile> {
        self.rows
            .iter()
            .filter_map(|row| {
                let name = row.name.read(cx).value().trim().to_string();
                let command = row.command.read(cx).value().trim().to_string();
                (!name.is_empty() && !command.is_empty()).then(|| LocalTerminalCustomProfile {
                    id: row.id.clone(),
                    name,
                    command,
                })
            })
            .collect()
    }

    fn sync_settings(&self, cx: &mut Context<Self>) {
        let profiles = self.profiles(cx);
        AppSettings::update_and_save(cx, |settings| {
            let selected_id = settings
                .local_terminal_profile
                .default_custom_profile_id
                .clone();
            settings.local_terminal_profile.custom_profiles = profiles;
            let selected_exists = selected_id.as_ref().is_some_and(|id| {
                settings
                    .local_terminal_profile
                    .custom_profiles
                    .iter()
                    .any(|profile| &profile.id == id)
            });
            settings.local_terminal_profile.default_custom_profile_id = if selected_exists {
                selected_id
            } else {
                settings
                    .local_terminal_profile
                    .custom_profiles
                    .first()
                    .map(|profile| profile.id.clone())
            };
        });
    }

    fn select_default(&mut self, id: String, cx: &mut Context<Self>) {
        AppSettings::update_and_save(cx, |settings| {
            settings.local_terminal_profile.default_custom_profile_id = Some(id);
        });
        cx.notify();
    }
}

impl Render for TerminalProfilesEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let default_id = AppSettings::global(cx)
            .local_terminal_profile
            .default_custom_profile_id
            .clone();
        v_flex()
            .w_full()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Settings.General.LocalTerminal.profiles_desc")),
            )
            .children(self.rows.iter().enumerate().map(|(index, row)| {
                let id = row.id.clone();
                let selected = default_id.as_deref() == Some(id.as_str());
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(div().w_32().child(Input::new(&row.name)))
                    .child(div().flex_1().child(Input::new(&row.command)))
                    .child(
                        Button::new(SharedString::from(format!("terminal-default-{id}")))
                            .icon(IconName::Check)
                            .ghost()
                            .small()
                            .selected(selected)
                            .tooltip(t!("Settings.General.LocalTerminal.set_default"))
                            .on_click(cx.listener(move |editor, _, _, cx| {
                                editor.select_default(id.clone(), cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("terminal-remove-{index}")))
                            .icon(IconName::Remove)
                            .ghost()
                            .small()
                            .tooltip(t!("Common.delete"))
                            .on_click(cx.listener(move |editor, _, _, cx| {
                                editor.remove_row(index, cx);
                            })),
                    )
            }))
            .child(
                Button::new("terminal-profile-add")
                    .icon(IconName::Plus)
                    .label(t!("Settings.General.LocalTerminal.add_terminal"))
                    .small()
                    .on_click(cx.listener(|editor, _, window, cx| {
                        editor.add_row(window, cx);
                    })),
            )
    }
}

fn next_profile_id() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}
