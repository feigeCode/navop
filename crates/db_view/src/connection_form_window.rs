use gpui::prelude::FluentBuilder;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div,
};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement,
    v_flex,
};
use one_core::cloud_sync::TeamOption;
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event};
use one_core::storage::{DatabaseType, StoredConnection, Workspace};
use rust_i18n::locale;
use rust_i18n::t;

use crate::common::db_connection_form::{DbConnectionForm, DbConnectionFormEvent};
use crate::database_view_plugin::{
    create_connection_form_for, create_external_connection_form_for,
};

/// 连接表单窗口的配置
pub struct ConnectionFormWindowConfig {
    pub db_type: DatabaseType,
    pub external_driver_id: Option<String>,
    pub editing_connection: Option<StoredConnection>,
    pub workspaces: Vec<Workspace>,
    pub teams: Vec<TeamOption>,
    pub ssh_connections: Vec<StoredConnection>,
}

/// 连接表单窗口
///
/// 包含 TitleBar、DbConnectionForm 和操作按钮
pub struct ConnectionFormWindow {
    focus_handle: FocusHandle,
    form: Entity<DbConnectionForm>,
    title: SharedString,
}

fn external_driver_id_from_connection(conn: Option<&StoredConnection>) -> Option<String> {
    conn.and_then(|conn| conn.to_db_connection().ok())
        .and_then(|config| {
            config
                .database_type
                .external_driver_id()
                .map(str::to_string)
        })
}

fn external_driver_id_for_form(
    db_type: &DatabaseType,
    explicit_driver_id: Option<&str>,
    conn: Option<&StoredConnection>,
) -> Option<String> {
    explicit_driver_id
        .map(str::to_string)
        .or_else(|| db_type.external_driver_id().map(str::to_string))
        .or_else(|| external_driver_id_from_connection(conn))
}

fn external_driver_name_for_title(driver_id: Option<&str>) -> Option<String> {
    driver_id.and_then(|driver_id| {
        db::ipc::IpcDriverRegistry::load_default()
            .find(driver_id)
            .map(|driver| driver.name)
    })
}

fn connection_title_for_locale(
    locale: &str,
    is_editing: bool,
    db_type: &DatabaseType,
    external_driver_name: Option<&str>,
) -> String {
    let db_type_label = external_driver_name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| db_type.as_str());

    db::translate_connection_title_for_locale(locale, is_editing, db_type_label)
}

impl ConnectionFormWindow {
    pub fn new(
        config: ConnectionFormWindowConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let is_editing = config.editing_connection.is_some();
        let db_type = config.db_type;

        let external_driver_id = external_driver_id_for_form(
            &db_type,
            config.external_driver_id.as_deref(),
            config.editing_connection.as_ref(),
        );
        let external_driver_name = external_driver_name_for_title(external_driver_id.as_deref());
        let title: SharedString = connection_title_for_locale(
            locale().as_ref(),
            is_editing,
            &db_type,
            external_driver_name.as_deref(),
        )
        .into();

        let form = external_driver_id
            .as_deref()
            .and_then(|driver_id| create_external_connection_form_for(driver_id, window, cx))
            .unwrap_or_else(|| create_connection_form_for(db_type, window, cx));

        form.update(cx, |f, cx| {
            f.set_workspaces(config.workspaces.clone(), window, cx);
            f.set_teams(config.teams.clone(), window, cx);
            f.set_ssh_connections(config.ssh_connections.clone(), window, cx);
        });

        if let Some(ref conn) = config.editing_connection {
            form.update(cx, |f, cx| {
                f.load_connection(conn, window, cx);
            });
        }

        let is_edit = is_editing;
        cx.subscribe_in(
            &form,
            window,
            move |_this, _form, event: &DbConnectionFormEvent, window, cx| match event {
                DbConnectionFormEvent::Saved(conn) => {
                    if is_edit {
                        emit_connection_event(
                            ConnectionDataEvent::ConnectionUpdated {
                                connection: conn.as_ref().clone(),
                            },
                            cx,
                        );
                    } else {
                        emit_connection_event(
                            ConnectionDataEvent::ConnectionCreated {
                                connection: conn.as_ref().clone(),
                            },
                            cx,
                        );
                    }
                    window.remove_window();
                }
                DbConnectionFormEvent::SaveError(_) => {}
            },
        )
        .detach();

        Self {
            focus_handle: cx.focus_handle(),
            form,
            title,
        }
    }

