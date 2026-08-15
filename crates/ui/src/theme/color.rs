use std::{collections::HashMap, fmt::Display};

use gpui::{Hsla, SharedString};
use palette::IntoColor;
use serde::{Deserialize, Deserializer, de::Error as _};

use anyhow::{Error, Result, anyhow};

/// Create a [`gpui::Hsla`] color.
///
/// - h: 0..360.0
/// - s: 0.0..100.0
/// - l: 0.0..100.0
#[inline]
pub fn hsl(h: f32, s: f32, l: f32) -> Hsla {
    Hsla::new(h, s / 100.0, l / 100.0, 1.0)
}

pub trait Colorize: Sized {
    /// Returns a new color with the given opacity.
    ///
    /// The opacity is a value between 0.0 and 1.0, where 0.0 is fully transparent and 1.0 is fully opaque.
    fn opacity(&self, opacity: f32) -> Self;
    /// Returns a new color with each channel divided by the given divisor.
    ///
    /// The divisor in range of 0.0 .. 1.0
    fn divide(&self, divisor: f32) -> Self;
    /// Return inverted color
    fn invert(&self) -> Self;
    /// Return inverted lightness
    fn invert_l(&self) -> Self;
    /// Return a new color with the lightness increased by the given factor.
    ///
    /// factor range: 0.0 .. 1.0
    fn lighten(&self, amount: f32) -> Self;
    /// Return a new color with the darkness increased by the given factor.
    ///
    /// factor range: 0.0 .. 1.0
    fn darken(&self, amount: f32) -> Self;
    /// Return a new color with the same lightness and alpha but different hue and saturation.
    fn apply(&self, base_color: Self) -> Self;
    /// Blend another color over this color.
    fn blend(&self, other: Self) -> Self;

    /// Mix two colors together, the `factor` is a value between 0.0 and 1.0 for first color.
    fn mix(&self, other: Self, factor: f32) -> Self;
    /// Mix two colors together in Oklab color space, the `factor` is a value between 0.0 and 1.0 for first color.
    ///
    /// This is similar to CSS `color-mix(in oklab, color1 factor%, color2)`.
    fn mix_oklab(&self, other: Self, factor: f32) -> Self;
    /// Change the `Hue` of the color by the given in range: 0.0 .. 1.0
    fn hue(&self, hue: f32) -> Self;
    /// Change the `Saturation` of the color by the given value in range: 0.0 .. 1.0
    fn saturation(&self, saturation: f32) -> Self;
    /// Change the `Lightness` of the color by the given value in range: 0.0 .. 1.0
    fn lightness(&self, lightness: f32) -> Self;

    /// Convert the color to a hex string. For example, "#F8FAFC".
    fn to_hex(&self) -> String;
    /// Parse a hex string to a color.
    fn parse_hex(hex: &str) -> Result<Self>;
}

