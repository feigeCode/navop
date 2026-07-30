//! Code-block language resolution and host/WASM highlight spans.

use std::ops::Range;

use crate::CodeHighlightStyle;

/// Canonical language key used by the syntax-highlighting registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CodeLanguageKey {
    /// Rust source code.
    Rust,
    /// JavaScript without JSX.
    JavaScript,
    /// JavaScript with JSX syntax.
    JavaScriptJsx,
    /// TypeScript without TSX.
    TypeScript,
    /// TypeScript with TSX syntax.
    TypeScriptTsx,
    /// JSON data.
    Json,
    /// Markdown source.
    Markdown,
    /// POSIX-like shell scripts.
    Bash,
    /// C source code.
    C,
    /// C++ source code.
    Cpp,
    /// C# source code.
    CSharp,
    /// CSS stylesheets.
    Css,
    /// Go source code.
    Go,
    /// HTML markup.
    Html,
    /// Java source code.
    Java,
    /// PHP source code.
    Php,
    /// Python source code.
    Python,
    /// Ruby source code.
    Ruby,
    /// YAML configuration.
    Yaml,
    /// TOML configuration.
    Toml,
    /// Mermaid diagram source.
    Mermaid,
    /// Plain text or unknown language fallback.
    PlainText,
}

/// Highlighted byte range inside a code block.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CodeHighlightPaint {
    Host(CodeHighlightStyle),
}

/// Highlighted byte range inside a code block.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CodeHighlightSpan {
    pub(crate) range: Range<usize>,
    pub(crate) paint: CodeHighlightPaint,
}

/// Highlight result cached on a code block.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CodeHighlightResult {
    pub(crate) language: CodeLanguageKey,
    pub(crate) spans: Vec<CodeHighlightSpan>,
}

/// Language aliases accepted from fenced-code info strings.
#[derive(Clone, Copy)]
struct LanguageDescriptor {
    key: CodeLanguageKey,
    aliases: &'static [&'static str],
}

const LANGUAGE_DESCRIPTORS: &[LanguageDescriptor] = &[
    LanguageDescriptor {
        key: CodeLanguageKey::Rust,
        aliases: &["rust", "rs"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::JavaScript,
        aliases: &["javascript", "js"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::JavaScriptJsx,
        aliases: &["jsx"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::TypeScript,
        aliases: &["typescript", "ts"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::TypeScriptTsx,
        aliases: &["tsx"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Json,
        aliases: &["json"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Markdown,
        aliases: &["markdown", "md"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Bash,
        aliases: &["bash", "sh", "shell", "zsh"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::C,
        aliases: &["c", "h"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Cpp,
        aliases: &["cpp", "cxx", "cc", "hpp", "hxx"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::CSharp,
        aliases: &["csharp", "cs", "c#"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Css,
        aliases: &["css"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Go,
        aliases: &["go", "golang"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Html,
        aliases: &["html"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Java,
        aliases: &["java"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Php,
        aliases: &["php"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Python,
        aliases: &["python", "py"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Ruby,
        aliases: &["ruby", "rb"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Yaml,
        aliases: &["yaml", "yml"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Toml,
        aliases: &["toml"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::PlainText,
        aliases: &["text", "txt", "plain"],
    },
    LanguageDescriptor {
        key: CodeLanguageKey::Mermaid,
        aliases: &["mermaid"],
    },
];

fn descriptor_for_language(language: &str) -> Option<&'static LanguageDescriptor> {
    LANGUAGE_DESCRIPTORS.iter().find(|descriptor| {
        descriptor
            .aliases
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(language))
    })
}

pub(crate) fn resolve_code_language_key(language: Option<&str>) -> Option<CodeLanguageKey> {
    let normalized = language?
        .split_whitespace()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    descriptor_for_language(normalized).map(|descriptor| descriptor.key)
}

#[cfg(test)]
mod tests {
    use super::{CodeLanguageKey, resolve_code_language_key};

    #[test]
    fn language_aliases_resolve_to_expected_keys() {
        let cases = [
            ("rust", CodeLanguageKey::Rust),
            ("rs", CodeLanguageKey::Rust),
            ("js", CodeLanguageKey::JavaScript),
            ("jsx", CodeLanguageKey::JavaScriptJsx),
            ("ts", CodeLanguageKey::TypeScript),
            ("tsx", CodeLanguageKey::TypeScriptTsx),
            ("json", CodeLanguageKey::Json),
            ("md", CodeLanguageKey::Markdown),
            ("sh", CodeLanguageKey::Bash),
            ("hpp", CodeLanguageKey::Cpp),
            ("c#", CodeLanguageKey::CSharp),
            ("golang", CodeLanguageKey::Go),
            ("html", CodeLanguageKey::Html),
            ("py", CodeLanguageKey::Python),
            ("rb", CodeLanguageKey::Ruby),
            ("yml", CodeLanguageKey::Yaml),
            ("toml", CodeLanguageKey::Toml),
            ("plain", CodeLanguageKey::PlainText),
            ("mermaid", CodeLanguageKey::Mermaid),
        ];

        for (alias, expected) in cases {
            assert_eq!(resolve_code_language_key(Some(alias)), Some(expected));
        }
        assert_eq!(
            resolve_code_language_key(Some("  rust title=demo  ")),
            Some(CodeLanguageKey::Rust)
        );
        assert_eq!(resolve_code_language_key(Some("unknown")), None);
        assert_eq!(resolve_code_language_key(None), None);
    }
}