    fn on_test(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.form.update(cx, |form, cx| {
            form.trigger_test_connection(cx);
        });
    }

    fn on_save(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.form.update(cx, |form, cx| {
            form.save_connection(cx);
        });
    }

    fn on_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form.update(cx, |form, cx| {
            form.trigger_cancel(cx);
        });
        window.remove_window();
    }
}

impl Focusable for ConnectionFormWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ConnectionFormWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_testing = self.form.read(cx).is_testing(cx);
        let test_result_msg = self.form.read(cx).test_result_msg(cx);

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(
                TitleBar::new().child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_1()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(self.title.clone()),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .p_4()
                    .overflow_y_scrollbar()
                    .child(self.form.clone()),
            )
            .when_some(test_result_msg, |this, msg| {
                let is_success = msg.starts_with("✓");
                this.child(
                    div()
                        .mx_4()
                        .mb_2()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(if is_success {
                            gpui::rgb(0xdcfce7)
                        } else {
                            gpui::rgb(0xfee2e2)
                        })
                        .text_color(if is_success {
                            gpui::rgb(0x166534)
                        } else {
                            gpui::rgb(0x991b1b)
                        })
                        .child(msg),
                )
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .p_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("cancel")
                            .small()
                            .label(t!("Common.cancel").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_cancel(window, cx);
                            })),
                    )
                    .child(
                        Button::new("test")
                            .small()
                            .outline()
                            .label(if is_testing {
                                t!("Connection.testing").to_string()
                            } else {
                                t!("Connection.test").to_string()
                            })
                            .disabled(is_testing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_test(window, cx);
                            })),
                    )
                    .child(
                        Button::new("ok")
                            .small()
                            .primary()
                            .label(t!("Common.ok").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_save(window, cx);
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::{DbConnectionConfig, StoredConnection};
    use std::collections::HashMap;

    fn stored_external_connection(driver_id: &str) -> StoredConnection {
        StoredConnection::new_database(
            "demo".to_string(),
            DbConnectionConfig {
                id: String::new(),
                database_type: DatabaseType::external(driver_id),
                name: "demo".to_string(),
                host: "localhost".to_string(),
                port: 0,
                username: String::new(),
                password: String::new(),
                database: None,
                service_name: None,
                sid: None,
                workspace_id: None,
                extra_params: HashMap::new(),
            },
            None,
        )
    }

    fn stored_connection_with_extra_driver_param() -> StoredConnection {
        let mut extra_params = HashMap::new();
        extra_params.insert("external_driver_id".to_string(), "iotdb".to_string());

        StoredConnection::new_database(
            "demo".to_string(),
            DbConnectionConfig {
                id: String::new(),
                database_type: DatabaseType::MySQL,
                name: "demo".to_string(),
                host: "localhost".to_string(),
                port: 0,
                username: String::new(),
                password: String::new(),
                database: None,
                service_name: None,
                sid: None,
                workspace_id: None,
                extra_params,
            },
            None,
        )
    }

    #[test]
    fn external_driver_id_from_connection_uses_database_type_identity() {
        let connection = stored_external_connection("iotdb");

        assert_eq!(
            Some("iotdb".to_string()),
            external_driver_id_from_connection(Some(&connection))
        );
    }

    #[test]
    fn external_driver_id_for_form_uses_database_type_without_explicit_config() {
        assert_eq!(
            Some("iotdb".to_string()),
            external_driver_id_for_form(&DatabaseType::external("iotdb"), None, None)
        );
    }

    #[test]
    fn external_driver_id_from_connection_ignores_extra_params_driver_id() {
        let connection = stored_connection_with_extra_driver_param();

        assert_eq!(None, external_driver_id_from_connection(Some(&connection)));
    }

    #[test]
    fn connection_title_uses_external_driver_name() {
        assert_eq!(
            "新建 Dameng DM 连接",
            connection_title_for_locale(
                "zh-CN",
                false,
                &DatabaseType::external("dm"),
                Some("Dameng DM")
            )
        );
    }

    #[test]
    fn connection_title_falls_back_to_database_type_name() {
        assert_eq!(
            "新建 External 连接",
            connection_title_for_locale("zh-CN", false, &DatabaseType::external("dm"), None)
        );
    }
}
