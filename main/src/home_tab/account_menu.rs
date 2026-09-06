use super::*;
use gpui_component::avatar::Avatar;

/// 用户行触发器：Popover 的 trigger 要求实现 Selectable，用本地包装承载。
struct AccountTrigger {
    avatar: Option<(SharedString, Option<String>)>,
    name: SharedString,
    collapsed: bool,
    selected: bool,
    hover_bg: gpui::Hsla,
    muted_fg: gpui::Hsla,
}

impl gpui_component::Selectable for AccountTrigger {
    fn is_selected(&self) -> bool {
        self.selected
    }

    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

impl IntoElement for AccountTrigger {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let avatar = neutral_avatar(self.avatar.as_ref(), &self.name, USER_ROW_AVATAR);
        let row = h_flex()
            .id("home-account-user-row")
            .w_full()
            .gap_2()
            .items_center()
            .p_1p5()
            .rounded(px(9.0))
            .cursor_pointer()
            .when(self.selected, |row| row.bg(self.hover_bg))
            .when(!self.selected, |row| {
                row.hover(|style| style.bg(self.hover_bg))
            })
            .child(avatar);
        if self.collapsed {
            row.justify_center().into_any_element()
        } else {
            // 常驻只保留用户名（邮箱收进菜单头部，redesign §8.2：减少无效的常驻信息）。
            row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(self.name),
            )
            .child(
                Icon::new(IconName::ChevronUp)
                    .small()
                    .text_color(self.muted_fg),
            )
            .into_any_element()
        }
    }
}

/// 账户菜单宽度（demo：min-width 196px，含同步状态行适当加宽）。
const ACCOUNT_MENU_WIDTH: gpui::Pixels = px(240.0);
/// 用户行头像尺寸（demo：27px 圆形头像）。
const USER_ROW_AVATAR: gpui::Pixels = px(27.0);

/// 中性头像 fallback：无真实头像时用单色用户图标 + secondary 底，
/// 替代 Avatar 的 hash 自动色（随机黄色字母与品牌色无关）。
/// 有真实头像 URL 时保留原图。
fn neutral_avatar_for_url(url: Option<String>, size: gpui::Pixels) -> AnyElement {
    match url {
        Some(url) => Avatar::new()
            .src(url)
            .with_size(gpui_component::Size::Size(size))
            .into_any_element(),
        // 单色用户图标直接承担头像槽位，继承行文字颜色（中性）。
        None => Icon::new(IconName::CircleUser)
            .size(size)
            .mono()
            .flex_shrink_0()
            .into_any_element(),
    }
}

/// AccountTrigger 使用的头像（`avatar` 为 (fallback 名, URL)）。
fn neutral_avatar(
    avatar: Option<&(SharedString, Option<String>)>,
    name: &SharedString,
    size: gpui::Pixels,
) -> AnyElement {
    let url = avatar.and_then(|(_, url)| url.clone());
    let _ = name; // fallback 用统一单色图标，不取首字母
    neutral_avatar_for_url(url, size)
}

impl HomePage {
    pub(super) fn render_account_entry(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let collapsed = self.sidebar_collapsed;
        let trigger = AccountTrigger {
            avatar: self.current_user.as_ref().map(|user| {
                (
                    user.resolved_display_name().into(),
                    user.avatar_url
                        .as_deref()
                        .map(str::trim)
                        .filter(|url| !url.is_empty())
                        .map(str::to_string),
                )
            }),
            name: self
                .current_user
                .as_ref()
                .map(UserInfo::resolved_display_name)
                .unwrap_or_else(|| t!("Auth.login").to_string())
                .into(),
            collapsed,
            selected: false,
            hover_bg: cx.theme().muted,
            muted_fg: cx.theme().muted_foreground,
        };
        Popover::new("home-account-menu")
            .anchor(Anchor::TopLeft)
            .trigger(trigger)
            .open(self.account_menu_open)
            .on_open_change(cx.listener(|this, open, _, cx| {
                this.account_menu_open = *open;
                cx.notify();
            }))
            .content(move |state, window, cx| {
                let _ = (state, window);
                view.update(cx, |home, cx| home.render_account_menu_content(cx))
            })
            .into_any_element()
    }

    fn account_avatar(&self, size: gpui::Pixels) -> AnyElement {
        let url = self
            .current_user
            .as_ref()
            .and_then(|user| {
                user.avatar_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
            })
            .map(str::to_string);
        neutral_avatar_for_url(url, size)
    }

