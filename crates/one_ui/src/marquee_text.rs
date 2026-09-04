use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, ElementId, IntoElement, ParentElement, SharedString,
    Styled, div, px,
};

const MARQUEE_MIN_CHARACTERS: usize = 28;
const APPROXIMATE_CHARACTER_WIDTH: f32 = 7.0;
const MARQUEE_GAP_WIDTH: f32 = 32.0;
const MIN_DURATION_SECS: f32 = 6.0;
const PIXELS_PER_SECOND: f32 = 32.0;

pub fn marquee_text(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    active: bool,
) -> AnyElement {
    let id = id.into();
    let text = text.into();
    let Some((travel, duration)) = active.then(|| marquee_motion(&text)).flatten() else {
        return div()
            .w_full()
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap()
            .child(text)
            .into_any_element();
    };
    let repeated = format!("{text}     {text}");
    div()
        .relative()
        .w_full()
        .overflow_hidden()
        .child(
            div()
                .relative()
                .whitespace_nowrap()
                .child(repeated)
                .with_animation(id, Animation::new(duration).repeat(), move |text, delta| {
                    text.left(px(-travel * delta))
                }),
        )
        .into_any_element()
}

fn marquee_motion(text: &str) -> Option<(f32, Duration)> {
    let characters = text.chars().count();
    if characters <= MARQUEE_MIN_CHARACTERS {
        return None;
    }
    let travel = characters as f32 * APPROXIMATE_CHARACTER_WIDTH + MARQUEE_GAP_WIDTH;
    let duration = Duration::from_secs_f32((travel / PIXELS_PER_SECOND).max(MIN_DURATION_SECS));
    Some((travel, duration))
}

#[cfg(test)]
mod tests {
    use super::{MARQUEE_MIN_CHARACTERS, marquee_motion};

    #[test]
    fn short_labels_stay_static_and_long_labels_scroll() {
        assert!(marquee_motion(&"a".repeat(MARQUEE_MIN_CHARACTERS)).is_none());
        assert!(marquee_motion(&"a".repeat(MARQUEE_MIN_CHARACTERS + 1)).is_some());
    }

    #[test]
    fn longer_labels_receive_longer_scroll_duration() {
        let (_, short_duration) = marquee_motion(&"a".repeat(40)).expect("long label");
        let (_, long_duration) = marquee_motion(&"a".repeat(120)).expect("longer label");
        assert!(long_duration > short_duration);
    }
}