/// Serde compatibility for theme colors.
///
/// GPUI historically serialized `Hsla` as a hex string. Newer GPUI versions
/// re-export `palette::Hsla`, whose derived serde representation is a
/// structured object. Theme files keep using the stable, human-readable hex
/// representation while still accepting the newer structured representation.
pub(crate) mod hsla_serde {
    use super::{Colorize as _, Hsla};
    use serde::{Deserialize, Deserializer, Serializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum HslaValue {
        Hex(String),
        Structured(Hsla),
    }

    fn into_hsla<E>(value: HslaValue) -> Result<Hsla, E>
    where
        E: serde::de::Error,
    {
        match value {
            HslaValue::Hex(value) => Hsla::parse_hex(&value).map_err(E::custom),
            HslaValue::Structured(color) => Ok(color),
        }
    }

    pub(crate) mod option {
        use super::*;

        pub fn serialize<S>(color: &Option<Hsla>, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            match color {
                Some(color) => serializer.serialize_some(&color.to_hex()),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Hsla>, D::Error>
        where
            D: Deserializer<'de>,
        {
            Option::<HslaValue>::deserialize(deserializer)?
                .map(into_hsla)
                .transpose()
        }
    }
}

/// Helper functions for Oklab color space conversions
mod oklab {
    use gpui::Rgba;

    /// Convert sRGB component to linear RGB
    #[inline]
    fn to_linear(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// Convert linear RGB component to sRGB
    #[inline]
    fn from_linear(c: f32) -> f32 {
        if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    }

    /// Convert RGB to Oklab color space
    #[allow(non_snake_case)]
    pub fn rgb_to_oklab(rgb: Rgba) -> (f32, f32, f32) {
        // sRGB to linear RGB
        let lr = to_linear(rgb.red);
        let lg = to_linear(rgb.green);
        let lb = to_linear(rgb.blue);

        // Linear RGB to LMS
        let l = 0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb;
        let m = 0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb;
        let s = 0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb;

        // LMS to Oklab (using cube root)
        let l_ = l.cbrt();
        let m_ = m.cbrt();
        let s_ = s.cbrt();

        let L = 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_;
        let a = 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_;
        let b = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_;

        (L, a, b)
    }

    /// Convert Oklab to RGB color space
    #[allow(non_snake_case)]
    pub fn oklab_to_rgb(L: f32, a: f32, b: f32) -> Rgba {
        // Oklab to LMS
        let l_ = L + 0.3963377774 * a + 0.2158037573 * b;
        let m_ = L - 0.1055613458 * a - 0.0638541728 * b;
        let s_ = L - 0.0894841775 * a - 1.2914855480 * b;

        let l = l_ * l_ * l_;
        let m = m_ * m_ * m_;
        let s = s_ * s_ * s_;

        // LMS to Linear RGB
        let lr = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
        let lg = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
        let lb = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;

        // Linear RGB to sRGB
        Rgba::new(
            from_linear(lr).clamp(0.0, 1.0),
            from_linear(lg).clamp(0.0, 1.0),
            from_linear(lb).clamp(0.0, 1.0),
            1.0,
        )
    }
}

impl Colorize for Hsla {
    fn opacity(&self, factor: f32) -> Self {
        let mut color = *self;
        color.alpha *= factor.clamp(0.0, 1.0);
        color
    }

    fn divide(&self, divisor: f32) -> Self {
        let mut color = *self;
        color.alpha = divisor;
        color
    }

    fn invert(&self) -> Self {
        Hsla::new(
            360.0 - self.hue.into_positive_degrees(),
            1.0 - self.saturation,
            1.0 - self.lightness,
            self.alpha,
        )
    }

    fn invert_l(&self) -> Self {
        let mut color = *self;
        color.lightness = 1.0 - self.lightness;
        color
    }

    fn lighten(&self, factor: f32) -> Self {
        let mut color = *self;
        color.lightness *= 1.0 + factor.clamp(0.0, 1.0);
        color
    }

    fn darken(&self, factor: f32) -> Self {
        let mut color = *self;
        color.lightness *= 1.0 - factor.clamp(0.0, 1.0);
        color
    }

    fn apply(&self, new_color: Self) -> Self {
        Hsla::new(
            new_color.hue,
            new_color.saturation,
            self.lightness,
            self.alpha,
        )
    }

    fn blend(&self, other: Self) -> Self {
        gpui::ColorExt::blend(self, &other)
    }

    /// Reference:
    /// https://github.com/bevyengine/bevy/blob/85eceb022da0326b47ac2b0d9202c9c9f01835bb/crates/bevy_color/src/hsla.rs#L112
    fn mix(&self, other: Self, factor: f32) -> Self {
        let factor = factor.clamp(0.0, 1.0);
        let inv = 1.0 - factor;

        #[inline]
        fn lerp_hue(a: f32, b: f32, t: f32) -> f32 {
            let diff = (b - a + 180.0).rem_euclid(360.) - 180.;
            (a + diff * t).rem_euclid(360.0)
        }

        Hsla::new(
            lerp_hue(
                self.hue.into_positive_degrees(),
                other.hue.into_positive_degrees(),
                factor,
            ),
            self.saturation * factor + other.saturation * inv,
            self.lightness * factor + other.lightness * inv,
            self.alpha * factor + other.alpha * inv,
        )
    }

    #[allow(non_snake_case)]
    fn mix_oklab(&self, other: Self, factor: f32) -> Self {
        let factor = factor.clamp(0.0, 1.0);
        let inv = 1.0 - factor;

        // Interpolate alpha first
        let result_alpha = self.alpha * factor + other.alpha * inv;

        // Handle the case where result alpha is zero
        if result_alpha == 0.0 {
            return Hsla::new(0.0, 0.0, 0.0, 0.0);
        }

        // Convert both colors to RGB
        let rgb1: gpui::Rgba = (*self).into_color();
        let rgb2: gpui::Rgba = other.into_color();

        // Convert to Oklab color space
        let (l1, a1, b1) = oklab::rgb_to_oklab(rgb1);
        let (l2, a2, b2) = oklab::rgb_to_oklab(rgb2);

        // Premultiply alpha in Oklab space (using alpha-premultiplied interpolation)
        // This matches CSS color-mix behavior
        let alpha1 = self.alpha;
        let alpha2 = other.alpha;

        // Premultiply
        let l1_pm = l1 * alpha1;
        let a1_pm = a1 * alpha1;
        let b1_pm = b1 * alpha1;

        let l2_pm = l2 * alpha2;
        let a2_pm = a2 * alpha2;
        let b2_pm = b2 * alpha2;

        // Interpolate premultiplied values
        let L_pm = l1_pm * factor + l2_pm * inv;
        let a_pm = a1_pm * factor + a2_pm * inv;
        let b_pm = b1_pm * factor + b2_pm * inv;

        // Unpremultiply
        let L = L_pm / result_alpha;
        let a = a_pm / result_alpha;
        let b = b_pm / result_alpha;

        // Convert back to RGB
        let mut rgb = oklab::oklab_to_rgb(L, a, b);
        rgb.alpha = result_alpha;

        // Convert RGB to HSLA
        rgb.into_color()
    }

    fn to_hex(&self) -> String {
        fn channel_to_u8(channel: f32) -> u32 {
            let scaled = channel * 255.;
            let rounded = scaled.round();

            if (scaled - rounded).abs() < 0.0001 {
                rounded as u32
            } else {
                scaled as u32
            }
        }

        let rgb: gpui::Rgba = (*self).into_color();

        if rgb.alpha < 1. {
            return format!(
                "#{:02X}{:02X}{:02X}{:02X}",
                channel_to_u8(rgb.red),
                channel_to_u8(rgb.green),
                channel_to_u8(rgb.blue),
                channel_to_u8(self.alpha)
            );
        }

        format!(
            "#{:02X}{:02X}{:02X}",
            channel_to_u8(rgb.red),
            channel_to_u8(rgb.green),
            channel_to_u8(rgb.blue)
        )
    }

    fn parse_hex(hex: &str) -> Result<Self> {
        let hex = hex.strip_prefix('#').unwrap_or(hex);
        let expanded;
        let hex = match hex.len() {
            3 | 4 => {
                expanded = hex
                    .chars()
                    .flat_map(|channel| [channel, channel])
                    .collect::<String>();
                expanded.as_str()
            }
            6 | 8 => hex,
            _ => return Err(anyhow::anyhow!("invalid hex color")),
        };

        let r = u8::from_str_radix(&hex[0..2], 16)? as f32 / 255.;
        let g = u8::from_str_radix(&hex[2..4], 16)? as f32 / 255.;
        let b = u8::from_str_radix(&hex[4..6], 16)? as f32 / 255.;
        let a = if hex.len() == 8 {
            u8::from_str_radix(&hex[6..8], 16)? as f32 / 255.
        } else {
            1.
        };

        Ok(gpui::Rgba::new(r, g, b, a).into_color())
    }

    fn hue(&self, hue: f32) -> Self {
        let mut color = *self;
        color.hue = palette::RgbHue::from_degrees(hue.clamp(0., 1.) * 360.0);
        color
    }

    fn saturation(&self, saturation: f32) -> Self {
        let mut color = *self;
        color.saturation = saturation.clamp(0., 1.);
        color
    }

    fn lightness(&self, lightness: f32) -> Self {
        let mut color = *self;
        color.lightness = lightness.clamp(0., 1.);
        color
    }
}

pub(crate) static DEFAULT_COLORS: once_cell::sync::Lazy<ShadcnColors> =
    once_cell::sync::Lazy::new(|| {
        serde_json::from_str(include_str!("./default-colors.json"))
            .expect("failed to parse default-colors.json")
    });

type ColorScales = HashMap<usize, ShadcnColor>;

mod color_scales {
    use std::collections::HashMap;

    use super::{ColorScales, ShadcnColor};

    use serde::de::{Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ColorScales, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut map = HashMap::new();
        for color in Vec::<ShadcnColor>::deserialize(deserializer)? {
            map.insert(color.scale, color);
        }
        Ok(map)
    }
}

/// Enum representing the available color names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorName {
    White,
    Black,
    Neutral,
    Gray,
    Red,
    Orange,
    Amber,
    Yellow,
    Lime,
    Green,
    Emerald,
    Teal,
    Cyan,
    Sky,
    Blue,
    Indigo,
    Violet,
    Purple,
    Fuchsia,
    Pink,
    Rose,
}

impl Display for ColorName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

// Strict color name parser.
impl TryFrom<&str> for ColorName {
    type Error = anyhow::Error;
    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "white" => Ok(ColorName::White),
            "black" => Ok(ColorName::Black),
            "neutral" => Ok(ColorName::Neutral),
            "gray" => Ok(ColorName::Gray),
            "red" => Ok(ColorName::Red),
            "orange" => Ok(ColorName::Orange),
            "amber" => Ok(ColorName::Amber),
            "yellow" => Ok(ColorName::Yellow),
            "lime" => Ok(ColorName::Lime),
            "green" => Ok(ColorName::Green),
            "emerald" => Ok(ColorName::Emerald),
            "teal" => Ok(ColorName::Teal),
            "cyan" => Ok(ColorName::Cyan),
            "sky" => Ok(ColorName::Sky),
            "blue" => Ok(ColorName::Blue),
            "indigo" => Ok(ColorName::Indigo),
            "violet" => Ok(ColorName::Violet),
            "purple" => Ok(ColorName::Purple),
            "fuchsia" => Ok(ColorName::Fuchsia),
            "pink" => Ok(ColorName::Pink),
            "rose" => Ok(ColorName::Rose),
            _ => Err(anyhow::anyhow!("Invalid color name")),
        }
    }
}

impl TryFrom<SharedString> for ColorName {
    type Error = anyhow::Error;
    fn try_from(value: SharedString) -> std::result::Result<Self, Self::Error> {
        value.as_ref().try_into()
    }
}

impl ColorName {
    /// Returns all available color names.
    pub fn all() -> [Self; 19] {
        [
            ColorName::Neutral,
            ColorName::Gray,
            ColorName::Red,
            ColorName::Orange,
            ColorName::Amber,
            ColorName::Yellow,
            ColorName::Lime,
            ColorName::Green,
            ColorName::Emerald,
            ColorName::Teal,
            ColorName::Cyan,
            ColorName::Sky,
            ColorName::Blue,
            ColorName::Indigo,
            ColorName::Violet,
            ColorName::Purple,
            ColorName::Fuchsia,
            ColorName::Pink,
            ColorName::Rose,
        ]
    }

