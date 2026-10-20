//! 设置 - 同步 - 「同步数据范围」勾选列表
//!
//! 让用户自选哪些数据类型参与云同步。行为要点（详见 `docs/design/sync-scope-selection.md`）：
//!
//! - **连接 / 工作空间**：始终启用且置灰不可取消。
//! - **钥匙串 / LLM 提供商**：仅 Personal 同步可用；切到 Navop 云同步时强制取消并置灰。
//! - **其余类型**：新增项统一默认关闭（Q1），Personal 下首次开启弹一次确认（R2 提示）。
//! - 勾选配置本身**不参与同步**（R7 / Q9）。

use chrono::TimeZone as _;
use gpui::{AnyElement, App, IntoElement, ParentElement as _, Styled as _, Window, div};
use gpui_component::checkbox::Checkbox;
use gpui_component::dialog::DialogButtonProps;
use gpui_component::setting::{RenderOptions, SettingGroup, SettingItem};
use gpui_component::{ActiveTheme as _, Disableable, Sizable, WindowExt as _, h_flex, v_flex};
use one_core::cloud_sync::scope::state::SyncTypeStatusRepository;
use one_core::crypto;
use one_core::settings::{
    AppSettings, LockMode, SYNC_SCOPE_DESCRIPTORS, SyncProvider, SyncScopeDescriptor, SyncScopeKey,
    SyncScopeSettings,
};
use one_core::storage::GlobalStorageState;
use rust_i18n::t;

/// Personal 同步通道的 scope / user 标识（与 `build_scope_source` 保持一致）
const PERSONAL_SCOPE_ENV: &str = "personal";
const PERSONAL_USER_ID: &str = "personal";

/// 构建「同步数据范围」分组。
///
/// `provider` 必须传**当前值**而不是默认值：`setting_pages` 每次渲染都会重建，
/// 切换同步方式后置灰状态会自动重新求值，不需要手动 refresh。
pub fn sync_scope_group(defaults: &SyncScopeSettings, provider: SyncProvider) -> SettingGroup {
    let mut items = Vec::new();

    // Q11：主密钥未设置 / 未解锁时无法加密，任何数据都同步不上去。
    // 这是「配置未就绪」而不是故障，单独提示，避免用户以为是同步坏了。
    if !crypto::has_master_key() {
        items.push(
            SettingItem::render(|_options, _window, cx| {
                div()
                    .text_sm()
                    .text_color(cx.theme().warning_foreground)
                    .child(t!("Settings.Sync.Scope.master_key_required").to_string())
                    .into_any_element()
            })
            .keywords(["master key", "主密钥"]),
        );
    }

    items.extend(sync_scope_items(defaults, provider));

    SettingGroup::new()
        .title(t!("Settings.Sync.Scope.group_title"))
        .description(t!("Settings.Sync.Scope.group_desc"))
        .items(items)
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
        .child(status_label(cx, descriptor))
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

/// 每行的状态列（Q12）：上次同步时间，或最近一次失败警告。
fn status_label(cx: &App, descriptor: &'static SyncScopeDescriptor) -> AnyElement {
    let Some((text, is_error)) = status_text(cx, descriptor.data_type) else {
        return div().into_any_element();
    };
    let color = if is_error {
        cx.theme().danger_foreground
    } else {
        cx.theme().muted_foreground
    };
    div()
        .text_xs()
        .text_color(color)
        .child(text)
        .into_any_element()
}

fn status_text(cx: &App, record_type: &str) -> Option<(String, bool)> {
    // 主密钥未解锁：所有类型统一显示，且优先于其它状态
    if !crypto::has_master_key() {
        return Some((
            t!("Settings.Sync.Scope.master_key_locked").to_string(),
            true,
        ));
    }

    let storage = cx.try_global::<GlobalStorageState>()?;
    let repo = SyncTypeStatusRepository::new(storage.storage.connection());
    let status = repo
        .get(PERSONAL_SCOPE_ENV, PERSONAL_USER_ID, record_type)
        .ok()
        .flatten();

    // 从未同步过的类型也要给出状态，不能整行空白：
    // 需求要求「每个同步数据都显示上次同步时间或失败报错信息」。
    let Some(status) = status else {
        return Some((t!("Settings.Sync.Scope.never_synced").to_string(), false));
    };

    if status.has_recent_error() {
        let code = status.last_error_code.clone().unwrap_or_default();
        let message = status
            .last_error_msg
            .clone()
            .unwrap_or_else(|| code.clone());
        return Some((format!("⚠ {message}"), true));
    }

    let Some(success_at) = status.last_success_at else {
        return Some((t!("Settings.Sync.Scope.never_synced").to_string(), false));
    };

    Some((
        t!(
            "Settings.Sync.Scope.last_synced",
            time = format_absolute_time(success_at)
        )
        .to_string(),
        false,
    ))
}

/// 上次同步时间：按**本地时区**输出 `YYYY/MM/DD HH:MM:SS`。
///
/// 用绝对时间而不是「刚刚 / N 分钟前」：相对文案在自动同步（60s 一轮）下几乎
/// 永远显示「刚刚」，看不出到底什么时候同步过。
fn format_absolute_time(timestamp: i64) -> String {
    chrono::Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%Y/%m/%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "-".to_string())
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
