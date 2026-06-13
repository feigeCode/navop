use gpui::{App, Pixels, px};
use one_core::settings::AppSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableDisplaySettings {
    pub row_height: u32,
}

impl TableDisplaySettings {
    pub const DEFAULT_ROW_HEIGHT: u32 = 44;
    pub const MIN_ROW_HEIGHT: u32 = 24;
    pub const MAX_ROW_HEIGHT: u32 = 100;

    pub fn new(row_height: u32) -> Self {
        Self {
            row_height: clamp_row_height(row_height),
        }
    }
}

impl Default for TableDisplaySettings {
    fn default() -> Self {
        Self {
            row_height: Self::DEFAULT_ROW_HEIGHT,
        }
    }
}

pub fn init_table_display_settings(cx: &mut App, settings: TableDisplaySettings) {
    AppSettings::update(cx, |app_settings| {
        app_settings.table_row_height = settings.row_height;
    });
}

pub fn set_table_row_height(height: u32, cx: &mut App) {
    let settings = TableDisplaySettings::new(height);
    AppSettings::update_and_save(cx, |app_settings| {
        app_settings.table_row_height = settings.row_height;
    });
}

pub fn table_row_height(cx: &App) -> Pixels {
    table_row_height_or(cx, px(TableDisplaySettings::DEFAULT_ROW_HEIGHT as f32))
}

pub fn table_row_height_or(cx: &App, fallback: Pixels) -> Pixels {
    cx.try_global::<AppSettings>()
        .map(|settings| px(clamp_row_height(settings.table_row_height) as f32))
        .unwrap_or(fallback)
}

fn clamp_row_height(height: u32) -> u32 {
    height.clamp(
        TableDisplaySettings::MIN_ROW_HEIGHT,
        TableDisplaySettings::MAX_ROW_HEIGHT,
    )
}

#[cfg(test)]
mod tests {
    use super::{TableDisplaySettings, clamp_row_height};

    #[test]
    fn table_row_height_clamps_to_supported_range() {
        assert_eq!(
            TableDisplaySettings::MIN_ROW_HEIGHT,
            clamp_row_height(TableDisplaySettings::MIN_ROW_HEIGHT - 1)
        );
        assert_eq!(44, clamp_row_height(44));
        assert_eq!(
            TableDisplaySettings::MAX_ROW_HEIGHT,
            clamp_row_height(TableDisplaySettings::MAX_ROW_HEIGHT + 1)
        );
    }
}