    /// Returns the color for the given scale.
    ///
    /// The `scale` is any of `[50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950]`
    /// falls back to 500 if out of range.
    pub fn scale(&self, scale: usize) -> Hsla {
        if self == &ColorName::White {
            return DEFAULT_COLORS.white.hsla;
        }
        if self == &ColorName::Black {
            return DEFAULT_COLORS.black.hsla;
        }

        let colors = match self {
            ColorName::Neutral => &DEFAULT_COLORS.neutral,
            ColorName::Gray => &DEFAULT_COLORS.gray,
            ColorName::Red => &DEFAULT_COLORS.red,
            ColorName::Orange => &DEFAULT_COLORS.orange,
            ColorName::Amber => &DEFAULT_COLORS.amber,
            ColorName::Yellow => &DEFAULT_COLORS.yellow,
            ColorName::Lime => &DEFAULT_COLORS.lime,
            ColorName::Green => &DEFAULT_COLORS.green,
            ColorName::Emerald => &DEFAULT_COLORS.emerald,
            ColorName::Teal => &DEFAULT_COLORS.teal,
            ColorName::Cyan => &DEFAULT_COLORS.cyan,
            ColorName::Sky => &DEFAULT_COLORS.sky,
            ColorName::Blue => &DEFAULT_COLORS.blue,
            ColorName::Indigo => &DEFAULT_COLORS.indigo,
            ColorName::Violet => &DEFAULT_COLORS.violet,
            ColorName::Purple => &DEFAULT_COLORS.purple,
            ColorName::Fuchsia => &DEFAULT_COLORS.fuchsia,
            ColorName::Pink => &DEFAULT_COLORS.pink,
            ColorName::Rose => &DEFAULT_COLORS.rose,
            _ => unreachable!(),
        };

        if let Some(color) = colors.get(&scale) {
            color.hsla
        } else {
            colors.get(&500).unwrap().hsla
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub(crate) struct ShadcnColors {
    pub(crate) black: ShadcnColor,
    pub(crate) white: ShadcnColor,
    #[serde(with = "color_scales")]
    pub(crate) slate: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) gray: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) zinc: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) neutral: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) stone: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) red: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) orange: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) amber: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) yellow: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) lime: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) green: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) emerald: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) teal: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) cyan: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) sky: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) blue: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) indigo: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) violet: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) purple: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) fuchsia: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) pink: ColorScales,
    #[serde(with = "color_scales")]
    pub(crate) rose: ColorScales,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
