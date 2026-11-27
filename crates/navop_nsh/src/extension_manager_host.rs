use std::sync::Arc;

use extension_runtime::MainExtensionViewHost;
use extension_view::ExtensionManagerView;
use gpui::{AnyElement, App, AppContext as _, Entity, FocusHandle, Focusable as _};
use gpui::{IntoElement, ParentElement, Styled, Window, div};
use nsh_platform_gpui::{ComponentContext, NativeComponent};
use serde_json::Value;

/// Embeds the existing extension manager UI for install, update, reload and
/// removal. Extension packaging remains owned by `extension_runtime`; the nsh
/// shell only hosts the manager surface.
pub struct ExtensionManagerHost {
    context: ComponentContext,
    view: Option<Entity<ExtensionManagerView>>,
    focus: Option<FocusHandle>,
    error: Option<String>,
}

impl ExtensionManagerHost {
    pub fn new(context: ComponentContext) -> Self {
        Self {
            context,
            view: None,
            focus: None,
            error: None,
        }
    }

    fn ensure_started(&mut self, window: &mut Window, cx: &mut App) {
        if self.view.is_some() || self.error.is_some() {
            return;
        }
        let host = Arc::new(MainExtensionViewHost);
        let view = cx.new(|cx| ExtensionManagerView::new(host, window, cx));
        self.focus = Some(view.read(cx).focus_handle(cx));
        self.view = Some(view);
        self.context.redraw();
    }
}

impl NativeComponent for ExtensionManagerHost {
    fn render(&mut self, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.ensure_started(window, cx);
        if let Some(view) = &self.view {
            return Some(view.clone().into_any_element());
        }
        Some(
            div()
                .size_full()
                .p_4()
                .child(
                    self.error
                        .clone()
                        .unwrap_or_else(|| "Extension manager is starting…".to_owned()),
                )
                .into_any_element(),
        )
    }

    fn set_prop(
        &mut self,
        _name: &str,
        _value: Option<&Value>,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn focus_handle(&self) -> Option<FocusHandle> {
        self.focus.clone()
    }
}