    /// 账户菜单内容（demo：用户信息头 + 同步状态 + 密钥 + 退出登录）。
    fn render_account_menu_content(&self, cx: &mut Context<Self>) -> AnyElement {
        let logged_in = self.current_user.is_some();
        let syncing = self.syncing
            || matches!(
                crate::personal_sync_runtime::runtime_status(cx),
                crate::personal_sync_status::PersonalSyncRuntimeStatus::Syncing
            );
        let show_team_key = is_feature_enabled(Feature::TeamManagement, cx)
            && should_show_team_key_menu_item(sync_route(cx), self.team_permissions.teams().len());
        let personal_unlocked = crypto::has_master_key();

        let mut menu = v_flex()
            .w(ACCOUNT_MENU_WIDTH)
            .gap_0p5()
            .child(self.render_account_menu_header(cx))
            .child(self.menu_divider(cx));

        let sync_trailing: SharedString = if syncing {
            t!("Home.syncing").to_string().into()
        } else {
            relative_sync_label(crate::personal_sync_status::last_sync_completed_at()).into()
        };
        let view = cx.entity();
        menu = menu.child(account_menu_row(
            view.clone(),
            IconName::Refresh,
            t!("Home.sync").to_string(),
            Some(sync_trailing),
            syncing,
            false,
            cx,
            |home, window, cx| home.handle_sync_click(window, cx),
        ));

        menu = menu.child(account_menu_row(
            view.clone(),
            IconName::Key,
            t!("Encryption.personal_key").to_string(),
            Some(
                if personal_unlocked {
                    t!("Home.unlock_state_unlocked").to_string()
                } else {
                    t!("Home.unlock_state_locked").to_string()
                }
                .into(),
            ),
            false,
            false,
            cx,
            |home, window, cx| home.show_encryption_key_dialog(window, cx),
        ));

        if show_team_key {
            menu = menu.child(account_menu_row(
                view.clone(),
                IconName::Building2,
                t!("Encryption.team_key").to_string(),
                None,
                false,
                false,
                cx,
                |home, window, cx| home.add_team_key_settings_tab(window, cx),
            ));
        }

        menu = menu.child(self.menu_divider(cx)).child(account_menu_row(
            view,
            IconName::CircleUser,
            if logged_in {
                t!("Auth.logout").to_string()
            } else {
                t!("Auth.login").to_string()
            },
            None,
            false,
            logged_in,
            cx,
            move |home, window, cx| {
                if logged_in {
                    home.sign_out(cx);
                } else {
                    home.show_login_dialog(window, cx);
                }
            },
        ));
        menu.into_any_element()
    }

    fn render_account_menu_header(&self, cx: &App) -> AnyElement {
        let name = self
            .current_user
            .as_ref()
            .map(UserInfo::resolved_display_name)
            .unwrap_or_else(|| t!("Auth.login").to_string());
        let email = self
            .current_user
            .as_ref()
            .map(|user| user.email.clone())
            .filter(|email| !email.trim().is_empty())
            .unwrap_or_default();
        h_flex()
            .w_full()
            .gap_2p5()
            .items_center()
            .px_2()
            .py_2p5()
            .child(self.account_avatar(px(32.0)))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(email),
                    ),
            )
            .into_any_element()
    }

    fn menu_divider(&self, cx: &App) -> AnyElement {
        div()
            .h(px(1.0))
            .w_full()
            .my_1()
            .bg(cx.theme().border)
            .into_any_element()
    }
}

/// 菜单行（demo §5.6：padding 7 9、圆角 7、12.5px、图标 15px muted、右侧状态 muted）。
fn account_menu_row(
    view: Entity<HomePage>,
    icon: IconName,
    label: String,
    trailing: Option<SharedString>,
    disabled: bool,
    danger: bool,
    cx: &App,
    action: impl Fn(&mut HomePage, &mut Window, &mut Context<HomePage>) + 'static,
) -> AnyElement {
    let foreground = if danger {
        cx.theme().danger
    } else {
        cx.theme().foreground
    };
    let icon_color = if danger {
        cx.theme().danger
    } else {
        cx.theme().muted_foreground
    };
    let mut row = h_flex()
        .id(SharedString::from(format!("account-menu-{label}")))
        .w_full()
        .gap_2()
        .items_center()
        .px_2p5()
        .py_1p5()
        .rounded(px(7.0))
        .text_sm()
        .text_color(foreground)
        .child(Icon::new(icon).small().text_color(icon_color))
        .child(div().flex_1().min_w_0().text_ellipsis().child(label));
    if let Some(trailing) = trailing {
        row = row.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(trailing),
        );
    }
    if disabled {
        row.opacity(0.6).into_any_element()
    } else {
        row.cursor_pointer()
            .hover(|style| style.bg(cx.theme().muted))
            .on_click(move |_, window, cx| {
                view.update(cx, |home, cx| {
                    home.account_menu_open = false;
                    cx.notify();
                    action(home, window, cx);
                });
            })
            .into_any_element()
    }
}

/// 同步相对时间文案（刚刚 / N 分钟前 / N 小时前 / N 天前）。
fn relative_sync_label(timestamp: Option<i64>) -> String {
    let Some(timestamp) = timestamp else {
        return t!("Home.sync_not_yet").to_string();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let diff = now.saturating_sub(timestamp);
    if diff < 60 {
        t!("Home.sync_just_now").to_string()
    } else if diff < 3600 {
        t!("Home.sync_minutes_ago", count = diff / 60).to_string()
    } else if diff < 86400 {
        t!("Home.sync_hours_ago", count = diff / 3600).to_string()
    } else {
        t!("Home.sync_days_ago", count = diff / 86400).to_string()
    }
}