pub(crate) struct ShadcnColor {
    #[serde(default)]
    pub(crate) scale: usize,
    #[serde(deserialize_with = "from_hsl_channel", alias = "hslChannel")]
    pub(crate) hsla: Hsla,
}

/// Deserialize Hsla from a string in the format "210 40% 98%"
fn from_hsl_channel<'de, D>(deserializer: D) -> Result<Hsla, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer).unwrap();

    let mut parts = s.split_whitespace();
    if parts.clone().count() != 3 {
        return Err(D::Error::custom(
            "expected hslChannel has 3 parts, e.g: '210 40% 98%'",
        ));
    }

    fn parse_number(s: &str) -> f32 {
        s.trim_end_matches('%')
            .parse()
            .expect("failed to parse number")
    }

    let (h, s, l) = (
        parse_number(parts.next().unwrap()),
        parse_number(parts.next().unwrap()),
        parse_number(parts.next().unwrap()),
    );

    Ok(hsl(h, s, l))
}

macro_rules! color_method {
    ($color:tt, $scale:tt) => {
        paste::paste! {
            #[inline]
            #[allow(unused)]
            pub fn [<$color _ $scale>]() -> Hsla {
                if let Some(color) = DEFAULT_COLORS.$color.get(&($scale as usize)) {
                    return color.hsla;
                }

                black()
            }
        }
    };
}

