//! LaTeX display-math parsing and RaTeX SVG rendering helpers.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use anyhow::{Context as _, anyhow};
use directories::ProjectDirs;
use gpui::{Hsla, Rgba};

const DISPLAY_MATH_SCALE: f32 = 1.25;
const INLINE_MATH_SCALE: f32 = 1.12;

/// Parsed display-math source preserved from Markdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DisplayMathSource {
    /// Full Markdown source, including `$$` delimiters.
    pub(crate) raw: String,
    /// LaTeX body between the display delimiters.
    pub(crate) body: String,
}

/// Result of rendering display math into an SVG cache file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LatexSvgRender {
    /// Path to the SVG file consumed by GPUI's image element.
    pub(crate) path: PathBuf,
    /// SVG document content, used by export paths.
    pub(crate) svg: String,
}

/// Parse a raw `$$...$$` Markdown block into the LaTeX body it contains.
pub(crate) fn parse_display_math_source(raw: &str) -> Option<DisplayMathSource> {
    let raw = raw.trim_matches('\n').to_string();
    let lines = raw.split('\n').collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    if lines.len() == 1 {
        let line = strip_display_indent(lines[0])?.trim_end();
        let body_and_close = line.strip_prefix("$$")?;
        let close = body_and_close.find("$$")?;
        let body = body_and_close[..close].trim().to_string();
        return Some(DisplayMathSource { raw, body });
    }

    let opener = strip_display_indent(lines[0])?.trim_end();
    let closer = lines.last()?.trim();
    if opener != "$$" || closer != "$$" {
        return None;
    }

    let body = lines[1..lines.len() - 1].join("\n");
    Some(DisplayMathSource { raw, body })
}

/// Display font size used for rendered display-math blocks.
pub(crate) fn display_math_font_size(base_font_size: f32) -> f32 {
    base_font_size * DISPLAY_MATH_SCALE
}

/// Display font size used for rendered inline math.
pub(crate) fn inline_math_font_size(base_font_size: f32) -> f32 {
    base_font_size * INLINE_MATH_SCALE
}

/// Render a display-math source into a cached SVG file.
pub(crate) fn render_display_math_svg(
    source: &DisplayMathSource,
    text_color: Hsla,
    font_size: f32,
) -> anyhow::Result<LatexSvgRender> {
    render_latex_svg_to_cache(&source.body, text_color, font_size)
}

/// Render an inline LaTeX body into a cached SVG file.
pub(crate) fn render_inline_math_svg(
    latex: &str,
    text_color: Hsla,
    font_size: f32,
) -> anyhow::Result<LatexSvgRender> {
    render_latex_svg_to_cache(latex, text_color, font_size)
}

fn render_latex_svg_to_cache(
    latex: &str,
    text_color: Hsla,
    font_size: f32,
) -> anyhow::Result<LatexSvgRender> {
    let key = latex_cache_key(latex, text_color, font_size);
    let path = latex_cache_dir()?.join(format!("{key}.svg"));
    if path.exists() {
        let svg = fs::read_to_string(&path)
            .with_context(|| format!("failed to read LaTeX SVG cache '{}'", path.display()))?;
        return Ok(LatexSvgRender { path, svg });
    }

    let svg = render_latex_to_svg(latex, text_color, font_size)?;
    fs::write(&path, &svg)
        .with_context(|| format!("failed to write LaTeX SVG cache '{}'", path.display()))?;
    Ok(LatexSvgRender { path, svg })
}

/// Render a LaTeX expression into self-contained SVG text.
pub(crate) fn render_latex_to_svg(
    latex: &str,
    text_color: Hsla,
    font_size: f32,
) -> anyhow::Result<String> {
    let parsed = ratex_parser::parse(latex).map_err(|err| anyhow!("{err}"))?;
    let layout = ratex_layout::layout(&parsed, &ratex_layout::LayoutOptions::default());
    let display_list = ratex_layout::to_display_list(&layout);
    let mut svg = ratex_svg::render_to_svg(
        &display_list,
        &ratex_svg::SvgOptions {
            font_size: f64::from(font_size.max(1.0)),
            padding: f64::from((font_size * 0.35).max(4.0)),
            embed_glyphs: true,
            ..ratex_svg::SvgOptions::default()
        },
    );
    svg = recolor_default_black(&svg, &svg_color(text_color));
    Ok(svg)
}

/// Stable cache key for formula content and visual parameters.
pub(crate) fn latex_cache_key(latex: &str, text_color: Hsla, font_size: f32) -> String {
    let mut hasher = DefaultHasher::new();
    latex.hash(&mut hasher);
    svg_color(text_color).hash(&mut hasher);
    font_size.to_bits().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn strip_display_indent(line: &str) -> Option<&str> {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    (indent <= 3).then_some(&line[indent..])
}

fn latex_cache_dir() -> anyhow::Result<PathBuf> {
    let root = ProjectDirs::from("com", "manyougz", "Velotype")
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("Velotype"));
    let dir = root.join("latex-svg");
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create LaTeX SVG cache '{}'", dir.display()))?;
    Ok(dir)
}

fn svg_color(color: Hsla) -> String {
    let color = Rgba::from(color);
    format!(
        "rgba({},{},{},{})",
        color_channel(color.r),
        color_channel(color.g),
        color_channel(color.b),
        trim_float(f64::from(color.a.clamp(0.0, 1.0)))
    )
}

fn color_channel(channel: f32) -> u8 {
    (channel.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn trim_float(value: f64) -> String {
    let formatted = format!("{value:.3}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn recolor_default_black(svg: &str, color: &str) -> String {
    svg.replace("rgba(0,0,0,1)", color)
        .replace("rgba(0, 0, 0, 1)", color)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgba;

    #[test]
    fn parses_single_line_display_math() {
        let parsed = parse_display_math_source("$$x^2$$").expect("display math");
        assert_eq!(parsed.body, "x^2");
        assert_eq!(parsed.raw, "$$x^2$$");
    }

    #[test]
    fn parses_multiline_display_math() {
        let parsed = parse_display_math_source("$$\n\\int_0^1 x^2 dx\n$$").expect("display math");
        assert_eq!(parsed.body, "\\int_0^1 x^2 dx");
    }

    #[test]
    fn rejects_unclosed_display_math() {
        assert!(parse_display_math_source("$$\n\\frac{1}{2}").is_none());
    }

    #[test]
    fn cache_key_changes_with_theme_inputs() {
        let first = latex_cache_key("\\frac{1}{2}", Hsla::from(rgba(0xffffffff)), 18.0);
        let second = latex_cache_key("\\frac{1}{2}", Hsla::from(rgba(0x000000ff)), 18.0);
        assert_ne!(first, second);
    }

    #[test]
    fn display_math_font_size_scales_base_text_size() {
        assert_eq!(display_math_font_size(20.0), 25.0);
    }

    #[test]
    fn inline_math_font_size_scales_base_text_size() {
        assert!((inline_math_font_size(20.0) - 22.4).abs() < 0.001);
    }

    #[test]
    fn renders_basic_formula_svg() {
        let svg =
            render_latex_to_svg("\\frac{1}{2}", Hsla::from(rgba(0xffffffff)), 18.0).expect("svg");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }

    #[test]
    fn invalid_latex_returns_error() {
        assert!(render_latex_to_svg("\\frac{a}", Hsla::from(rgba(0xffffffff)), 18.0).is_err());
    }
}
