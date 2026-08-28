mod masking;

use one_core::settings::{SqlIndentStyle, SqlKeywordCase};
use sqlformat::{FormatOptions, Indent, QueryParams, format};

use masking::mask_embedded_parameters;

/// SQL 格式化选项
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SqlFormatOptions {
    pub keyword_case: SqlKeywordCase,
    pub indent: SqlIndentStyle,
    /// 自定义嵌入脚本包裹符（起止符对），格式化时整段保护
    pub custom_wrappers: Vec<(String, String)>,
}

impl SqlFormatOptions {
    /// 从用户设置构造格式化选项
    pub fn from_settings(settings: &one_core::settings::SqlFormatSettings) -> Self {
        Self {
            keyword_case: settings.keyword_case,
            indent: settings.indent,
            custom_wrappers: settings.custom_wrapper_pairs(),
        }
    }
}

/// SQL 美化：将 SQL 格式化为可读性更好的多行形式。
/// 默认保持关键字大小写原样，2 空格缩进。
pub fn format_sql(sql: &str) -> String {
    format_sql_with_options(sql, SqlFormatOptions::default())
}

pub fn format_sql_with_options(sql: &str, options: SqlFormatOptions) -> String {
    let (masked_sql, parameters) = mask_embedded_parameters(sql, &options.custom_wrappers);
    let format_options = FormatOptions {
        indent: to_sqlformat_indent(options.indent),
        uppercase: to_sqlformat_uppercase(options.keyword_case),
        ..FormatOptions::default()
    };
    let mut formatted = format(&masked_sql, &QueryParams::None, &format_options);

    for (marker, parameter) in parameters {
        formatted = formatted.replace(&marker, &parameter);
    }

    formatted
}

/// SQL 压缩：将 SQL 压缩为单行形式
pub fn compress_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn to_sqlformat_indent(indent: SqlIndentStyle) -> Indent {
    match indent {
        SqlIndentStyle::TwoSpaces => Indent::Spaces(2),
        SqlIndentStyle::FourSpaces => Indent::Spaces(4),
        SqlIndentStyle::Tabs => Indent::Tabs,
    }
}

/// `None` 表示保持原文大小写，交给 sqlformat 原样输出
fn to_sqlformat_uppercase(keyword_case: SqlKeywordCase) -> Option<bool> {
    match keyword_case {
        SqlKeywordCase::Preserve => None,
        SqlKeywordCase::Upper => Some(true),
        SqlKeywordCase::Lower => Some(false),
    }
}

#[cfg(test)]
mod tests;
