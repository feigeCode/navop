use crate::sql_editor::sql_tokenizer::{SqlKeyword, SqlToken, SqlTokenKind};
/// SQL Symbol Table module for alias tracking
///
/// This module provides alias resolution for SQL queries, tracking
/// table aliases defined in FROM and JOIN clauses.
use std::collections::HashMap;

/// Symbol table for tracking table aliases in SQL queries.
///
/// The symbol table maps aliases to their source table names,
/// supporting nested scopes for subqueries.
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    /// alias -> (table_name, scope_id)
    /// Uses lowercase keys for case-insensitive lookup
    aliases: HashMap<String, (String, usize)>,
    projected_columns: HashMap<String, Vec<String>>,
    /// Current scope depth (0 = top level, increments for subqueries)
    current_scope: usize,
}

impl SymbolTable {
    /// Create a new empty symbol table
    pub fn new() -> Self {
        Self {
            aliases: HashMap::new(),
            projected_columns: HashMap::new(),
            current_scope: 0,
        }
    }

    /// Register a table alias mapping.
    ///
    /// The alias is stored in lowercase for case-insensitive lookup,
    /// but the table name preserves its original case.
    pub fn register_alias(&mut self, alias: &str, table: &str) {
        let key = alias.to_lowercase();
        self.aliases
            .insert(key, (table.to_string(), self.current_scope));
    }

    /// Resolve an alias to its source table name.
    ///
    /// Lookup is case-insensitive.
    /// Returns None if the alias is not found.
    pub fn resolve(&self, alias: &str) -> Option<&str> {
        let key = alias.to_lowercase();
        self.aliases.get(&key).map(|(table, _)| table.as_str())
    }

    pub fn projected_columns(&self, alias: &str) -> Option<&[String]> {
        self.projected_columns
            .get(&alias.to_lowercase())
            .map(Vec::as_slice)
    }

    fn register_projected_columns(&mut self, alias: &str, columns: Vec<String>) {
        if !columns.is_empty() {
            self.projected_columns.insert(alias.to_lowercase(), columns);
        }
    }

    /// Check if an identifier is a known alias.
    ///
    /// Lookup is case-insensitive.
    pub fn is_alias(&self, name: &str) -> bool {
        let key = name.to_lowercase();
        self.aliases.contains_key(&key)
    }

    /// Enter a new scope (e.g., for a subquery).
    ///
    /// Increments the scope depth. Aliases registered after this
    /// will be associated with the new scope.
    pub fn enter_scope(&mut self) {
        self.current_scope += 1;
    }

    /// Exit the current scope.
    ///
    /// Removes all aliases registered in the current scope and
    /// decrements the scope depth.
    pub fn exit_scope(&mut self) {
        if self.current_scope > 0 {
            self.aliases
                .retain(|_, (_, scope)| *scope < self.current_scope);
            self.current_scope -= 1;
        }
    }

    /// Get the current scope depth.
    pub fn current_scope(&self) -> usize {
        self.current_scope
    }

    /// Get all registered aliases (for debugging/testing).
    pub fn all_aliases(&self) -> impl Iterator<Item = (&str, &str)> {
        self.aliases
            .iter()
            .map(|(k, (v, _))| (k.as_str(), v.as_str()))
    }

    /// Get the number of registered aliases.
    pub fn len(&self) -> usize {
        self.aliases.len()
    }

    /// Check if the symbol table is empty.
    pub fn is_empty(&self) -> bool {
        self.aliases.is_empty()
    }
}

