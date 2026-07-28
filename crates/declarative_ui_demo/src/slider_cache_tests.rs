use std::rc::Rc;

use gpui::{
    AnyWindowHandle, App, AppContext, Context, Entity, IntoElement, Render, TestAppContext,
    VisualTestContext, Window, WindowOptions, div,
};
use gpui_component::slider::{SliderEvent, SliderScale, SliderState, SliderValue};

use crate::{
    ActionEvent, NodePath, Runtime, StateStore,
    render_context::ActionDispatcher,
    slider_cache::{SliderCache, SliderCallbacks, SliderConfig, SliderEnvironment, SliderRequest},
};

const SLIDER_MIN: f32 = 0.0;
const SLIDER_MAX: f32 = 100.0;
const SLIDER_STEP: f32 = 1.0;

struct SliderHarness {
    cache: SliderCache,
    runtime: Entity<Runtime>,
}

impl SliderHarness {
    fn new(runtime: Entity<Runtime>) -> Self {
        Self {
            cache: SliderCache::default(),
            runtime,
        }
    }

    fn resolve(
        &mut self,
        case: SliderCase,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<SliderState> {
        let dispatcher_runtime = self.runtime.clone();
        let dispatcher: ActionDispatcher = Rc::new(move |event, cx| {
            let _ = dispatcher_runtime.update(cx, |runtime, cx| runtime.dispatch(event, cx));
        });
        let request = SliderRequest::new(
            case.id,
            case.config,
            SliderCallbacks::new(case.binding, case.action),
        );
        self.cache.resolve(
            request,
            SliderEnvironment {
                runtime: self.runtime.clone(),
                dispatcher,
                window,
                cx,
            },
        )
    }

    fn retain(&mut self, source: &str) {
        let root = crate::parse_html(source).expect("valid live tree");
        self.cache.retain_live(&root);
    }
}

impl Render for SliderHarness {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

struct SliderCase {
    id: String,
    config: SliderConfig,
    binding: Option<String>,
    action: Option<ActionEvent>,
}

impl SliderCase {
    fn bound(value: f32) -> Self {
        Self {
            id: "slider:volume".to_owned(),
            config: slider_config(value),
            binding: Some("volume".to_owned()),
            action: None,
        }
    }

    fn with_release_action(mut self) -> Self {
        self.action = Some(ActionEvent::new(
            "observe-release",
            self.id.clone(),
            NodePath::root(),
        ));
        self
    }
}

#[gpui::test]
fn slider_change_writes_binding_and_release_dispatches_action_afterward(cx: &mut TestAppContext) {
    let (window, runtime, harness) = mount_harness(cx);
    let mut visual = VisualTestContext::from_window(window, cx);
    let slider = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(SliderCase::bound(20.0).with_release_action(), window, cx)
    });

    slider.update_in(&mut visual, |slider, window, cx| {
        slider.set_value(42.0, window, cx);
        cx.emit(SliderEvent::Change(SliderValue::Single(42.0)));
    });
    visual.run_until_parked();
    assert_eq!(
        (Some("42".to_owned()), None, 1),
        runtime_snapshot(&runtime, &visual)
    );

    slider.update_in(&mut visual, |_slider, _window, cx| {
        cx.emit(SliderEvent::Release(SliderValue::Single(42.0)));
    });
    visual.run_until_parked();
    assert_eq!(
        (Some("42".to_owned()), Some("42".to_owned()), 2),
        runtime_snapshot(&runtime, &visual)
    );
}

#[gpui::test]
fn slider_writeback_clamps_native_step_rounding_to_the_declared_range(cx: &mut TestAppContext) {
    let (window, runtime, harness) = mount_harness(cx);
    let mut visual = VisualTestContext::from_window(window, cx);
    let mut case = SliderCase::bound(0.0).with_release_action();
    case.config.max = 1.0;
    case.config.step = 2.0;
    let slider = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(case, window, cx)
    });

    slider.update_in(&mut visual, |_slider, _window, cx| {
        cx.emit(SliderEvent::Change(SliderValue::Single(2.0)));
    });
    visual.run_until_parked();
    assert_eq!(
        (Some("1".to_owned()), None, 1),
        runtime_snapshot(&runtime, &visual)
    );

    slider.update_in(&mut visual, |_slider, _window, cx| {
        cx.emit(SliderEvent::Release(SliderValue::Single(2.0)));
    });
    visual.run_until_parked();
    assert_eq!(
        (Some("1".to_owned()), Some("1".to_owned()), 2),
        runtime_snapshot(&runtime, &visual)
    );
}

#[gpui::test]
fn external_bound_value_sync_reuses_slider_entity_without_emitting_events(cx: &mut TestAppContext) {
    let (window, runtime, harness) = mount_harness(cx);
    let mut visual = VisualTestContext::from_window(window, cx);
    let first = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(SliderCase::bound(20.0), window, cx)
    });

    runtime.update(&mut visual, |runtime, cx| {
        runtime.set("volume", "55", cx);
    });
    let second = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(SliderCase::bound(55.0), window, cx)
    });
    visual.run_until_parked();

    assert_eq!(first.entity_id(), second.entity_id());
    assert_eq!(
        SliderValue::Single(55.0),
        second.read_with(&visual, |slider, _| slider.value())
    );
    assert_eq!(
        (Some("55".to_owned()), None, 1),
        runtime_snapshot(&runtime, &visual)
    );
}

#[gpui::test]
fn removed_slider_drops_its_state_writeback_subscription(cx: &mut TestAppContext) {
    let (window, runtime, harness) = mount_harness(cx);
    let mut visual = VisualTestContext::from_window(window, cx);
    let slider = harness.update_in(&mut visual, |harness, window, cx| {
        harness.resolve(SliderCase::bound(20.0), window, cx)
    });
    harness.update(&mut visual, |harness, _| harness.retain("<div></div>"));

    slider.update_in(&mut visual, |_slider, _window, cx| {
        cx.emit(SliderEvent::Change(SliderValue::Single(75.0)));
    });
    visual.run_until_parked();

    assert_eq!(
        (Some("20".to_owned()), None, 0),
        runtime_snapshot(&runtime, &visual)
    );
}

fn slider_config(value: f32) -> SliderConfig {
    SliderConfig {
        min: SLIDER_MIN,
        max: SLIDER_MAX,
        step: SLIDER_STEP,
        value,
        scale: SliderScale::Linear,
    }
}

fn runtime_snapshot(
    runtime: &Entity<Runtime>,
    visual: &VisualTestContext,
) -> (Option<String>, Option<String>, u64) {
    runtime.read_with(visual, |runtime, _| {
        (
            runtime.get("volume").map(str::to_owned),
            runtime.get("observed").map(str::to_owned),
            runtime.revision(),
        )
    })
}

fn mount_harness(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Runtime>, Entity<SliderHarness>) {
    cx.update(gpui_component::init);
    cx.update(|cx| {
        let mut state = StateStore::default();
        state.set("volume", "20");
        let runtime = cx.new(|_| {
            let mut runtime = Runtime::new(state);
            runtime
                .on("observe-release", |context| {
                    let value = context.get("volume").unwrap_or_default().to_owned();
                    context.set("observed", value);
                    Ok(())
                })
                .expect("unique test action");
            runtime
        });
        let harness = cx.new(|_| SliderHarness::new(runtime.clone()));
        let window = cx
            .open_window(WindowOptions::default(), {
                let harness = harness.clone();
                move |_, _| harness
            })
            .expect("slider test window opens");
        (window.into(), runtime, harness)
    })
}
