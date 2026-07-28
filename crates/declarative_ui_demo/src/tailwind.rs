const ROUNDED_MEDIUM_PX: u16 = 6;
const ROUNDED_LARGE_PX: u16 = 8;
const TEXT_SMALL_PX: u16 = 14;
const TEXT_BASE_PX: u16 = 16;
const TEXT_LARGE_PX: u16 = 18;
const TEXT_EXTRA_LARGE_PX: u16 = 20;
pub const MAX_SPACING_SCALE: u16 = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorToken {
    Zinc950,
    Zinc900,
    Zinc800,
    Zinc700,
    Zinc400,
    Zinc100,
    Blue600,
    Emerald400,
    White,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailwindModifier {
    Flex,
    FlexColumn,
    FlexRow,
    FlexOne,
    FlexShrinkZero,
    ItemsStart,
    ItemsCenter,
    ItemsEnd,
    JustifyCenter,
    JustifyBetween,
    JustifyEnd,
    Gap(u16),
    Padding(u16),
    PaddingX(u16),
    PaddingY(u16),
    WidthFull,
    HeightFull,
    SizeFull,
    MinWidthZero,
    MinHeightZero,
    Background(ColorToken),
    TextColor(ColorToken),
    Border,
    BorderColor(ColorToken),
    Rounded(u16),
    TextSize(u16),
    FontSemibold,
    OverflowHidden,
    OverflowYScroll,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TailwindParseResult {
    pub modifiers: Vec<TailwindModifier>,
    pub unsupported: Vec<String>,
}

pub fn parse_classes(classes: &[String]) -> TailwindParseResult {
    let mut result = TailwindParseResult::default();
    for class in classes {
        if let Some(modifier) = parse_class(class) {
            result.modifiers.push(modifier);
        } else {
            result.unsupported.push(class.clone());
        }
    }
    result
}

fn parse_class(class: &str) -> Option<TailwindModifier> {
    match class {
        "flex" => Some(TailwindModifier::Flex),
        "flex-col" => Some(TailwindModifier::FlexColumn),
        "flex-row" => Some(TailwindModifier::FlexRow),
        "flex-1" => Some(TailwindModifier::FlexOne),
        "flex-shrink-0" => Some(TailwindModifier::FlexShrinkZero),
        "items-start" => Some(TailwindModifier::ItemsStart),
        "items-center" => Some(TailwindModifier::ItemsCenter),
        "items-end" => Some(TailwindModifier::ItemsEnd),
        "justify-center" => Some(TailwindModifier::JustifyCenter),
        "justify-between" => Some(TailwindModifier::JustifyBetween),
        "justify-end" => Some(TailwindModifier::JustifyEnd),
        "w-full" => Some(TailwindModifier::WidthFull),
        "h-full" => Some(TailwindModifier::HeightFull),
        "size-full" => Some(TailwindModifier::SizeFull),
        "min-w-0" => Some(TailwindModifier::MinWidthZero),
        "min-h-0" => Some(TailwindModifier::MinHeightZero),
        "border" => Some(TailwindModifier::Border),
        "rounded-md" => Some(TailwindModifier::Rounded(ROUNDED_MEDIUM_PX)),
        "rounded-lg" => Some(TailwindModifier::Rounded(ROUNDED_LARGE_PX)),
        "text-sm" => Some(TailwindModifier::TextSize(TEXT_SMALL_PX)),
        "text-base" => Some(TailwindModifier::TextSize(TEXT_BASE_PX)),
        "text-lg" => Some(TailwindModifier::TextSize(TEXT_LARGE_PX)),
        "text-xl" => Some(TailwindModifier::TextSize(TEXT_EXTRA_LARGE_PX)),
        "font-semibold" => Some(TailwindModifier::FontSemibold),
        "overflow-hidden" => Some(TailwindModifier::OverflowHidden),
        "overflow-y-scroll" => Some(TailwindModifier::OverflowYScroll),
        _ => parse_value_class(class).or_else(|| parse_color_class(class)),
    }
}

fn parse_value_class(class: &str) -> Option<TailwindModifier> {
    let (prefix, value) = class.rsplit_once('-')?;
    let value = value.parse::<u16>().ok()?;
    if value > MAX_SPACING_SCALE {
        return None;
    }
    match prefix {
        "gap" => Some(TailwindModifier::Gap(value)),
        "p" => Some(TailwindModifier::Padding(value)),
        "px" => Some(TailwindModifier::PaddingX(value)),
        "py" => Some(TailwindModifier::PaddingY(value)),
        _ => None,
    }
}

fn parse_color_class(class: &str) -> Option<TailwindModifier> {
    for (prefix, constructor) in [
        (
            "bg-",
            TailwindModifier::Background as fn(ColorToken) -> TailwindModifier,
        ),
        ("text-", TailwindModifier::TextColor),
        ("border-", TailwindModifier::BorderColor),
    ] {
        if let Some(name) = class.strip_prefix(prefix) {
            return parse_color(name).map(constructor);
        }
    }
    None
}

fn parse_color(name: &str) -> Option<ColorToken> {
    match name {
        "zinc-950" => Some(ColorToken::Zinc950),
        "zinc-900" => Some(ColorToken::Zinc900),
        "zinc-800" => Some(ColorToken::Zinc800),
        "zinc-700" => Some(ColorToken::Zinc700),
        "zinc-400" => Some(ColorToken::Zinc400),
        "zinc-100" => Some(ColorToken::Zinc100),
        "blue-600" => Some(ColorToken::Blue600),
        "emerald-400" => Some(ColorToken::Emerald400),
        "white" => Some(ColorToken::White),
        _ => None,
    }
}
