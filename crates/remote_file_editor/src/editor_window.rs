use crate::file_policy::{
    EditorMode, FilePolicy, MAX_EDITABLE_FILE_SIZE, decode_text_content, determine_file_policy,
};
use crate::language::language_for_path;
use crate::{CloseIntercept, decide_close_intercept};
use gpui::{
    App, AppContext, Bounds, Context, Entity, IntoElement, ParentElement, PromptLevel, Render,
    Size as GpuiSize, Styled, Window, WindowBounds, WindowKind, WindowOptions, div, px, size,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Root, Selectable as _, Sizable as _, Size, TitleBar,
    WindowExt,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState, Search},
    notification::Notification,
    v_flex,
};
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;
use sftp::{RusshSftpClient, SftpClient};
use std::sync::Arc;
use tokio::sync::Mutex;

pub fn open_remote_file_editor<T: 'static>(
    remote_path: String,
    client: Arc<Mutex<RusshSftpClient>>,
    cx: &mut Context<T>,
) {
    let title = t!(
        "RemoteFileEditor.title",
        name = display_name_from_path(&remote_path)
    )
    .to_string();
    cx.spawn(async move |_this, cx| {
        let title = title.clone();
        let remote_path_for_log = remote_path.clone();
        let result = cx.update(|cx| {
            let mut window_size = size(px(960.0), px(720.0));
            if let Some(display) = cx.primary_display() {
                let display_size = display.bounds().size;
                window_size.width = window_size.width.min(display_size.width * 0.85);
                window_size.height = window_size.height.min(display_size.height * 0.85);
            }
            let window_bounds = Bounds::centered(None, window_size, cx);
            let window_opts = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(window_bounds)),
                titlebar: Some(TitleBar::title_bar_options()),
                window_min_size: Some(GpuiSize {
                    width: px(640.0),
                    height: px(480.0),
                }),
                kind: WindowKind::Normal,
                #[cfg(target_os = "linux")]
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                #[cfg(target_os = "linux")]
                window_decorations: Some(gpui::WindowDecorations::Client),
                ..Default::default()
            };

            cx.open_window(window_opts, move |window, cx| {
                window.activate_window();
                window.set_window_title(&title);
                let view =
                    cx.new(|cx| RemoteFileEditorWindow::new(remote_path, client, window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        });

        if let Err(error) = result {
            tracing::error!(path = %remote_path_for_log, ?error, "failed to open remote file editor");
        }
    })
    .detach();
}

struct LoadedFile {
    text: String,
    policy: FilePolicy,
    file_size: usize,
    language: String,
}

struct RemoteFileEditorWindow {
    remote_path: String,
    display_name: String,
    client: Arc<Mutex<RusshSftpClient>>,
    editor: Option<Entity<InputState>>,
    subscriptions: Vec<gpui::Subscription>,
    saved_text: String,
    file_size: usize,
    policy: FilePolicy,
    loading: bool,
    saving: bool,
    soft_wrap: bool,
    close_prompt_open: bool,
    close_after_save: bool,
    status_message: String,
    load_error: Option<String>,
}

impl RemoteFileEditorWindow {
    fn new(
        remote_path: String,
        client: Arc<Mutex<RusshSftpClient>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            display_name: display_name_from_path(&remote_path),
            remote_path,
            client,
            editor: None,
            subscriptions: Vec::new(),
            saved_text: String::new(),
            file_size: 0,
            policy: FilePolicy {
                mode: EditorMode::Code,
                is_large_file: false,
            },
            loading: true,
            saving: false,
            soft_wrap: false,
            close_prompt_open: false,
            close_after_save: false,
            status_message: t!("RemoteFileEditor.status.loading").to_string(),
            load_error: None,
        };
        this.register_close_guard(window, cx);
        this.reload(window, cx);
        this
    }

    fn register_close_guard(&self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            view.update(cx, |this, cx| this.handle_window_should_close(window, cx))
                .unwrap_or(true)
        });
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        self.load_error = None;
        self.status_message = t!("RemoteFileEditor.status.loading").to_string();
        cx.notify();

        let remote_path = self.remote_path.clone();
        let client = self.client.clone();
        let task = Tokio::spawn(cx, async move {
            let bytes = {
                let mut client = client.lock().await;
                client
                    .read_file(&remote_path, MAX_EDITABLE_FILE_SIZE)
                    .await?
            };
            let file_size = bytes.len();
            let policy = determine_file_policy(file_size)?;
            let text = decode_text_content(&bytes)?;
            let language = language_for_path(&remote_path, policy.is_large_file).to_string();
            Ok::<_, anyhow::Error>(LoadedFile {
                text,
                policy,
                file_size,
                language,
            })
        });

        let view = cx.entity().clone();
        window
            .spawn(cx, async move |cx| match task.await {
                Ok(Ok(loaded)) => {
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.apply_loaded_file(loaded, window, cx);
                    });
                }
                Ok(Err(error)) => {
                    let message = error.to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.loading = false;
                        this.load_error = Some(message.clone());
                        this.status_message = t!("RemoteFileEditor.status.load_failed").to_string();
                        window.push_notification(Notification::error(message), cx);
                    });
                }
                Err(error) => {
                    let message = error.to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.loading = false;
                        this.load_error = Some(message.clone());
                        this.status_message = t!("RemoteFileEditor.status.load_failed").to_string();
                        window.push_notification(Notification::error(message), cx);
                    });
                }
            })
            .detach();
    }

    fn apply_loaded_file(
        &mut self,
        loaded: LoadedFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let LoadedFile {
            text,
            policy,
            file_size,
            language,
        } = loaded;

        let initial_text = text.clone();
        let soft_wrap = self.soft_wrap;
        let editor = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .code_editor(language)
                .line_number(true)
                .searchable(true)
                .soft_wrap(soft_wrap);
            state.set_value(initial_text, window, cx);
            state
        });

        self.subscriptions.clear();
        self.subscriptions.push(
            cx.subscribe(&editor, |_this, _input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        );

        editor.update(cx, |state: &mut InputState, cx| {
            state.focus(window, cx);
        });

        self.editor = Some(editor);
        self.saved_text = text;
        self.file_size = file_size;
        self.policy = policy;
        self.loading = false;
        self.saving = false;
        self.load_error = None;
        self.status_message = if policy.is_large_file {
            t!("RemoteFileEditor.status.loaded_plain_text").to_string()
        } else {
            t!("RemoteFileEditor.status.loaded").to_string()
        };
        cx.notify();
    }

    fn save(&mut self, close_after_save: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.close_after_save |= close_after_save;
        let Some(editor) = self.editor.clone() else {
            if self.close_after_save {
                self.close_after_save = false;
                window.remove_window();
            }
            return;
        };

        if self.saving {
            return;
        }

        let text = editor.read(cx).text().to_string();
        self.saving = true;
        self.status_message = t!("RemoteFileEditor.status.saving").to_string();
        cx.notify();

        let remote_path = self.remote_path.clone();
        let client = self.client.clone();
        let task = Tokio::spawn(cx, async move {
            let mut client = client.lock().await;
            client.write_file(&remote_path, text.as_bytes()).await?;
            Ok::<_, anyhow::Error>(text)
        });

        let view = cx.entity().clone();
        window
            .spawn(cx, async move |cx| match task.await {
                Ok(Ok(saved_text)) => {
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.saved_text = saved_text;
                        this.file_size = this.saved_text.len();
                        this.saving = false;
                        this.status_message = t!("RemoteFileEditor.status.saved").to_string();
                        let close_after_save = this.close_after_save;
                        this.close_after_save = false;
                        if close_after_save {
                            window.remove_window();
                        } else {
                            window.push_notification(
                                Notification::success(
                                    t!("RemoteFileEditor.notification.saved").to_string(),
                                ),
                                cx,
                            );
                        }
                        cx.notify();
                    });
                }
                Ok(Err(error)) => {
                    let message = error.to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.saving = false;
                        this.close_after_save = false;
                        this.status_message = t!("RemoteFileEditor.status.save_failed").to_string();
                        window.push_notification(Notification::error(message), cx);
                    });
                }
                Err(error) => {
                    let message = error.to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.saving = false;
                        this.close_after_save = false;
                        this.status_message = t!("RemoteFileEditor.status.save_failed").to_string();
                        window.push_notification(Notification::error(message), cx);
                    });
                }
            })
            .detach();
    }

    fn handle_window_should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match decide_close_intercept(self.is_dirty(cx), self.close_prompt_open) {
            CloseIntercept::Allow => true,
            CloseIntercept::Ignore => false,
            CloseIntercept::Prompt => {
                self.show_unsaved_changes_prompt(window, cx);
                false
            }
        }
    }

    fn show_unsaved_changes_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_prompt_open = true;
        let prompt_title = t!("RemoteFileEditor.prompt.unsaved_title").to_string();
        let prompt_message = t!("RemoteFileEditor.prompt.unsaved_message").to_string();
        let save_label = t!("RemoteFileEditor.action.save").to_string();
        let discard_label = t!("RemoteFileEditor.action.discard").to_string();
        let cancel_label = t!("RemoteFileEditor.action.cancel").to_string();
        let buttons = [
            save_label.as_str(),
            discard_label.as_str(),
            cancel_label.as_str(),
        ];
        let answer = window.prompt(
            PromptLevel::Warning,
            &prompt_title,
            Some(&prompt_message),
            &buttons,
            cx,
        );
        let window_handle = window.window_handle();

        cx.spawn(async move |this, cx| {
            let selection = answer.await.ok();
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.close_prompt_open = false;
                    match selection {
                        Some(0) => this.save(true, window, cx),
                        Some(1) => window.remove_window(),
                        _ => {
                            this.close_after_save = false;
                        }
                    }
                });
            });
        })
        .detach();
    }

    fn trigger_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };

        editor.update(cx, |state, cx| {
            state.focus(window, cx);
        });
        window.dispatch_action(Box::new(Search), cx);
    }

    fn toggle_soft_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.soft_wrap = !self.soft_wrap;
        if let Some(editor) = self.editor.as_ref() {
            editor.update(cx, |state, cx| {
                state.set_soft_wrap(self.soft_wrap, window, cx);
            });
        }
        cx.notify();
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.editor
            .as_ref()
            .map(|editor| editor.read(cx).text().to_string() != self.saved_text)
            .unwrap_or(false)
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dirty = self.is_dirty(cx);
        let disabled = self.loading || self.saving || self.editor.is_none();

        h_flex()
            .gap_2()
            .items_center()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().title_bar)
            .child(
                Button::new("remote-file-save")
                    .label(t!("RemoteFileEditor.action.save"))
                    .with_size(Size::Small)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.save(false, window, cx);
                    })),
            )
            .child(
                Button::new("remote-file-search")
                    .label(t!("RemoteFileEditor.action.search"))
                    .with_size(Size::Small)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.trigger_search(window, cx);
                    })),
            )
            .child(
                Button::new("remote-file-reload")
                    .label(t!("RemoteFileEditor.action.reload"))
                    .with_size(Size::Small)
                    .disabled(self.loading || self.saving)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.reload(window, cx);
                    })),
            )
            .child(
                Button::new("remote-file-soft-wrap")
                    .label(t!("RemoteFileEditor.action.soft_wrap"))
                    .selected(self.soft_wrap)
                    .with_size(Size::Small)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_soft_wrap(window, cx);
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.policy_label()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(if dirty {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(if dirty {
                        t!("RemoteFileEditor.state.modified")
                    } else {
                        t!("RemoteFileEditor.state.saved")
                    }),
            )
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .items_center()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.remote_path.clone()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format_size(self.file_size)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.status_message.clone()),
            )
    }

    fn render_body(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.loading {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(div().text_sm().child(t!("RemoteFileEditor.body.loading")))
                .into_any_element();
        }

        if let Some(error) = self.load_error.as_ref() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(div().text_base().child(t!("RemoteFileEditor.body.unable_to_open")))
                .child(
                    div()
                        .max_w(px(560.0))
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(error.clone()),
                )
                .into_any_element();
        }

        match self.editor.as_ref() {
            Some(editor) => v_flex()
                .size_full()
                .child(Input::new(editor).size_full())
                .into_any_element(),
            None => v_flex().size_full().into_any_element(),
        }
    }

    fn policy_label(&self) -> String {
        match self.policy.mode {
            EditorMode::Code => t!("RemoteFileEditor.policy.code").to_string(),
            EditorMode::PlainText => t!("RemoteFileEditor.policy.plain_text").to_string(),
        }
    }
}

impl Render for RemoteFileEditorWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                        .child(self.display_name.clone()),
                ),
            )
            .child(self.render_toolbar(cx))
            .child(v_flex().flex_1().child(self.render_body(window, cx)))
            .child(self.render_status_bar(cx))
    }
}

fn display_name_from_path(path: &str) -> String {
    path.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn format_size(size: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = 1024 * 1024;

    if size >= MIB {
        format!("{:.1} MiB", size as f64 / MIB as f64)
    } else if size >= KIB {
        format!("{:.1} KiB", size as f64 / KIB as f64)
    } else {
        format!("{} B", size)
    }
}
