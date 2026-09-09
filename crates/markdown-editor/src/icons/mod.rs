//! Semantic icon mappings for the embedded editor.
//!
//! The editor intentionally owns no icon assets. Every icon is mapped to
//! Navop's shared `gpui_component` icon catalog here so editor UI code does not
//! depend on asset paths or scatter project-specific icon choices throughout
//! the renderer.

use gpui_component::IconName;

use crate::components::CalloutVariant;

pub(crate) mod callout {
    use super::{CalloutVariant, IconName};

    pub(crate) fn icon(variant: CalloutVariant) -> IconName {
        match variant {
            CalloutVariant::Note => IconName::Info,
            CalloutVariant::Tip => IconName::Plus,
            CalloutVariant::Important => IconName::Star,
            CalloutVariant::Warning => IconName::TriangleAlert,
            CalloutVariant::Caution => IconName::CircleX,
        }
    }
}

pub(crate) mod indicators {
    use super::IconName;

    pub(crate) const CHECKED: IconName = IconName::Check;
}

pub(crate) mod alignment {
    use super::IconName;

    pub(crate) const LEFT: IconName = IconName::PanelLeft;
    pub(crate) const CENTER: IconName = IconName::Menu;
    pub(crate) const RIGHT: IconName = IconName::PanelRight;
}
