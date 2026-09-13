use gpui::Hsla;

/// 调色扩展：CE fork 暴露的 `Colorize` trait（`mix`、`lighten`、`darken`）
/// 在 gpui-pre 里没有了。这里最小化复活 `mix`。
pub trait ColorMix {
    fn mix(&self, other: Self, factor: f32) -> Self;
}

impl ColorMix for Hsla {
    fn mix(&self, other: Self, factor: f32) -> Self {
        let f = factor.clamp(0.0, 1.0);
        Self {
            h: self.h + (other.h - self.h) * f,
            s: self.s + (other.s - self.s) * f,
            l: self.l + (other.l - self.l) * f,
            a: self.a + (other.a - self.a) * f,
        }
    }
}