impl SymbolTable {
    /// Build a symbol table from a token stream.
    ///
    /// Parses FROM, JOIN, and UPDATE clauses to extract table aliases:
    /// - `FROM table_name alias` -> maps alias to table_name
    /// - `FROM table_name AS alias` -> maps alias to table_name
    /// - `JOIN table_name alias ON ...` -> maps alias to table_name
    /// - `UPDATE table_name alias SET ...` -> maps alias to table_name
    /// - `FROM table_name` (no alias) -> maps table_name to itself
    ///
    /// Handles subqueries by tracking parenthesis depth.
    pub fn build_from_tokens(tokens: &[SqlToken]) -> Self {
        let mut symbol_table = SymbolTable::new();

        // Filter out whitespace and comments for easier parsing
        let meaningful_tokens: Vec<&SqlToken> = tokens
            .iter()
            .filter(|t| {
                !matches!(
                    t.kind,
                    SqlTokenKind::Whitespace
                        | SqlTokenKind::LineComment
                        | SqlTokenKind::BlockComment
                        | SqlTokenKind::Eof
                )
            })
            .collect();

        let mut i = 0;
        let mut paren_depth = 0;

        while i < meaningful_tokens.len() {
            let token = meaningful_tokens[i];

            // Track parenthesis depth for subquery handling
            match token.kind {
                SqlTokenKind::LParen => {
                    paren_depth += 1;
                    i += 1;
                    continue;
                }
                SqlTokenKind::RParen => {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                    }
                    i += 1;
                    continue;
                }
                _ => {}
            }

            // Look for WITH keyword (CTE)
            if token.is_keyword_of(SqlKeyword::With) {
                i += 1;
                // Parse CTE definitions: WITH cte1 AS (...), cte2 AS (...)
                i = Self::parse_cte_definitions(&mut symbol_table, &meaningful_tokens, i);
                continue;
            }

            // Look for FROM, JOIN, or UPDATE keywords
            let is_from = token.is_keyword_of(SqlKeyword::From);
            let is_update = token.is_keyword_of(SqlKeyword::Update);
            let is_join = matches!(
                &token.kind,
                SqlTokenKind::Keyword(SqlKeyword::Join)
                    | SqlTokenKind::Keyword(SqlKeyword::Inner)
                    | SqlTokenKind::Keyword(SqlKeyword::Left)
                    | SqlTokenKind::Keyword(SqlKeyword::Right)
                    | SqlTokenKind::Keyword(SqlKeyword::Full)
                    | SqlTokenKind::Keyword(SqlKeyword::Cross)
            );

            if is_from || is_join || is_update {
                // For JOIN variants (INNER, LEFT, etc.), skip to the actual JOIN keyword
                let mut j = i + 1;
                if is_join && !token.is_keyword_of(SqlKeyword::Join) {
                    // Skip until we find JOIN
                    while j < meaningful_tokens.len() {
                        if meaningful_tokens[j].is_keyword_of(SqlKeyword::Join) {
                            j += 1;
                            break;
                        }
                        j += 1;
                    }
                }

                // Parse table references after FROM/JOIN
                i = Self::parse_table_references(&mut symbol_table, &meaningful_tokens, j);
            } else {
                i += 1;
            }
        }

