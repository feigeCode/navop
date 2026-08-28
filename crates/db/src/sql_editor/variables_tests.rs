use super::variables::{SqlVariableDeclaration, collect_declarations, expand_variables};

#[test]
fn collect_declarations_finds_simple_set_statements() {
    let document = "\
        @set ids = (1, 2, 3);\n\
        SELECT * FROM users WHERE id IN ${ids};\n\
    ";
    let declarations = collect_declarations(document);
    assert_eq!(1, declarations.len());
    assert_eq!("ids", declarations[0].name);
    assert_eq!("(1, 2, 3)", declarations[0].value);
}

#[test]
fn collect_declarations_handles_multiple_with_whitespace() {
    let document = "\
        @set a = 1;\n\
        @set b = 'x';\n\
        SELECT ${a}, ${b} FROM t;\n\
    ";
    let declarations = collect_declarations(document);
    assert_eq!(2, declarations.len());
    assert_eq!("a", declarations[0].name);
    assert_eq!("b", declarations[1].name);
}

#[test]
fn collect_declarations_ignores_non_set_statements() {
    let document = "SELECT 1; @set foo = 2; SELECT 3;";
    let declarations = collect_declarations(document);
    assert_eq!(1, declarations.len());
    assert_eq!("foo", declarations[0].name);
}

#[test]
fn collect_declarations_ignores_set_in_string_or_comment() {
    let document = "\
        -- @set fake = 1;\n\
        SELECT '@set also_fake = 2;' AS s;\n\
        @set real = 3;\n\
    ";
    let declarations = collect_declarations(document);
    assert_eq!(1, declarations.len());
    assert_eq!("real", declarations[0].name);
}

#[test]
fn expand_variables_replaces_dollar_braced_within_target() {
    let document = "\
        @set ids = (1, 2, 3);\n\
        SELECT * FROM users WHERE id IN ${ids};\n\
    ";
    let range = document.find("SELECT").unwrap()..document.len();
    let expansion = expand_variables(document, range);
    assert_eq!(
        "SELECT * FROM users WHERE id IN (1, 2, 3);\n",
        expansion.target_sql
    );
    assert!(expansion.unresolved.is_empty());
}

#[test]
fn expand_variables_is_case_insensitive_for_names() {
    let document = "\
        @set IDs = (4, 5);\n\
        SELECT * FROM t WHERE id IN ${ids};\n\
    ";
    let range = document.find("SELECT").unwrap()..document.len();
    let expansion = expand_variables(document, range);
    assert_eq!("SELECT * FROM t WHERE id IN (4, 5);\n", expansion.target_sql);
}

#[test]
fn expand_replaces_dollar_bare_parameters() {
    let document = "\
        @set x = 'hello';\n\
        SELECT $x AS greeting;\n\
    ";
    let range = document.find("SELECT").unwrap()..document.len();
    let expansion = expand_variables(document, range);
    assert_eq!("SELECT 'hello' AS greeting;\n", expansion.target_sql);
}

#[test]
fn undeclared_placeholder_is_left_for_parameter_system() {
    let document = "\
        SELECT * FROM t WHERE id = ${unknown};\n\
    ";
    let range = 0..document.len();
    let expansion = expand_variables(document, range);
    assert_eq!(
        "SELECT * FROM t WHERE id = ${unknown};\n",
        expansion.target_sql
    );
    assert_eq!(
        "${unknown}",
        expansion
            .unresolved
            .first()
            .map(|o| &document[o.start..o.end])
            .unwrap_or("")
    );
}

#[test]
fn set_declaration_inside_target_is_removed() {
    let document = "\
        SELECT * FROM t;\n\
        @set y = 7;\n\
        SELECT ${y} + 1;\n\
    ";
    // 选择整个文档，声明在目标内部 -> 应被移除（连同终止分号）。
    let range = 0..document.len();
    let expansion = expand_variables(document, range);
    assert!(!expansion.target_sql.contains("@set"));
    assert_eq!("SELECT * FROM t;\n\nSELECT 7 + 1;\n", expansion.target_sql);
}

#[test]
fn system_double_at_is_not_replaced() {
    let document = "\
        SELECT @@version AS v;\n\
    ";
    let range = 0..document.len();
    let expansion = expand_variables(document, range);
    assert_eq!("SELECT @@version AS v;\n", expansion.target_sql);
}

#[test]
fn placeholder_inside_string_is_not_replaced() {
    let document = "\
        @set id = 5;\n\
        SELECT 'value ${id}' AS v;\n\
    ";
    let range = document.find("SELECT").unwrap()..document.len();
    let expansion = expand_variables(document, range);
    assert_eq!("SELECT 'value ${id}' AS v;\n", expansion.target_sql);
}

#[test]
fn unresolved_placeholder_offsets_are_byte_accurate() {
    let document = "\
        SELECT * FROM t WHERE a = ${x} AND b = '中文' AND c = ${y};\n\
    ";
    let range = 0..document.len();
    let expansion = expand_variables(document, range);
    assert_eq!(2, expansion.unresolved.len());
    for occurrence in &expansion.unresolved {
        let text = &document[occurrence.start..occurrence.end];
        assert!(text.starts_with("${") && text.ends_with('}'));
    }
}

#[test]
fn declaration_value_keeps_parenthesized_list() {
    let doc = "@set ids = (10, 20, 30);\nSELECT 1;\n";
    let declarations = collect_declarations(doc);
    assert_eq!("(10, 20, 30)", declarations[0].value);
    assert_eq!(
        "ids",
        SqlVariableDeclaration {
            name: "ids".to_string(),
            value: "(10, 20, 30)".to_string(),
            range: 0..22,
        }
        .name
    );
}