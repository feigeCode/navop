use extension_component::ViewWindowOptions;
use gpui::SharedString;
use one_core::popup_window::PopupWindowOptions;

const DEFAULT_EXTENSION_WINDOW_WIDTH: f32 = 720.0;
const DEFAULT_EXTENSION_WINDOW_HEIGHT: f32 = 640.0;
const DEFAULT_EXTENSION_WINDOW_MIN_WIDTH: f32 = 480.0;
const DEFAULT_EXTENSION_WINDOW_MIN_HEIGHT: f32 = 360.0;

pub fn popup_options_for_view(
    title: impl Into<SharedString>,
    window: Option<&ViewWindowOptions>,
) -> PopupWindowOptions {
    let mut options = PopupWindowOptions::new(title)
        .size(
            DEFAULT_EXTENSION_WINDOW_WIDTH,
            DEFAULT_EXTENSION_WINDOW_HEIGHT,
        )
        .min_width(DEFAULT_EXTENSION_WINDOW_MIN_WIDTH)
        .min_height(DEFAULT_EXTENSION_WINDOW_MIN_HEIGHT);
    let Some(window) = window else {
        return options;
    };
    if let Some(width) = positive_size(window.width) {
        options = options.width(width);
    }
    if let Some(height) = positive_size(window.height) {
        options = options.height(height);
    }
    if let Some(min_width) = positive_size(window.min_width) {
        options = options.min_width(min_width);
    }
    if let Some(min_height) = positive_size(window.min_height) {
        options = options.min_height(min_height);
    }
    options
}

fn positive_size(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_options_use_view_window_size_when_present() {
        let options = popup_options_for_view(
            "Sized",
            Some(&ViewWindowOptions {
                width: Some(800.0),
                height: Some(700.0),
                min_width: Some(500.0),
                min_height: Some(420.0),
            }),
        );

        assert_eq!(800.0, options.width);
        assert_eq!(700.0, options.height);
        assert_eq!(500.0, options.min_width);
        assert_eq!(420.0, options.min_height);
    }

    #[test]
    fn popup_options_ignore_invalid_view_window_size() {
        let options = popup_options_for_view(
            "Sized",
            Some(&ViewWindowOptions {
                width: Some(0.0),
                height: Some(f32::NAN),
                min_width: Some(-1.0),
                min_height: Some(f32::INFINITY),
            }),
        );

        assert_eq!(DEFAULT_EXTENSION_WINDOW_WIDTH, options.width);
        assert_eq!(DEFAULT_EXTENSION_WINDOW_HEIGHT, options.height);
        assert_eq!(DEFAULT_EXTENSION_WINDOW_MIN_WIDTH, options.min_width);
        assert_eq!(DEFAULT_EXTENSION_WINDOW_MIN_HEIGHT, options.min_height);
    }
}