macro_rules! color_methods {
    ($color:tt) => {
        paste::paste! {
            /// Get color by scale number.
            ///
            /// The possible scale numbers are:
            /// 50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950
            ///
            /// If the scale number is not found, it will return black color.
            #[inline]
            pub fn [<$color>](scale: usize) -> Hsla {
                if let Some(color) = DEFAULT_COLORS.$color.get(&scale) {
                    return color.hsla;
                }

                black()
            }
        }

        color_method!($color, 50);
        color_method!($color, 100);
        color_method!($color, 200);
        color_method!($color, 300);
        color_method!($color, 400);
        color_method!($color, 500);
        color_method!($color, 600);
        color_method!($color, 700);
        color_method!($color, 800);
        color_method!($color, 900);
        color_method!($color, 950);
    };
}

pub fn black() -> Hsla {
    DEFAULT_COLORS.black.hsla
}

pub fn white() -> Hsla {
    DEFAULT_COLORS.white.hsla
}

color_methods!(slate);
color_methods!(gray);
color_methods!(zinc);
color_methods!(neutral);
color_methods!(stone);
color_methods!(red);
color_methods!(orange);
color_methods!(amber);
color_methods!(yellow);
color_methods!(lime);
color_methods!(green);
color_methods!(emerald);
color_methods!(teal);
color_methods!(cyan);
color_methods!(sky);
color_methods!(blue);
color_methods!(indigo);
color_methods!(violet);
color_methods!(purple);
color_methods!(fuchsia);
color_methods!(pink);
color_methods!(rose);

