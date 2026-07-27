use gpui::{FontWeight, Hsla, Styled, px, rgb};

use crate::{ColorToken, TailwindModifier};

const TAILWIND_SPACING_PX: f32 = 4.0;

pub fn apply_modifiers<E: Styled>(element: E, modifiers: &[TailwindModifier]) -> E {
    modifiers.iter().fold(element, |element, modifier| {
        apply_modifier(element, *modifier)
    })
}

fn apply_modifier<E: Styled>(element: E, modifier: TailwindModifier) -> E {
    match modifier {
        TailwindModifier::Flex
        | TailwindModifier::FlexColumn
        | TailwindModifier::FlexRow
        | TailwindModifier::FlexOne
        | TailwindModifier::FlexShrinkZero => apply_flex(element, modifier),
        TailwindModifier::ItemsStart
        | TailwindModifier::ItemsCenter
        | TailwindModifier::ItemsEnd
        | TailwindModifier::JustifyCenter
        | TailwindModifier::JustifyBetween
        | TailwindModifier::JustifyEnd => apply_alignment(element, modifier),
        TailwindModifier::Gap(_)
        | TailwindModifier::Padding(_)
        | TailwindModifier::PaddingX(_)
        | TailwindModifier::PaddingY(_) => apply_spacing(element, modifier),
        TailwindModifier::WidthFull
        | TailwindModifier::HeightFull
        | TailwindModifier::SizeFull
        | TailwindModifier::MinWidthZero
        | TailwindModifier::MinHeightZero
        | TailwindModifier::OverflowHidden => apply_size(element, modifier),
        _ => apply_visual(element, modifier),
    }
}

fn apply_flex<E: Styled>(element: E, modifier: TailwindModifier) -> E {
    match modifier {
        TailwindModifier::Flex => element.flex(),
        TailwindModifier::FlexColumn => element.flex_col(),
        TailwindModifier::FlexRow => element.flex_row(),
        TailwindModifier::FlexOne => element.flex_1(),
        TailwindModifier::FlexShrinkZero => element.flex_shrink_0(),
        _ => element,
    }
}

fn apply_alignment<E: Styled>(element: E, modifier: TailwindModifier) -> E {
    match modifier {
        TailwindModifier::ItemsStart => element.items_start(),
        TailwindModifier::ItemsCenter => element.items_center(),
        TailwindModifier::ItemsEnd => element.items_end(),
        TailwindModifier::JustifyCenter => element.justify_center(),
        TailwindModifier::JustifyBetween => element.justify_between(),
        TailwindModifier::JustifyEnd => element.justify_end(),
        _ => element,
    }
}

fn apply_spacing<E: Styled>(element: E, modifier: TailwindModifier) -> E {
    match modifier {
        TailwindModifier::Gap(value) => element.gap(spacing(value)),
        TailwindModifier::Padding(value) => element.p(spacing(value)),
        TailwindModifier::PaddingX(value) => element.px(spacing(value)),
        TailwindModifier::PaddingY(value) => element.py(spacing(value)),
        _ => element,
    }
}

fn apply_size<E: Styled>(element: E, modifier: TailwindModifier) -> E {
    match modifier {
        TailwindModifier::WidthFull => element.w_full(),
        TailwindModifier::HeightFull => element.h_full(),
        TailwindModifier::SizeFull => element.size_full(),
        TailwindModifier::MinWidthZero => element.min_w_0(),
        TailwindModifier::MinHeightZero => element.min_h_0(),
        TailwindModifier::OverflowHidden => element.overflow_hidden(),
        _ => element,
    }
}

fn apply_visual<E: Styled>(element: E, modifier: TailwindModifier) -> E {
    match modifier {
        TailwindModifier::Background(color) => element.bg(color_value(color)),
        TailwindModifier::TextColor(color) => element.text_color(color_value(color)),
        TailwindModifier::Border => element.border_1(),
        TailwindModifier::BorderColor(color) => element.border_color(color_value(color)),
        TailwindModifier::Rounded(value) => element.rounded(px(f32::from(value))),
        TailwindModifier::TextSize(value) => element.text_size(px(f32::from(value))),
        TailwindModifier::FontSemibold => element.font_weight(FontWeight::SEMIBOLD),
        _ => element,
    }
}

fn spacing(value: u16) -> gpui::Pixels {
    px(f32::from(value) * TAILWIND_SPACING_PX)
}

fn color_value(color: ColorToken) -> Hsla {
    let value = match color {
        ColorToken::Zinc950 => 0x09_09_0b,
        ColorToken::Zinc900 => 0x18_18_1b,
        ColorToken::Zinc800 => 0x27_27_2a,
        ColorToken::Zinc700 => 0x3f_3f_46,
        ColorToken::Zinc400 => 0xa1_a1_aa,
        ColorToken::Zinc100 => 0xf4_f4_f5,
        ColorToken::Blue600 => 0x25_63_eb,
        ColorToken::Emerald400 => 0x34_d3_99,
        ColorToken::White => 0xff_ff_ff,
    };
    rgb(value).into()
}
