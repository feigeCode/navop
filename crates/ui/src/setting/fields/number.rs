use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Entity, IntoElement, SharedString, StyleRefinement, Styled,
    Subscription, Window, prelude::FluentBuilder as _,
};

use crate::{
    AxisExt, Sizable, StyledExt,
    input::{InputEvent, InputState, NumberInput, NumberInputEvent, StepAction},
    setting::{
        AnySettingField, RenderOptions,
        fields::{SettingFieldRender, get_value, set_value},
    },
};

#[derive(Clone, Debug)]
pub struct NumberFieldOptions {
    /// The minimum value for the number input, default is `f64::MIN`.
    pub min: f64,
    /// The maximum value for the number input, default is `f64::MAX`.
    pub max: f64,
    /// The step value for the number input, default is `1.0`.
    pub step: f64,
}

impl Default for NumberFieldOptions {
    fn default() -> Self {
        Self {
            min: f64::MIN,
            max: f64::MAX,
            step: 1.0,
        }
    }
}

pub(crate) struct NumberField {
    options: NumberFieldOptions,
}

impl NumberField {
    pub(crate) fn new(options: Option<&NumberFieldOptions>) -> Self {
        Self {
            options: options.cloned().unwrap_or_default(),
        }
    }
}

fn update_number_from_step(
    value: &str,
    action: StepAction,
    options: &NumberFieldOptions,
    mut set_value: impl FnMut(f64),
) -> Option<f64> {
    let value = value.parse::<f64>().ok()?;
    let new_value = match action {
        StepAction::Increment => value + options.step,
        StepAction::Decrement => value - options.step,
    }
    .clamp(options.min, options.max);
    set_value(new_value);
    Some(new_value)
}

struct State {
    input: Entity<InputState>,
    initial_value: f64,
    _subscriptions: Vec<Subscription>,
}

impl SettingFieldRender for NumberField {
    fn render(
        &self,
        field: Rc<dyn AnySettingField>,
        options: &RenderOptions,
        style: &StyleRefinement,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let value = get_value::<f64>(&field, cx);
        let set_value = set_value::<f64>(&field, cx);
        let num_options = self.options.clone();

        let state = window
            .use_keyed_state(
                SharedString::from(format!(
                    "number-state-{}-{}-{}",
                    options.page_ix, options.group_ix, options.item_ix
                )),
                cx,
                |window, cx| {
                    let input =
                        cx.new(|cx| InputState::new(window, cx).default_value(value.to_string()));
                    let step_options = num_options.clone();
                    let change_options = num_options.clone();
                    let _subscriptions = vec![
                        cx.subscribe_in(&input, window, {
                            let set_value = set_value.clone();
                            move |state: &mut State, input, event: &NumberInputEvent, window, cx| {
                                match event {
                                    NumberInputEvent::Step(action) => {
                                        input.update(cx, |input, cx| {
                                            let value = input.value();
                                            if let Some(new_value) = update_number_from_step(
                                                &value,
                                                *action,
                                                &step_options,
                                                |value| set_value(value, cx),
                                            ) {
                                                state.initial_value = new_value;
                                                input.set_value(
                                                    SharedString::from(new_value.to_string()),
                                                    window,
                                                    cx,
                                                );
                                            }
                                        })
                                    }
                                }
                            }
                        }),
                        cx.subscribe_in(&input, window, {
                            move |state: &mut State, input, event: &InputEvent, window, cx| {
                                match event {
                                    InputEvent::Change => {
                                        input.update(cx, |input, cx| {
                                            let value = input.value();
                                            if value == state.initial_value.to_string() {
                                                return;
                                            }

                                            if let Ok(value) = value.parse::<f64>() {
                                                let clamp_value = value
                                                    .clamp(change_options.min, change_options.max);

                                                set_value(clamp_value, cx);
                                                state.initial_value = clamp_value;
                                                if clamp_value != value {
                                                    input.set_value(
                                                        SharedString::from(clamp_value.to_string()),
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            }
                                        });
                                    }
                                    _ => {}
                                }
                            }
                        }),
                    ];

                    State {
                        input,
                        initial_value: value,
                        _subscriptions,
                    }
                },
            )
            .read(cx);

        NumberInput::new(&state.input)
            .with_size(options.size)
            .map(|this| {
                if options.layout.is_horizontal() {
                    this.w_32()
                } else {
                    this.w_full()
                }
            })
            .refine_style(style)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{NumberFieldOptions, update_number_from_step};
    use crate::input::StepAction;

    #[test]
    fn number_step_updates_persisted_value() {
        let options = NumberFieldOptions {
            min: 8.0,
            max: 72.0,
            step: 1.0,
        };
        let mut persisted = None;

        let next = update_number_from_step("14", StepAction::Increment, &options, |value| {
            persisted = Some(value);
        });

        assert_eq!(Some(15.0), next);
        assert_eq!(Some(15.0), persisted);
    }

    #[test]
    fn number_step_clamps_before_persisting() {
        let options = NumberFieldOptions {
            min: 8.0,
            max: 72.0,
            step: 1.0,
        };
        let mut persisted = None;

        let next = update_number_from_step("72", StepAction::Increment, &options, |value| {
            persisted = Some(value);
        });

        assert_eq!(Some(72.0), next);
        assert_eq!(Some(72.0), persisted);
    }
}