/// Try to parse the color, HEX or [Tailwind Color](https://tailwindcss.com/docs/colors) expression.
///
/// # Parameter `color` should be one string value listed below:
///
/// - `#RRGGBB` - The HEX color string.
/// - `#RRGGBBAA` - The HEX color string with alpha.
///
/// Or the Tailwind Color format:
///
/// - `name` - The color name `black`, `white`, or any other defined in `crate::color`.
/// - `name-scale` - The color name with scale.
/// - `name/opacity` - The color name with opacity, `opacity` should be an integer between 0 and 100.
/// - `name-scale/opacity` - The color name with scale and opacity.
///
pub fn try_parse_color(color: &str) -> Result<Hsla> {
    if color.starts_with("#") {
        return Hsla::parse_hex(color);
    }

    let mut name = String::new();
    let mut scale = None;
    let mut opacity = None;
    // 0: name, 1: scale, 2: opacity
    let mut state = 0;
    let mut part = String::new();

    for c in color.chars() {
        match c {
            '-' if state == 0 => {
                name = std::mem::take(&mut part);
                state = 1;
            }
            '/' if state <= 1 => {
                if state == 0 {
                    name = std::mem::take(&mut part);
                } else if state == 1 {
                    scale = part.parse::<usize>().ok();
                    part.clear();
                }
                state = 2;
            }
            _ => part.push(c),
        }
    }

    match state {
        0 => name = part,
        1 => scale = part.parse::<usize>().ok(),
        2 => opacity = part.parse::<f32>().ok(),
        _ => {}
    }

    if name.is_empty() {
        return Err(anyhow!("Empty color name"));
    }

    let mut hsla = match name.as_str() {
        "black" => Ok::<Hsla, Error>(crate::black()),
        "white" => Ok(crate::white()),
        _ => {
            let color_name = ColorName::try_from(name.as_str())?;
            if let Some(scale) = scale {
                Ok(color_name.scale(scale))
            } else {
                Ok(color_name.scale(500))
            }
        }
    }?;

    if let Some(opacity) = opacity {
        if opacity > 100. {
            return Err(anyhow!("Invalid color opacity"));
        }
        hsla = hsla.opacity(opacity / 100.);
    }

    Ok(hsla)
}

#[cfg(test)]
mod tests {
    use gpui::{rgb, rgba};

    use super::*;

    fn assert_hsla_approx_eq(actual: Hsla, expected: Hsla) {
        assert!(
            (actual.hue.into_positive_degrees() - expected.hue.into_positive_degrees()).abs()
                < 0.0001,
            "hue differs: actual={actual:?}, expected={expected:?}"
        );
        assert!(
            (actual.saturation - expected.saturation).abs() < 0.0001,
            "saturation differs: actual={actual:?}, expected={expected:?}"
        );
        assert!(
            (actual.lightness - expected.lightness).abs() < 0.0001,
            "lightness differs: actual={actual:?}, expected={expected:?}"
        );
        assert!(
            (actual.alpha - expected.alpha).abs() < 0.0001,
            "alpha differs: actual={actual:?}, expected={expected:?}"
        );
    }

    #[test]
    fn test_default_colors() {
        assert_eq!(white(), hsl(0.0, 0.0, 100.0));
        assert_eq!(black(), hsl(0.0, 0.0, 0.0));

        assert_eq!(slate_50(), hsl(210.0, 40.0, 98.0));
        assert_eq!(slate_100(), hsl(210.0, 40.0, 96.1));
        assert_eq!(slate_900(), hsl(222.2, 47.4, 11.2));

        assert_eq!(red_50(), hsl(0.0, 85.7, 97.3));
        assert_eq!(yellow_100(), hsl(54.9, 96.7, 88.0));
        assert_eq!(green_200(), hsl(141.0, 78.9, 85.1));
        assert_eq!(cyan_300(), hsl(187.0, 92.4, 69.0));
        assert_eq!(blue_400(), hsl(213.1, 93.9, 67.8));
        assert_eq!(indigo_500(), hsl(238.7, 83.5, 66.7));
    }

