use anyhow::{Context as _, Result};

pub(super) fn component_registry() -> Result<gpui_shell::FrozenComponentRegistry> {
    gpui_component_shell::components().context("build gpui-component shell registry")
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui::{ParentElement as _, Styled as _};
    use gpui_shell::{ShellRuntime, ViewLoadOptions, policy::Policy};

    use super::*;

    struct TestHost(gpui::Entity<gpui_shell::ScriptView>);

    impl gpui::Render for TestHost {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div().size_full().child(self.0.clone())
        }
    }

    #[test]
    fn navop_shell_runtime_uses_the_component_adapter_catalog() {
        let registry = component_registry().expect("component registry");
        assert_eq!(Some("gpui-component"), registry.module_specifier());
        assert!(registry.descriptors().any(|item| item.name() == "Button"));
    }

    #[gpui::test]
    fn navop_shell_view_materializes_a_gpui_component(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            gpui_shell::init_embedded(cx);
        });
        let runtime = ShellRuntime::new_isolated_with_components(
            component_registry().expect("component registry"),
        )
        .expect("shell runtime");
        let root = tempfile::TempDir::new().expect("temporary extension");
        std::fs::write(
            root.path().join("main.js"),
            r#"
                import { View, div } from "gpui";
                import { Button } from "gpui-component";
                export default class Probe extends View {
                  render() {
                    return div().child(new Button("probe").label("Component button"));
                  }
                }
            "#,
        )
        .expect("script source");
        let mounted = Rc::new(RefCell::new(None));
        let mounted_for_window = Rc::clone(&mounted);
        let script_root = root.path().to_path_buf();
        let window = cx.add_window(move |window, cx| {
            let loaded = runtime
                .load_view(
                    ViewLoadOptions::new(script_root, "main.js", Rc::new(Policy::new())),
                    window,
                    cx,
                )
                .expect("load component script");
            let view = loaded.view().clone();
            *mounted_for_window.borrow_mut() = Some(loaded);
            TestHost(view)
        });
        let mut visual = gpui::VisualTestContext::from_window(*window, cx);

        visual.update(|window, cx| window.draw(cx).clear(cx));
        let view = mounted.borrow().as_ref().unwrap().view().clone();
        let tree = visual.update(|_, cx| view.read(cx).snapshot().unwrap().debug_tree());
        assert!(tree.contains("Button :label(registered)"), "{tree}");
        visual.update(|_, cx| mounted.borrow_mut().as_mut().unwrap().unload(cx));
    }
}
