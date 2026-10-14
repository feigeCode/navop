//! 设置 - 同步 - 「同步数据范围」勾选列表
//!
//! 让用户自选哪些数据类型参与云同步。行为要点（详见 `docs/design/sync-scope-selection.md`）：
//!
//! - **连接 / 工作空间**：始终启用且置灰不可取消。
//! - **钥匙串 / LLM 提供商**：仅 Personal 同步可用；切到 Navop 云同步时强制取消并置灰。
//! - **其余类型**：新增项统一默认关闭（Q1），Personal 下首次开启弹一次确认（R2 提示）。
//! - 勾选配置本身**不参与同步**（R7 / Q9）。

use gpui::{AnyElement, App, IntoElement, ParentElement as _, Styled as _, Window, div};
use gpui_component::checkbox::Checkbox;
use gpui_component::dialog::DialogButtonProps;
use gpui_component::setting::{RenderOptions, SettingGroup, SettingItem};
use gpui_component::{ActiveTheme as _, Disableable, Sizable, WindowExt as _, h_flex, v_flex};
use one_core::settings::{
    AppSettings, LockMode, SYNC_SCOPE_DESCRIPTORS, SyncProvider, SyncScopeDescriptor, SyncScopeKey,
    SyncScopeSettings,
};
use rust_i18n::t;

/// 构建「同步数据范围」分组。
///
/// `provider` 必须传**当前值**而不是默认值：`setting_pages` 每次渲染都会重建，
/// 切换同步方式后置灰状态会自动重新求值，不需要手动 refresh。
pub fn sync_scope_group(defaults: &SyncScopeSettings, provider: SyncProvider) -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.Sync.Scope.group_title"))
        .description(t!("Settings.Sync.Scope.group_desc"))
        .items(sync_scope_items(defaults, provider))
}

pub fn sync_scope_items(defaults: &SyncScopeSettings, provider: SyncProvider) -> Vec<SettingItem> {
    SYNC_SCOPE_DESCRIPTORS
        .iter()
        .map(|descriptor| sync_scope_item(descriptor, defaults, provider))
        .collect()
}

fn sync_scope_item(
    descriptor: &'static SyncScopeDescriptor,
    defaults: &SyncScopeSettings,
    provider: SyncProvider,
) -> SettingItem {
    let locked = is_locked(descriptor, provider);
    let default_value = defaults.get(descriptor.key);

    SettingItem::render(move |options, window, cx| {
        render_sync_scope_row(descriptor, options, window, cx)
    })
    .disabled(locked)
    // `default_value` 只用于「重置」按钮的脏值判断；Element 变体不会真正渲染它，
    // 但保留它能让分组的 reset 行为与其他条目保持一致。
    .on_reset(
        move |cx: &App| AppSettings::global(cx).sync_scope.get(descriptor.key) != default_value,
        move |_window: &mut Window, cx: &mut App| {
            AppSettings::update_and_save(cx, |settings| {
                settings.sync_scope.set(descriptor.key, default_value);
            });
        },
    )
}

/// 该行是否置灰（禁止交互）
fn is_locked(descriptor: &SyncScopeDescriptor, provider: SyncProvider) -> bool {
    match descriptor.lock {
        // 核心项：不允许取消，否则会出现「同步已开启但没有东西在同步」
        LockMode::AlwaysOn => true,
        // 仅 Personal 支持：Navop 云同步下强制取消并置灰
        LockMode::PersonalOnly => provider == SyncProvider::OnetCloud,
        LockMode::Optional => false,
    }
}

/// 注意：必须返回 `AnyElement` 而不是 `impl IntoElement`。
/// `SettingItem::render` 要求闭包返回 `E: IntoElement`，而 `impl IntoElement` 会把
/// `options` / `window` / `cx` 的生命周期带进返回类型，触发 "lifetime may not live
/// long enough"。与 `render_personal_sync_path_field` 一致，统一 `into_any_element()`。
fn render_sync_scope_row(
    descriptor: &'static SyncScopeDescriptor,
    options: &RenderOptions,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let settings = AppSettings::global(cx);
    let checked = settings.sync_scope.get(descriptor.key);
    let key = descriptor.key;
    let title = t!(descriptor.i18n_title).to_string();
    let description = t!(descriptor.i18n_desc).to_string();
    let disabled = options.is_disabled();
    let muted = cx.theme().muted_foreground;

    h_flex()
        .w_full()
        .gap_3()
        .items_center()
        .justify_between()
        .child(
            v_flex()
                .flex_1()
                .gap_1()
                .child(div().text_sm().child(title))
                .child(div().text_xs().text_color(muted).child(description)),
        )
        .child(
            Checkbox::new(descriptor.data_type)
                .checked(checked)
                .disabled(disabled)
                .with_size(options.size())
                .on_click(move |checked: &bool, window, cx| {
                    handle_toggle(key, *checked, window, cx);
                }),
        )
        .into_any_element()
}

/// 勾选变更入口；Personal 下首次开启需要用户确认（Q1 / R2）。
fn handle_toggle(key: SyncScopeKey, enabled: bool, window: &mut Window, cx: &mut App) {
    let settings = AppSettings::global(cx);
    let needs_confirm = enabled
        && settings.sync_provider == SyncProvider::Personal
        && settings.sync_scope.is_first_enable(key)
        && !settings.sync_scope.suppress_personal_warning;

    if !needs_confirm {
        apply_toggle(key, enabled, cx);
        return;
    }

    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title(t!("Settings.Sync.Scope.personal_confirm_title").to_string())
            .child(t!("Settings.Sync.Scope.personal_confirm_message").to_string())
            .confirm()
            .button_props(
                DialogButtonProps::default()
                    .ok_text(t!("Common.confirm").to_string())
                    .cancel_text(t!("Common.cancel").to_string()),
            )
            .on_ok(move |_, _window, cx: &mut App| {
                apply_toggle(key, true, cx);
                true
            })
    });
}

fn apply_toggle(key: SyncScopeKey, enabled: bool, cx: &mut App) {
    AppSettings::update_and_save(cx, |settings| {
        settings.sync_scope.set(key, enabled);
        if enabled {
            settings.sync_scope.mark_enabled(key);
        }
    });
}