    #[test]
    fn test_to_hex_string() {
        let color: Hsla = rgb(0xf8fafc).into_color();
        assert_eq!(color.to_hex(), "#F8FAFC");

        let color: Hsla = rgb(0xfef2f2).into_color();
        assert_eq!(color.to_hex(), "#FEF2F2");

        let color: Hsla = rgba(0x0413fcaa).into_color();
        assert_eq!(color.to_hex(), "#0413FCAA");
    }

    #[test]
    fn test_from_hex_string() {
        let color: Hsla = Hsla::parse_hex("#F8FAFC").unwrap();
        assert_eq!(color, rgb(0xf8fafc).into_color());

        let color: Hsla = Hsla::parse_hex("#FEF2F2").unwrap();
        assert_eq!(color, rgb(0xfef2f2).into_color());

        let color: Hsla = Hsla::parse_hex("#0413FCAA").unwrap();
        assert_eq!(color, rgba(0x0413fcaa).into_color());
    }

    #[test]
    fn test_from_short_hex_string() {
        assert_eq!(
            Hsla::parse_hex("#fff").unwrap(),
            Hsla::parse_hex("#ffffff").unwrap()
        );
        assert_eq!(
            Hsla::parse_hex("#1234").unwrap(),
            Hsla::parse_hex("#11223344").unwrap()
        );
        assert!(Hsla::parse_hex("##fff").is_err());
    }

    #[test]
    fn hsla_serde_uses_hex_and_accepts_palette_structures() {
        #[derive(Debug, serde::Serialize, Deserialize)]
        struct OptionalColor {
            #[serde(default, with = "super::hsla_serde::option")]
            color: Option<Hsla>,
        }

        let color = OptionalColor {
            color: Some(Hsla::parse_hex("#000000").unwrap()),
        };
        assert_eq!(
            serde_json::to_value(&color).unwrap(),
            serde_json::json!({ "color": "#000000" })
        );

        let from_hex: OptionalColor =
            serde_json::from_value(serde_json::json!({ "color": "#1234" })).unwrap();
        assert_eq!(
            from_hex.color.unwrap(),
            Hsla::parse_hex("#11223344").unwrap()
        );

        let structured = serde_json::to_value(Hsla::parse_hex("#33669980").unwrap()).unwrap();
        let from_structured: OptionalColor =
            serde_json::from_value(serde_json::json!({ "color": structured })).unwrap();
        assert_eq!(
            from_structured.color.unwrap(),
            Hsla::parse_hex("#33669980").unwrap()
        );
    }

    #[test]
    fn test_lighten() {
        let color = super::hsl(240.0, 5.0, 30.0);
        let color = color.lighten(0.5);
        assert_eq!(color.lightness, 0.45000002);
        let color = color.lighten(0.5);
        assert_eq!(color.lightness, 0.675);
        let color = color.lighten(0.1);
        assert_eq!(color.lightness, 0.7425);
    }

    #[test]
    fn test_darken() {
        let color = super::hsl(240.0, 5.0, 96.0);
        let color = color.darken(0.5);
        assert_eq!(color.lightness, 0.48);
        let color = color.darken(0.5);
        assert_eq!(color.lightness, 0.24);
    }

    #[test]
    fn test_mix() {
        let red = Hsla::parse_hex("#FF0000").unwrap();
        let blue = Hsla::parse_hex("#0000FF").unwrap();
        let green = Hsla::parse_hex("#00FF00").unwrap();
        let yellow = Hsla::parse_hex("#FFFF00").unwrap();

        assert_eq!(red.mix(blue, 0.5).to_hex(), "#FF00FF");
        assert_eq!(green.mix(red, 0.5).to_hex(), "#FFFF00");
        assert_eq!(blue.mix(yellow, 0.2).to_hex(), "#0099FF");
    }

