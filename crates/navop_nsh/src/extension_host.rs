use gpui::{AnyElement, App, Entity, IntoElement, ParentElement, Styled, Window, div};
use nsh_platform_gpui::{ComponentContext, NativeComponent};
use serde_json::Value;
use universal_plugins::{NshHost, NshTab};

/// A GPUI bridge that embeds an installed extension's nsh app inside the
/// nsh-native shell.
pub struct ExtensionAppComponent {
    context: ComponentContext,
    extension_id: Option<String>,
    route: Option<String>,
    app: Option<Entity<NshTab>>,
    error: Option<String>,
}

impl ExtensionAppComponent {
    pub fn new(context: ComponentContext) -> Self {
        Self {
            context,
            extension_id: None,
            route: None,
            app: None,
            error: None,
        }
    }

    fn ensure_started(&mut self, cx: &mut App) {
        if self.app.is_some() || self.error.is_some() {
            return;
        }
        let Some(extension_id) = self.extension_id.as_deref() else {
            return;
        };
        let Some(host) = cx.try_global::<NshHost>().cloned() else {
            self.error = Some("The extension nsh host is not initialized".to_owned());
            return;
        };
        match host.open_embedded(extension_id, self.route.as_deref(), cx) {
            Some(app) => self.app = Some(app),
            None => {
                self.error = Some(format!(
                    "Extension {extension_id} is unavailable or its nsh app is missing"
                ))
            }
        }
        self.context.redraw();
    }
}

impl NativeComponent for ExtensionAppComponent {
    fn render(&mut self, _window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.ensure_started(cx);
        if let Some(app) = &self.app {
            return Some(app.clone().into_any_element());
        }
        let message = self
            .error
            .clone()
            .unwrap_or_else(|| "Select an extension to open".to_owned());
        Some(div().size_full().p_4().child(message).into_any_element())
    }

    fn set_prop(&mut self, name: &str, value: Option<&Value>, _window: &mut Window, _cx: &mut App) {
        match name {
            "extension_id" => {
                let next = value.and_then(Value::as_str).map(str::to_owned);
                if self.extension_id != next {
                    self.extension_id = next;
                    self.app = None;
                    self.error = None;
                    self.context.redraw();
                }
            }
            "route" => {
                let next = value.and_then(Value::as_str).map(str::to_owned);
                if self.route != next {
                    self.route = next;
                    self.app = None;
                    self.error = None;
                    self.context.redraw();
                }
            }
            _ => {}
        }
    }
}
