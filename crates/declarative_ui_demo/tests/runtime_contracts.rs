use std::collections::BTreeSet;

use declarative_ui_demo::{
    ActionEvent, ActionOutcome, NodePath, Runtime, RuntimeError, RuntimeEvent, StateChange,
    StateChangeOrigin, StateStore,
};
use gpui::{AppContext, Context, Subscription, TestAppContext};

struct RuntimeProbe {
    events: Vec<RuntimeEvent>,
    _subscription: Subscription,
}

impl RuntimeProbe {
    fn new(runtime: &gpui::Entity<Runtime>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(runtime, |probe, _, event: &RuntimeEvent, _| {
            probe.events.push(event.clone());
        });
        Self {
            events: Vec::new(),
            _subscription: subscription,
        }
    }
}

#[gpui::test]
fn duplicate_actions_are_rejected_without_replacing_the_original_handler(cx: &mut TestAppContext) {
    let runtime = cx.update(|cx| {
        cx.new(|_| {
            let mut runtime = Runtime::default();
            runtime
                .on("save", |context| {
                    context.set("handler", "first");
                    Ok(())
                })
                .expect("first registration succeeds");
            runtime
        })
    });
    let error = cx
        .update(|cx| {
            runtime.update(cx, |runtime, _| {
                runtime.on("save", |context| {
                    context.set("handler", "second");
                    Ok(())
                })
            })
        })
        .expect_err("duplicate registration must be rejected");

    let event = ActionEvent::new("save", "button:save", NodePath::root());
    cx.update(|cx| {
        runtime
            .update(cx, |runtime, cx| runtime.dispatch(event, cx))
            .expect("original handler remains registered");
    });

    assert_eq!(RuntimeError::DuplicateAction("save".to_owned()), error);
    assert_eq!(
        Some("first".to_owned()),
        runtime.read_with(cx, |runtime, _| {
            runtime.get("handler").map(str::to_owned)
        })
    );
    assert_eq!(1, runtime.read_with(cx, |runtime, _| runtime.revision()));
}

#[gpui::test]
fn panicking_actions_roll_back_and_emit_only_action_failed(cx: &mut TestAppContext) {
    let (runtime, probe) = cx.update(|cx| {
        let mut state = StateStore::default();
        state.set("status", "idle");
        let runtime = cx.new(|_| {
            let mut runtime = Runtime::new(state);
            runtime
                .on("save", |context| {
                    context.set("status", "half-written");
                    panic!("handler exploded");
                })
                .expect("register action");
            runtime
        });
        let probe = cx.new(|cx| RuntimeProbe::new(&runtime, cx));
        (runtime, probe)
    });
    cx.run_until_parked();

    let event = ActionEvent::new("save", "button:save", NodePath(vec![2]));
    let result =
        cx.update(|cx| runtime.update(cx, |runtime, cx| runtime.dispatch(event.clone(), cx)));
    cx.run_until_parked();

    assert_eq!(
        Err(RuntimeError::HandlerPanicked {
            action: "save".to_owned(),
            message: "handler exploded".to_owned(),
        }),
        result
    );
    cx.update(|cx| {
        let runtime = runtime.read(cx);
        assert_eq!(0, runtime.revision());
        assert_eq!(Some("idle"), runtime.get("status"));
        assert_eq!(
            vec![RuntimeEvent::ActionFailed {
                event,
                error: RuntimeError::HandlerPanicked {
                    action: "save".to_owned(),
                    message: "handler exploded".to_owned(),
                },
            }],
            probe.read(cx).events
        );
    });
}

#[gpui::test]
fn successful_actions_commit_once_and_emit_ordered_events(cx: &mut TestAppContext) {
    let (runtime, probe) = observed_runtime(cx, |context| {
        context.set("status", "saving");
        context.set("status", "saved");
        context.set("count", "1");
        Ok(())
    });
    let event = ActionEvent::new("save", "button:save", NodePath::root());

    let outcome = cx
        .update(|cx| runtime.update(cx, |runtime, cx| runtime.dispatch(event.clone(), cx)))
        .expect("action succeeds");
    cx.run_until_parked();

    assert_eq!(
        ActionOutcome {
            state_changed: true,
            revision: 1,
        },
        outcome
    );
    cx.update(|cx| {
        assert_eq!(Some("saved"), runtime.read(cx).get("status"));
        assert_eq!(
            vec![
                RuntimeEvent::StateChanged(StateChange {
                    revision: 1,
                    changed_keys: BTreeSet::from(["count".to_owned(), "status".to_owned()]),
                    origin: StateChangeOrigin::Action {
                        name: "save".to_owned(),
                        source_id: "button:save".to_owned(),
                    },
                }),
                RuntimeEvent::ActionCompleted {
                    event,
                    outcome: ActionOutcome {
                        state_changed: true,
                        revision: 1,
                    },
                },
            ],
            probe.read(cx).events
        );
    });
}

#[gpui::test]
fn no_op_actions_complete_without_state_events_or_revision_changes(cx: &mut TestAppContext) {
    let (runtime, probe) = observed_runtime(cx, |_context| Ok(()));
    let event = ActionEvent::new("save", "button:save", NodePath::root());

    let outcome = cx
        .update(|cx| runtime.update(cx, |runtime, cx| runtime.dispatch(event.clone(), cx)))
        .expect("no-op action succeeds");
    cx.run_until_parked();

    assert_eq!(
        ActionOutcome {
            state_changed: false,
            revision: 0,
        },
        outcome
    );
    cx.update(|cx| {
        assert_eq!(0, runtime.read(cx).revision());
        assert_eq!(
            vec![RuntimeEvent::ActionCompleted { event, outcome }],
            probe.read(cx).events
        );
    });
}

fn observed_runtime(
    cx: &mut TestAppContext,
    handler: impl Fn(
        &mut declarative_ui_demo::ActionContext<'_>,
    ) -> Result<(), declarative_ui_demo::ActionError>
    + 'static,
) -> (gpui::Entity<Runtime>, gpui::Entity<RuntimeProbe>) {
    let result = cx.update(|cx| {
        let mut initial = StateStore::default();
        initial.set("status", "idle");
        initial.set("count", "0");
        let runtime = cx.new(|_| {
            let mut runtime = Runtime::new(initial);
            runtime.on("save", handler).expect("register action");
            runtime
        });
        let probe = cx.new(|cx| RuntimeProbe::new(&runtime, cx));
        (runtime, probe)
    });
    cx.run_until_parked();
    result
}