    #[test]
    fn test_mix_oklab() {
        let red = Hsla::parse_hex("#FF0000").unwrap();
        let blue = Hsla::parse_hex("#0000FF").unwrap();
        let transparent = gpui::transparent_black();

        // Test mixing red with transparent (similar to CSS color-mix example)
        // color-mix(in oklab, red 20%, transparent) should give red with 20% opacity
        let result = red.mix_oklab(transparent, 0.2);
        assert!((result.alpha - 0.2).abs() < 0.01); // Alpha should be 20%

        // The color should remain red (hue should be preserved)
        let rgb_result: gpui::Rgba = result.into_color();
        let rgb_red: gpui::Rgba = red.into_color();
        // Allow some tolerance due to color space conversions
        assert!(
            (rgb_result.red - rgb_red.red).abs() < 0.05,
            "Red channel should be preserved"
        );
        assert!(rgb_result.green < 0.05, "Green channel should be near 0");
        assert!(rgb_result.blue < 0.05, "Blue channel should be near 0");

        // Test basic color mixing in Oklab space
        let purple = red.mix_oklab(blue, 0.5);
        // Oklab mixing should produce different results than HSL mixing
        let purple_hsl = red.mix(blue, 0.5);
        assert_ne!(purple.to_hex(), purple_hsl.to_hex());

        // Test factor boundaries (allowing small floating point errors)
        let result_0 = red.mix_oklab(blue, 0.0);
        let result_1 = red.mix_oklab(blue, 1.0);

        // Check that result is close to expected (within 1 color unit per channel)
        let rgb_0: gpui::Rgba = result_0.into_color();
        let rgb_blue: gpui::Rgba = blue.into_color();
        assert!((rgb_0.red - rgb_blue.red).abs() < 0.01);
        assert!((rgb_0.green - rgb_blue.green).abs() < 0.01);
        assert!((rgb_0.blue - rgb_blue.blue).abs() < 0.01);

        let rgb_1: gpui::Rgba = result_1.into_color();
        let rgb_red: gpui::Rgba = red.into_color();
        assert!((rgb_1.red - rgb_red.red).abs() < 0.01);
        assert!((rgb_1.green - rgb_red.green).abs() < 0.01);
        assert!((rgb_1.blue - rgb_red.blue).abs() < 0.01);
    }

    #[test]
    fn test_color_name() {
        assert_eq!(ColorName::Purple.to_string(), "Purple");
        assert_eq!(format!("{}", ColorName::Green), "Green");
        assert_eq!(format!("{:?}", ColorName::Yellow), "Yellow");

        let color = ColorName::Green;
        assert_eq!(color.scale(500).to_hex(), "#21C55E");
        assert_eq!(color.scale(1500).to_hex(), "#21C55E");

        for name in ColorName::all().iter() {
            let name1: ColorName = name.to_string().as_str().try_into().unwrap();
            assert_eq!(name1, *name);
        }
    }

    #[test]
    fn test_h_s_l() {
        let color = hsl(260., 94., 80.);
        assert_hsla_approx_eq(color.hue(200. / 360.), hsl(200., 94., 80.));
        assert_hsla_approx_eq(color.saturation(74. / 100.), hsl(260., 74., 80.));
        assert_hsla_approx_eq(color.lightness(74. / 100.), hsl(260., 94., 74.));
    }

    #[test]
    fn test_try_parse_color() {
        assert_hsla_approx_eq(
            try_parse_color("#F2F200").unwrap(),
            Hsla::new(60.0, 1.0, 0.4745098, 1.0),
        );
        assert_hsla_approx_eq(
            try_parse_color("#00f21888").unwrap(),
            Hsla::new(125.95041, 1.0, 0.4745098, 0.53333336),
        );
        assert_eq!(try_parse_color("black").ok(), Some(crate::black()));
        assert_eq!(try_parse_color("white-800").ok(), Some(crate::white()));
        assert_eq!(try_parse_color("red").ok(), Some(crate::red_500()));
        assert_eq!(try_parse_color("blue-600").ok(), Some(crate::blue_600()));
        assert_eq!(
            try_parse_color("pink/33").ok(),
            Some(crate::pink_500().opacity(0.33))
        );
        assert_eq!(
            try_parse_color("orange-300/66").ok(),
            Some(crate::orange_300().opacity(0.66))
        );
    }
}
