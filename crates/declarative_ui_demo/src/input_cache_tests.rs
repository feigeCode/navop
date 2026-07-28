use gpui::{
    AnyWindowHandle, App, AppContext, Context, Entity, IntoElement, Render, TestAppContext,
    VisualTestContext, Window, WindowOptions, div,
};
use gpui_component::input::{InputEvent, InputState};

use crate::{
    ComponentProps, NodePath, Runtime, StateStore,
    input_cache::{InputCache, InputEnvironment, InputRequest},
    parse_html, resolve_bindings,
};

struct InputHarness {
    cache: InputCache,
    runtime: Entity<Runtime>,
}

impl InputHarness {
    fn new(runtime: Entity<Runtime>) -> Self {
        Self {
            cache: InputCache::default(),
            runtime,
        }
    }

    fn resolve(
        &mut self,
        case: InputCase<'_>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<InputState> {
        let root = parse_html(case.source).expect("valid input declaration");
        let resolved = resolve_bindings(&root, self.runtime.read(cx).state());
        let element = resolved.element().expect("standalone input").clone();
        let multiline = element.tag == "textarea";
        let props = ComponentProps::new(element, case.path);
        self.cache.resolve(
            InputRequest::new(&props, multiline, self.runtime.clone()),
            InputEnvironment { window, cx },
        )
    }

    fn retain(&mut self, source: &str) {
        let root = parse_html(source).expect("valid live tree");
        self.cache.retain_live(&root);
    }
}

impl Render for InputHarness {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone)]
struct InputCase<'a> {
    source: &'a str,
    path: NodePath,
}

impl<'a> InputCase<'a> {
    fn root(source: &'a str) -> Self {
        Self {
            source,
            path: NodePath::root(),
        }
    }

    fn at(source: &'a str, index: usize) -> Self {
        Self {
            source,
            path: NodePath(vec![index]),
        }
    }
}

#[gpui::test]
fn user_changes_are_written_back_to_runtime_state(cx: &mut TestAppContext) {
    let (window, runtime, harness) = mount_harness(cx);
    let mut visual = VisualTestContext::from_window(window, cx);
    let input = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(
            InputCase::root(r#"<input key="username" bind="username" />"#),
            window,
            cx,
        )
    });
    visual.run_until_parked();

    input.update_in(&mut visual, |input, window, cx| {
        input.set_value("typed by user", window, cx);
        cx.emit(InputEvent::Change);
    });
    visual.run_until_parked();

    assert_eq!(
        (Some("typed by user".to_owned()), 1),
        runtime.read_with(&visual, |runtime, _| {
            (
                runtime.get("username").map(str::to_owned),
                runtime.revision(),
            )
        })
    );
}

#[gpui::test]
fn state_sync_reuses_the_entity_without_creating_a_binding_loop(cx: &mut TestAppContext) {
    let (window, runtime, harness) = mount_harness(cx);
    let mut visual = VisualTestContext::from_window(window, cx);
    let declaration =
        InputCase::root(r#"<input key="username" bind="username" placeholder="User" />"#);
    let first = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(declaration.clone(), window, cx)
    });
    visual.run_until_parked();

    runtime.update(&mut visual, |runtime, cx| {
        runtime.set("username", "operator", cx);
    });
    let second = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(declaration, window, cx)
    });
    visual.run_until_parked();

    assert_eq!(first.entity_id(), second.entity_id());
    assert_eq!(
        second.read_with(&visual, |input, _| input.value()),
        "operator"
    );
    assert_eq!(
        1,
        runtime.read_with(&visual, |runtime, _| runtime.revision())
    );
}

