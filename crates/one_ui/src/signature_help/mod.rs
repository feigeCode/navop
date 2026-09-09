mod lifecycle;
mod overlay;

use std::rc::Rc;

use anyhow::Result;
use gpui::{
    App, Context, DefiniteLength, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, SharedString, StyleRefinement, Styled, Subscription, Task, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::StyledExt as _;
use gpui_component::input::{Editor, EditorState, Escape, GutterMarkerRenderer, InputEvent, Rope};
use lsp_types::SignatureHelp;

use lifecycle::{SignatureHelpLifecycle, cycle_overload, inserted_text, should_refresh_for_edit};
use overlay::SignatureHelpOverlay;

pub trait SignatureHelpProvider: 'static {
    fn signature_help(
        &self,
        text: &Rope,
        offset: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<SignatureHelp>>>;
}

pub struct ExtendedEditorState {
    editor: Entity<EditorState>,
    provider: Option<Rc<dyn SignatureHelpProvider>>,
    lifecycle: SignatureHelpLifecycle,
    last_text: String,
    last_cursor: usize,
    help: Option<Rc<SignatureHelp>>,
    anchor: Option<usize>,
    active_signature: usize,
    request_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl ExtendedEditorState {
    pub fn new(editor: Entity<EditorState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (last_text, last_cursor, document_revision) = {
            let editor = editor.read(cx);
            (
                editor.value().to_string(),
                editor.cursor(),
                editor.document_revision(),
            )
        };
        let observe_editor = editor.clone();
        let observe = cx.observe_in(&editor, window, move |this, _, window, cx| {
            this.sync_editor(&observe_editor, window, cx);
        });
        let events = cx.subscribe_in(&editor, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::Blur) {
                this.close_signature_help(cx);
            }
        });
        let mut lifecycle = SignatureHelpLifecycle::default();
        lifecycle.observe_document(last_cursor, document_revision);
        Self {
            editor,
            provider: None,
            lifecycle,
            last_text,
            last_cursor,
            help: None,
            anchor: None,
            active_signature: 0,
            request_task: Task::ready(()),
            _subscriptions: vec![observe, events],
        }
    }

    pub fn editor(&self) -> &Entity<EditorState> {
        &self.editor
    }

    pub fn set_signature_help_provider(
        &mut self,
        provider: Option<Rc<dyn SignatureHelpProvider>>,
        cx: &mut Context<Self>,
    ) {
        self.provider = provider;
        if self.provider.is_none() {
            self.close_signature_help(cx);
        }
    }

    pub fn document_revision(&self) -> u64 {
        self.lifecycle.document_revision()
    }

    pub fn close_signature_help(&mut self, cx: &mut Context<Self>) {
        self.lifecycle.invalidate();
        self.lifecycle.set_open(false);
        self.help = None;
        self.anchor = None;
        self.active_signature = 0;
        cx.notify();
    }

    pub fn refresh_signature_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(provider) = self.provider.clone() else {
            self.close_signature_help(cx);
            return;
        };
        let (cursor, document_revision, text) = {
            let editor = self.editor.read(cx);
            (
                editor.cursor(),
                editor.document_revision(),
                editor.value().to_string(),
            )
        };
        self.lifecycle.observe_document(cursor, document_revision);
        self.last_cursor = cursor;
        self.last_text = text;
        let request = self.lifecycle.begin_request();
        let offset = self.lifecycle.cursor();
        let text = self.editor.read(cx).text().clone();
        let task = provider.signature_help(&text, offset, window, cx);
        let owner = cx.entity();
        self.request_task = cx.spawn_in(window, async move |_, cx| {
            let result = task.await.ok().flatten();
            let _ = owner.update(cx, |state, cx| state.apply_response(request, result, cx));
        });
    }

    pub fn cycle_signature(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.help.as_ref().map_or(0, |help| help.signatures.len());
        self.active_signature = cycle_overload(len, self.active_signature, delta);
        cx.notify();
    }

    pub fn active_signature(&self) -> usize {
        self.active_signature
    }

    pub fn help(&self) -> Option<&SignatureHelp> {
        self.help.as_deref()
    }

    pub fn anchor(&self) -> Option<usize> {
        self.anchor
    }

    fn sync_editor(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (text, cursor, document_revision) = {
            let editor = editor.read(cx);
            (
                editor.value().to_string(),
                editor.cursor(),
                editor.document_revision(),
            )
        };
        let text_changed = text != self.last_text;
        let cursor_changed = cursor != self.last_cursor;
        if !text_changed && !cursor_changed {
            return;
        }
        let inserted = inserted_text(&self.last_text, &text).to_string();
        self.last_text = text;
        self.last_cursor = cursor;
        self.lifecycle.observe_document(cursor, document_revision);
        let active = self.lifecycle.is_active();
        let refresh = text_changed && should_refresh_for_edit(&inserted, active);
        if refresh || (cursor_changed && active) {
            self.refresh_signature_help(window, cx);
        }
    }

    fn apply_response(
        &mut self,
        request: lifecycle::RequestIdentity,
        result: Option<SignatureHelp>,
        cx: &mut Context<Self>,
    ) {
        if !self.lifecycle.accepts(request) {
            return;
        }
        let Some(help) = result.filter(|help| !help.signatures.is_empty()) else {
            self.close_signature_help(cx);
            return;
        };
        self.active_signature = cycle_overload(
            help.signatures.len(),
            help.active_signature.unwrap_or(0) as usize,
            0,
        );
        self.anchor = Some(self.lifecycle.cursor());
        self.help = Some(Rc::new(help));
        self.lifecycle.set_open(true);
        cx.notify();
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        if self.lifecycle.is_active() {
            self.close_signature_help(cx);
        }
    }
}

