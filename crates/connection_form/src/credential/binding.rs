use gpui::{Context, Entity, Subscription, Window};
use gpui_component::input::{InputEvent, InputState};

use super::{CredentialField, CredentialReferencePicker};

pub struct ManualCredentialOverride {
    picker: Entity<CredentialReferencePicker>,
    input: Entity<InputState>,
    field: CredentialField,
}

impl ManualCredentialOverride {
    pub fn new(
        picker: &Entity<CredentialReferencePicker>,
        input: &Entity<InputState>,
        field: CredentialField,
    ) -> Self {
        Self {
            picker: picker.clone(),
            input: input.clone(),
            field,
        }
    }

    pub fn subscribe<T: 'static>(self, window: &mut Window, cx: &mut Context<T>) -> Subscription {
        cx.subscribe_in(
            &self.input,
            window,
            move |_, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Focus) {
                    self.picker.update(cx, |picker, cx| {
                        picker.use_manual_field(self.field, window, cx);
                    });
                }
            },
        )
    }
}
