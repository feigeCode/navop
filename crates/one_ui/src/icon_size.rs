use gpui::{Pixels, px};
use gpui_component::Size;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IconSize {
    Micro,
    Small,
    #[default]
    Default,
    Medium,
    Large,
    Display,
    Hero,
}

impl IconSize {
    pub fn pixels(self) -> Pixels {
        match self {
            Self::Micro => px(12.),
            Self::Small => px(14.),
            Self::Default => px(16.),
            Self::Medium => px(20.),
            Self::Large => px(24.),
            Self::Display => px(32.),
            Self::Hero => px(40.),
        }
    }
}

impl From<IconSize> for Size {
    fn from(size: IconSize) -> Self {
        Size::Size(size.pixels())
    }
}

impl From<IconSize> for gpui_component::IconSize {
    fn from(size: IconSize) -> Self {
        match size {
            IconSize::Micro => gpui_component::IconSize::Micro,
            IconSize::Small => gpui_component::IconSize::Small,
            IconSize::Default => gpui_component::IconSize::Default,
            IconSize::Medium => gpui_component::IconSize::Medium,
            IconSize::Large => gpui_component::IconSize::Large,
            IconSize::Display => gpui_component::IconSize::Display,
            IconSize::Hero => gpui_component::IconSize::Hero,
        }
    }
}
