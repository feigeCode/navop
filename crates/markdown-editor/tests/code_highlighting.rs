use gpui_component::highlighter::{HighlightTheme, LanguageRegistry, SyntaxHighlighter};
use ropey::Rope;

#[test]
fn fenced_rust_code_uses_the_input_syntax_highlighter() {
    let rust = LanguageRegistry::singleton()
        .language("rust")
        .expect("markdown fenced code languages must be compiled into the editor");
    assert!(
        !rust.highlights.is_empty(),
        "the registered Rust grammar must include highlight queries"
    );

    let source = "fn main() {\n    let answer = 42;\n}\n";
    let text = Rope::from_str(source);
    let mut highlighter = SyntaxHighlighter::new("rust");
    assert_eq!("rust", highlighter.language().as_ref());
    assert!(highlighter.update(None, &text, None));

    let styles = highlighter.styles(&(0..source.len()), &HighlightTheme::default_dark());
    assert!(
        styles.iter().any(|(_, style)| style.color.is_some()),
        "Rust source must produce at least one colored syntax span"
    );
}