        symbol_table
    }

    /// Parse table references starting at position `start`.
    /// Returns the next position to continue parsing.
    fn parse_table_references(
        symbol_table: &mut SymbolTable,
        tokens: &[&SqlToken],
        start: usize,
    ) -> usize {
        let mut i = start;

        while i < tokens.len() {
            // Skip subqueries (parenthesized expressions)
            if matches!(tokens[i].kind, SqlTokenKind::LParen) {
                let projection_start = i + 1;
                let mut depth = 1;
                i += 1;
                while i < tokens.len() && depth > 0 {
                    match tokens[i].kind {
                        SqlTokenKind::LParen => depth += 1,
                        SqlTokenKind::RParen => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
                let projection_end = i.saturating_sub(1);
                let projected_columns =
                    Self::projection_columns(&tokens[projection_start..projection_end]);
                // After subquery, check for alias
                if i < tokens.len() {
                    // Skip AS if present
                    if tokens[i].is_keyword_of(SqlKeyword::As) {
                        i += 1;
                    }
                    // Next should be alias (identifier)
                    if i < tokens.len() && matches!(tokens[i].kind, SqlTokenKind::Ident) {
                        // 子查询别名 - 注册为特殊标记 "#subquery"
                        // 这样补全代码可以识别它是子查询而不是普通表
                        let alias = Self::extract_ident_text(tokens[i]);
                        symbol_table.register_alias(&alias, "#subquery");
                        symbol_table.register_projected_columns(&alias, projected_columns);
                        i += 1;
                    }
                }
                continue;
            }

            // Expect table name (identifier)
            if !matches!(
                tokens[i].kind,
                SqlTokenKind::Ident | SqlTokenKind::QuotedIdent
            ) {
                break;
            }

            let table_name = Self::extract_ident_text(tokens[i]);
            i += 1;

            // Check for schema.table pattern
            if i < tokens.len() && matches!(tokens[i].kind, SqlTokenKind::Dot) {
                i += 1; // skip dot
                if i < tokens.len()
                    && matches!(
                        tokens[i].kind,
                        SqlTokenKind::Ident | SqlTokenKind::QuotedIdent
                    )
                {
                    // Use the actual table name (after dot)
                    let actual_table = Self::extract_ident_text(tokens[i]);
                    i += 1;
                    // Continue with actual_table as the table name
                    i = Self::parse_alias(symbol_table, tokens, i, &actual_table);
                } else {
                    // Malformed, register what we have
                    symbol_table.register_alias(&table_name, &table_name);
                }
            } else {
                // Simple table name, check for alias
                i = Self::parse_alias(symbol_table, tokens, i, &table_name);
            }

            // Check for comma (multiple tables in FROM)
            if i < tokens.len() && matches!(tokens[i].kind, SqlTokenKind::Comma) {
                i += 1;
                continue;
            }

            // Stop at keywords that end the table list
            if i < tokens.len() {
                let is_terminator = matches!(
                    &tokens[i].kind,
                    SqlTokenKind::Keyword(SqlKeyword::Where)
                        | SqlTokenKind::Keyword(SqlKeyword::On)
                        | SqlTokenKind::Keyword(SqlKeyword::Group)
                        | SqlTokenKind::Keyword(SqlKeyword::Order)
                        | SqlTokenKind::Keyword(SqlKeyword::Having)
                        | SqlTokenKind::Keyword(SqlKeyword::Limit)
                        | SqlTokenKind::Keyword(SqlKeyword::Union)
                        | SqlTokenKind::Keyword(SqlKeyword::Join)
                        | SqlTokenKind::Keyword(SqlKeyword::Inner)
                        | SqlTokenKind::Keyword(SqlKeyword::Left)
                        | SqlTokenKind::Keyword(SqlKeyword::Right)
                        | SqlTokenKind::Keyword(SqlKeyword::Full)
                        | SqlTokenKind::Keyword(SqlKeyword::Cross)
                        | SqlTokenKind::Keyword(SqlKeyword::Select)
                );
                if is_terminator {
                    break;
                }
            }

            break;
        }

        i
    }

    /// Parse optional alias after table name.
    /// Returns the next position to continue parsing.
    fn parse_alias(
        symbol_table: &mut SymbolTable,
        tokens: &[&SqlToken],
        start: usize,
        table_name: &str,
    ) -> usize {
        let mut i = start;

        // Check for AS keyword
        let has_as = i < tokens.len() && tokens[i].is_keyword_of(SqlKeyword::As);
        if has_as {
            i += 1;
        }

        // Check for alias (identifier that's not a keyword)
        if i < tokens.len() && matches!(tokens[i].kind, SqlTokenKind::Ident) {
            let alias = Self::extract_ident_text(tokens[i]);
            // 只有当别名不存在时才注册（避免覆盖 CTE 定义）
            if !symbol_table.is_alias(&alias) {
                symbol_table.register_alias(&alias, table_name);
            }
            i += 1;
        } else if !has_as {
            // No alias, map table name to itself
            // 但如果表名已经是 CTE 或子查询，不要覆盖
            if !symbol_table.is_alias(table_name) {
                symbol_table.register_alias(table_name, table_name);
            }
        }

        i
    }

    /// Parse CTE definitions after WITH keyword.
    /// Format: WITH cte1 AS (...), cte2 AS (...) SELECT ...
    /// Returns the next position to continue parsing.
    fn parse_cte_definitions(
        symbol_table: &mut SymbolTable,
        tokens: &[&SqlToken],
        start: usize,
    ) -> usize {
        let mut i = start;

        loop {
            // Expect CTE name (identifier)
            if i >= tokens.len()
                || !matches!(
                    tokens[i].kind,
                    SqlTokenKind::Ident | SqlTokenKind::QuotedIdent
                )
            {
                break;
            }

            let cte_name = Self::extract_ident_text(tokens[i]);
            i += 1;

            // Expect AS keyword
            if i >= tokens.len() || !tokens[i].is_keyword_of(SqlKeyword::As) {
                // 注册 CTE 名称（即使没有完整定义）
                symbol_table.register_alias(&cte_name, "#cte");
                break;
            }
            i += 1;

            // Expect opening parenthesis
            if i >= tokens.len() || !matches!(tokens[i].kind, SqlTokenKind::LParen) {
                symbol_table.register_alias(&cte_name, "#cte");
                break;
            }

            // Skip the CTE subquery (parenthesized expression)
            let projection_start = i + 1;
            let mut depth = 1;
            i += 1;
            while i < tokens.len() && depth > 0 {
                match tokens[i].kind {
                    SqlTokenKind::LParen => depth += 1,
                    SqlTokenKind::RParen => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            let projection_end = i.saturating_sub(1);
            let projected_columns =
                Self::projection_columns(&tokens[projection_start..projection_end]);

            // 注册 CTE 名称为 "#cte" 标记
            symbol_table.register_alias(&cte_name, "#cte");
            symbol_table.register_projected_columns(&cte_name, projected_columns);

            // Check for comma (multiple CTEs)
            if i < tokens.len() && matches!(tokens[i].kind, SqlTokenKind::Comma) {
                i += 1;
                continue;
            }

            // End of CTE definitions
            break;
        }

        i
    }

    /// Extract identifier text, handling quoted identifiers.
    fn extract_ident_text(token: &SqlToken) -> String {
        match token.kind {
            SqlTokenKind::QuotedIdent => {
                let s = &token.text;
                if let Some(body) = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    body.replace("\"\"", "\"")
                } else if let Some(body) = s.strip_prefix('`').and_then(|s| s.strip_suffix('`')) {
                    body.replace("``", "`")
                } else if let Some(body) = s.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                    body.replace("]]", "]")
                } else {
                    s.clone()
                }
            }
            _ => token.text.clone(),
        }
    }

    fn projection_columns(tokens: &[&SqlToken]) -> Vec<String> {
        let Some(select_index) = tokens
            .iter()
            .position(|token| token.is_keyword_of(SqlKeyword::Select))
        else {
            return Vec::new();
        };
        let mut columns = Vec::new();
        let mut segment_start = select_index + 1;
        let mut depth = 0usize;
        for index in segment_start..=tokens.len() {
            let at_end = index == tokens.len();
            if !at_end {
                match tokens[index].kind {
                    SqlTokenKind::LParen => depth += 1,
                    SqlTokenKind::RParen => depth = depth.saturating_sub(1),
                    SqlTokenKind::Keyword(SqlKeyword::From) if depth == 0 => {
                        Self::push_projection_column(&tokens[segment_start..index], &mut columns);
                        break;
                    }
                    SqlTokenKind::Comma if depth == 0 => {
                        Self::push_projection_column(&tokens[segment_start..index], &mut columns);
                        segment_start = index + 1;
                    }
                    _ => {}
                }
            } else {
                Self::push_projection_column(&tokens[segment_start..index], &mut columns);
            }
        }
        columns
    }

    fn push_projection_column(tokens: &[&SqlToken], columns: &mut Vec<String>) {
        if tokens.is_empty() || tokens.iter().any(|token| token.text == "*") {
            return;
        }
        if let Some(as_index) = tokens
            .iter()
            .rposition(|token| token.is_keyword_of(SqlKeyword::As))
        {
            if let Some(alias) = tokens.get(as_index + 1) {
                if matches!(alias.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent) {
                    columns.push(Self::extract_ident_text(alias));
                }
            }
            return;
        }
        if let Some(last) = tokens
            .iter()
            .rev()
            .find(|token| matches!(token.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent))
        {
            columns.push(Self::extract_ident_text(last));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql_editor::sql_tokenizer::SqlTokenizer;

    #[test]
    fn test_simple_from_with_alias() {
        let sql = "SELECT u.id FROM users u";
        let mut tokenizer = SqlTokenizer::new(sql);
        let tokens = tokenizer.tokenize();
        let st = SymbolTable::build_from_tokens(&tokens);

        assert_eq!(st.resolve("u"), Some("users"));
        assert!(st.is_alias("u"));
        assert!(st.is_alias("U")); // case-insensitive
    }

    #[test]
    fn records_cte_and_derived_table_projection_columns() {
        let sql = "WITH recent AS (SELECT id, total AS amount FROM orders) \
                   SELECT r.id, d.label FROM recent r \
                   JOIN (SELECT user_id, name label FROM users) d ON d.user_id = r.id";
        let tokens = SqlTokenizer::new(sql).tokenize();
        let symbols = SymbolTable::build_from_tokens(&tokens);

        assert_eq!(
            Some(["id".to_string(), "amount".to_string()].as_slice()),
            symbols.projected_columns("recent")
        );
        assert_eq!(
            Some(["user_id".to_string(), "label".to_string()].as_slice()),
            symbols.projected_columns("d")
        );
    }

    #[test]
    fn test_from_with_as_keyword() {
        let sql = "SELECT u.id FROM users AS u";
        let mut tokenizer = SqlTokenizer::new(sql);
        let tokens = tokenizer.tokenize();
        let st = SymbolTable::build_from_tokens(&tokens);

        assert_eq!(st.resolve("u"), Some("users"));
    }

    #[test]
    fn test_from_without_alias() {
        let sql = "SELECT id FROM users";
        let mut tokenizer = SqlTokenizer::new(sql);
        let tokens = tokenizer.tokenize();
        let st = SymbolTable::build_from_tokens(&tokens);

        assert_eq!(st.resolve("users"), Some("users"));
    }

    #[test]
    fn test_join_with_alias() {
        let sql = "SELECT u.id FROM users u JOIN departments d ON u.dept_id = d.id";
        let mut tokenizer = SqlTokenizer::new(sql);
        let tokens = tokenizer.tokenize();
        let st = SymbolTable::build_from_tokens(&tokens);

        assert_eq!(st.resolve("u"), Some("users"));
        assert_eq!(st.resolve("d"), Some("departments"));
    }

    #[test]
    fn test_left_join() {
        let sql = "SELECT * FROM users u LEFT JOIN orders o ON u.id = o.user_id";
        let mut tokenizer = SqlTokenizer::new(sql);
        let tokens = tokenizer.tokenize();
        let st = SymbolTable::build_from_tokens(&tokens);

        assert_eq!(st.resolve("u"), Some("users"));
        assert_eq!(st.resolve("o"), Some("orders"));
    }

    #[test]
    fn test_multiple_tables_comma() {
        let sql = "SELECT * FROM users u, orders o WHERE u.id = o.user_id";
        let mut tokenizer = SqlTokenizer::new(sql);
        let tokens = tokenizer.tokenize();
        let st = SymbolTable::build_from_tokens(&tokens);

        assert_eq!(st.resolve("u"), Some("users"));
        assert_eq!(st.resolve("o"), Some("orders"));
    }

    #[test]
    fn test_schema_qualified_table() {
        let sql = "SELECT * FROM public.users u";
        let mut tokenizer = SqlTokenizer::new(sql);
        let tokens = tokenizer.tokenize();
        let st = SymbolTable::build_from_tokens(&tokens);

        assert_eq!(st.resolve("u"), Some("users"));
    }

    #[test]
    fn test_scope_management() {
        let mut st = SymbolTable::new();
        st.register_alias("u", "users");

        st.enter_scope();
        st.register_alias("o", "orders");

        assert_eq!(st.resolve("u"), Some("users"));
        assert_eq!(st.resolve("o"), Some("orders"));

        st.exit_scope();

        assert_eq!(st.resolve("u"), Some("users"));
        assert_eq!(st.resolve("o"), None); // removed with scope
    }
}