#[gpui::test]
fn keyed_inputs_keep_identity_when_their_paths_are_reordered(cx: &mut TestAppContext) {
    let (window, _runtime, harness) = mount_harness(cx);
    let mut visual = VisualTestContext::from_window(window, cx);
    let (first_a, first_b) = harness.update_in(&mut visual, |harness, window, cx| {
        (
            harness.resolve(
                InputCase::at(r#"<input key="a" value="A" />"#, 0),
                window,
                cx,
            ),
            harness.resolve(
                InputCase::at(r#"<input key="b" value="B" />"#, 1),
                window,
                cx,
            ),
        )
    });

    let (second_b, second_a) = harness.update_in(&mut visual, |harness, window, cx| {
        (
            harness.resolve(
                InputCase::at(r#"<input key="b" value="B" />"#, 0),
                window,
                cx,
            ),
            harness.resolve(
                InputCase::at(r#"<input key="a" value="A" />"#, 1),
                window,
                cx,
            ),
        )
    });

    assert_eq!(first_a.entity_id(), second_a.entity_id());
    assert_eq!(first_b.entity_id(), second_b.entity_id());
}

#[gpui::test]
fn password_mode_is_part_of_the_cached_input_configuration(cx: &mut TestAppContext) {
    let (window, _runtime, harness) = mount_harness(cx);
    let mut visual = VisualTestContext::from_window(window, cx);
    let text = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(
            InputCase::root(r#"<input key="credential" type="text" value="secret" />"#),
            window,
            cx,
        )
    });
    let password = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(
            InputCase::root(r#"<input key="credential" type="password" value="secret" />"#),
            window,
            cx,
        )
    });
    let password_again = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(
            InputCase::root(r#"<input key="credential" type="PASSWORD" value="secret" />"#),
            window,
            cx,
        )
    });

    assert_ne!(text.entity_id(), password.entity_id());
    assert_eq!(password.entity_id(), password_again.entity_id());
}

#[gpui::test]
fn configuration_changes_replace_the_entity_and_drop_the_old_subscription(cx: &mut TestAppContext) {
    let (window, runtime, harness) = mount_harness(cx);
    let mut visual = VisualTestContext::from_window(window, cx);
    let first = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(
            InputCase::root(r#"<input key="field" bind="username" placeholder="First" />"#),
            window,
            cx,
        )
    });
    let second = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(
            InputCase::root(r#"<input key="field" bind="username" placeholder="Second" />"#),
            window,
            cx,
        )
    });
    visual.run_until_parked();

    assert_ne!(first.entity_id(), second.entity_id());
    first.update_in(&mut visual, |input, window, cx| {
        input.set_value("stale edit", window, cx);
        cx.emit(InputEvent::Change);
    });
    visual.run_until_parked();

    assert_eq!(
        (Some("admin".to_owned()), 0),
        runtime.read_with(&visual, |runtime, _| {
            (
                runtime.get("username").map(str::to_owned),
                runtime.revision(),
            )
        })
    );
}

#[gpui::test]
fn removed_inputs_drop_their_state_writeback_subscription(cx: &mut TestAppContext) {
    let (window, runtime, harness) = mount_harness(cx);
    let mut visual = VisualTestContext::from_window(window, cx);
    let input = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(
            InputCase::root(r#"<input key="username" bind="username" />"#),
            window,
            cx,
        )
    });
    visual.run_until_parked();
    harness.update(&mut visual, |harness, _| harness.retain("<div></div>"));

    input.update_in(&mut visual, |input, window, cx| {
        input.set_value("stale edit", window, cx);
        cx.emit(InputEvent::Change);
    });
    visual.run_until_parked();

    assert_eq!(
        (Some("admin".to_owned()), 0),
        runtime.read_with(&visual, |runtime, _| {
            (
                runtime.get("username").map(str::to_owned),
                runtime.revision(),
            )
        })
    );
}

fn mount_harness(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Runtime>, Entity<InputHarness>) {
    cx.update(gpui_component::init);
    cx.update(|cx| {
        let mut state = StateStore::default();
        state.set("username", "admin");
        let runtime = cx.new(|_| Runtime::new(state));
        let harness = cx.new(|_| InputHarness::new(runtime.clone()));
        let window = cx
            .open_window(WindowOptions::default(), {
                let harness = harness.clone();
                move |_, _| harness
            })
            .expect("input test window opens");
        (window.into(), runtime, harness)
    })
}
