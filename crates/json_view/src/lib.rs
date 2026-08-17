//! 可复用的 JSON 格式化与树形展示组件。

rust_i18n::i18n!("locales", fallback = "en");

mod json_view;
mod tab_content;

pub use json_view::JsonFormatterView;