#[derive(IntoElement)]
pub struct ExtendedEditor {
    state: Entity<ExtendedEditorState>,
    style: StyleRefinement,
    height: Option<DefiniteLength>,
    appearance: bool,
    bordered: bool,
    disabled: bool,
    readonly: bool,
    tab_index: isize,
    aria_label: Option<SharedString>,
    gutter_marker_renderer: Option<GutterMarkerRenderer>,
}

impl ExtendedEditor {
    pub fn new(state: &Entity<ExtendedEditorState>) -> Self {
        Self {
            state: state.clone(),
            style: StyleRefinement::default(),
            height: None,
            appearance: true,
            bordered: true,
            disabled: false,
            readonly: false,
            tab_index: 0,
            aria_label: None,
            gutter_marker_renderer: None,
        }
    }

    pub fn h(mut self, height: impl Into<DefiniteLength>) -> Self {
        self.height = Some(height.into());
        self
    }

    pub fn appearance(mut self, appearance: bool) -> Self {
        self.appearance = appearance;
        self
    }

    pub fn bordered(mut self, bordered: bool) -> Self {
        self.bordered = bordered;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    pub fn tab_index(mut self, tab_index: isize) -> Self {
        self.tab_index = tab_index;
        self
    }

    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    pub fn gutter_marker_renderer(mut self, renderer: GutterMarkerRenderer) -> Self {
        self.gutter_marker_renderer = Some(renderer);
        self
    }
}

impl Styled for ExtendedEditor {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ExtendedEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let editor_state = self.state.read(cx).editor().clone();
        let open = self.state.read(cx).help().is_some();
        div()
            .relative()
            .on_action(window.listener_for(&self.state, ExtendedEditorState::on_escape))
            .refine_style(&self.style)
            .child(
                Editor::new(&editor_state)
                    .size_full()
                    .appearance(self.appearance)
                    .bordered(self.bordered)
                    .disabled(self.disabled)
                    .readonly(self.readonly)
                    .tab_index(self.tab_index)
                    .when_some(self.gutter_marker_renderer, |editor, renderer| {
                        editor.gutter_marker_renderer(move |marker| renderer(marker))
                    })
                    .when_some(self.height, |editor, height| editor.h(height))
                    .when_some(self.aria_label, |editor, label| editor.aria_label(label)),
            )
            .when(open, |this| {
                this.child(SignatureHelpOverlay::new(self.state))
            })
    }
}
