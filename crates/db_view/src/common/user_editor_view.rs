use super::DatabaseUserFormEvent;
use db::GlobalDbState;
use db::plugin::DatabaseUserOperationRequest;
use db::plugin_manifest::DatabaseFormKind;
use gpui::{
    AnyView, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, Styled, Subscription, Window, div,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    highlighter::Language,
    input::{Input, InputState},
    v_flex,
};
use one_core::storage::DatabaseType;
use rust_i18n::t;

pub struct UserEditorView {
    focus_handle: FocusHandle,
    form: AnyView,
    sql_preview: Entity<InputState>,
    current_tab: EditorTab,
    operation: DatabaseFormKind,
    database_type: DatabaseType,
    error_message: Entity<Option<String>>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, PartialEq)]
enum EditorTab {
    Form,
    SqlPreview,
}

impl UserEditorView {
    pub fn new<F>(
        form: Entity<F>,
        database_type: DatabaseType,
        operation: DatabaseFormKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self
    where
        F: Render + EventEmitter<DatabaseUserFormEvent> + 'static,
    {
        let sql_preview = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(Language::from_str("sql"))
                .line_number(false)
                .multi_line(true)
        });
        let form_subscription = cx.subscribe_in(
            &form,
            window,
            move |this, _form, event, window, cx| match event {
                DatabaseUserFormEvent::FormChanged(request) => {
                    this.update_sql_preview(request, window, cx);
                }
            },
        );

        Self {
            focus_handle: cx.focus_handle(),
            form: form.into(),
            sql_preview,
            current_tab: EditorTab::Form,
            operation,
            database_type,
            error_message: cx.new(|_| None),
            _subscriptions: vec![form_subscription],
        }
    }

    fn update_sql_preview(
        &mut self,
        request: &DatabaseUserOperationRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sql = self
            .build_sql(request, cx)
            .unwrap_or_else(|| "-- 当前数据库类型暂不支持该用户操作。".to_string());
        self.sql_preview.update(cx, |state, cx| {
            state.set_value(sql, window, cx);
        });
    }

    fn build_sql(&self, request: &DatabaseUserOperationRequest, cx: &App) -> Option<String> {
        let plugin = cx
            .global::<GlobalDbState>()
            .get_plugin(&self.database_type)
            .ok()?;
        match self.operation {
            DatabaseFormKind::CreateUser => plugin.build_create_user_sql(request),
            DatabaseFormKind::EditUser => plugin.build_modify_user_sql(request),
            DatabaseFormKind::DeleteUser => plugin.build_drop_user_sql(request),
            DatabaseFormKind::UserPrivileges => plugin.build_user_privileges_sql(request),
            _ => None,
        }
    }

    pub fn get_sql(&self, cx: &App) -> String {
        self.sql_preview.read(cx).text().to_string()
    }

    pub fn set_save_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error_message.update(cx, |message, cx| {
            *message = Some(error);
            cx.notify();
        });
    }
}

impl Focusable for UserEditorView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for UserEditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let form_button = tab_button(
            "user_tab_form",
            t!("EditorView.form_tab").to_string(),
            self.current_tab == EditorTab::Form,
        );
        let sql_button = tab_button(
            "user_tab_sql",
            t!("EditorView.sql_preview_tab").to_string(),
            self.current_tab == EditorTab::SqlPreview,
        );
        let main_content = if self.current_tab == EditorTab::Form {
            div().flex_1().w_full().child(self.form.clone())
        } else {
            div()
                .flex_1()
                .w_full()
                .min_h_48()
                .child(Input::new(&self.sql_preview).size_full().disabled(true))
        };

        let mut container = v_flex()
            .size_full()
            .child(
                h_flex()
                    .gap_2()
                    .p_2()
                    .border_b_1()
                    .border_color(gpui::rgb(0xe0e0e0))
                    .child(tab_on_click(form_button, EditorTab::Form, cx))
                    .child(tab_on_click(sql_button, EditorTab::SqlPreview, cx)),
            )
            .child(main_content);

        if let Some(message) = self.error_message.read(cx).clone() {
            container = container.child(
                div()
                    .mx_4()
                    .mb_4()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(gpui::rgb(0xfee2e2))
                    .text_color(gpui::rgb(0x991b1b))
                    .child(format!("× {}", message)),
            );
        }
        container
    }
}

fn tab_button(id: &'static str, label: String, active: bool) -> Button {
    let button = Button::new(id).label(label);
    if active {
        button.primary()
    } else {
        button.ghost()
    }
}

fn tab_on_click(button: Button, tab: EditorTab, cx: &mut Context<UserEditorView>) -> Button {
    button.on_click(cx.listener(move |this, _, _, cx| {
        this.current_tab = tab;
        cx.notify();
    }))
}
