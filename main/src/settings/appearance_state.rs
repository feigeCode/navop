use gpui::{AppContext, Context, Entity, Window};
use gpui_component::{
    Colorize,
    color_picker::{ColorPickerEvent, ColorPickerState},
    slider::{SliderEvent, SliderState},
    try_parse_color,
};
use one_core::{settings::AppSettings, themes};

const DEFAULT_ACCENT_HUE: f32 = 0.61;
const DEFAULT_ACCENT_SATURATION: f32 = 0.72;
const DEFAULT_ACCENT_LIGHTNESS: f32 = 0.52;

pub struct AppearanceSettingsState {
    pub opacity_slider: Entity<SliderState>,
    pub accent_picker: Entity<ColorPickerState>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl AppearanceSettingsState {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = AppSettings::current(cx);
        let opacity_slider = cx.new(|_| {
            SliderState::new()
                .min(AppSettings::MIN_WINDOW_OPACITY * 100.0)
                .max(100.0)
                .step(1.0)
                .default_value(settings.window_opacity * 100.0)
        });
        let accent = try_parse_color(&settings.custom_accent_color).unwrap_or_else(|_| {
            gpui::hsla(
                DEFAULT_ACCENT_HUE,
                DEFAULT_ACCENT_SATURATION,
                DEFAULT_ACCENT_LIGHTNESS,
                1.0,
            )
        });
        let accent_picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(accent));
        let _subscriptions = vec![
            subscribe_opacity(&opacity_slider, cx),
            subscribe_accent(&accent_picker, cx),
        ];
        Self {
            opacity_slider,
            accent_picker,
            _subscriptions,
        }
    }

    pub fn sync_from_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = AppSettings::global(cx).window_opacity * 100.0;
        if (self.opacity_slider.read(cx).value().start() - value).abs() <= f32::EPSILON {
            return;
        }
        self.opacity_slider.update(cx, |slider, cx| {
            slider.set_value(value, window, cx);
        });
    }

    pub fn set_opacity(
        &mut self,
        value: f32,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.opacity_slider.update(cx, |slider, cx| {
            slider.set_value(value, window, cx);
        });
        AppSettings::update(cx, |settings| settings.window_opacity = value / 100.0);
        if save {
            AppSettings::global(cx).save();
        }
        window.refresh();
    }
}

fn subscribe_opacity(
    slider: &Entity<SliderState>,
    cx: &mut Context<AppearanceSettingsState>,
) -> gpui::Subscription {
    cx.subscribe(slider, |_, _, event: &SliderEvent, cx| {
        let value = match event {
            SliderEvent::Change(value) | SliderEvent::Release(value) => value.start() / 100.0,
        };
        AppSettings::update(cx, |settings| settings.window_opacity = value);
        if matches!(event, SliderEvent::Release(_)) {
            AppSettings::global(cx).save();
        }
        if matches!(event, SliderEvent::Release(_)) {
            cx.refresh_windows();
        }
    })
}

fn subscribe_accent(
    picker: &Entity<ColorPickerState>,
    cx: &mut Context<AppearanceSettingsState>,
) -> gpui::Subscription {
    cx.subscribe(picker, |_, _, event: &ColorPickerEvent, cx| {
        let ColorPickerEvent::Change(Some(color)) = event else {
            return;
        };
        AppSettings::update_and_save(cx, |settings| {
            settings.custom_accent_color = color.to_hex();
        });
        themes::apply_appearance(&AppSettings::current(cx), cx);
    })
}
