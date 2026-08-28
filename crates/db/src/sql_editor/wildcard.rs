//! SELECT * 展开纯算法
//!
//! 把 `SELECT *` / `SELECT u.*` 展开为显式列清单。纯算法：不接触 UI，也不
//! 直接依赖数据库。列元数据通过 [`SqlWildcardMetadata`] 抽象注入，测试可用
//! fake 实现。
//!
//! 支持：
//! - 单表 `SELECT *`
//! - alias `SELECT u.*`
//! - 多表（逗号 / JOIN）按 FROM 顺序展开
//! - 同名列按 qualifier 配置加表前缀
//! - CTE / derived table projection 列
//! - metadata 不完整时 fail closed（返回 [`WildcardExpansionError`]）
//! - 保留 quoted identifier
//! - apply 时校验 range 内容未变化（stale 拒绝）

use super::sql_tokenizer::{SqlKeyword, SqlToken, SqlTokenKind, SqlTokenizer};
pub use super::statement_ranges::SqlTextRange;

/// 一个被引用的表对象。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlObjectRef {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
}

/// 一次展开的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlWildcardExpansion {
    /// 待替换的 `*` 的字节范围（相对语句起始的 base）。
    pub range: SqlTextRange,
    /// 替换文本（展开后的列清单）。
    pub replacement: String,
    /// 展开所需引用的表对象。
    pub required_tables: Vec<SqlObjectRef>,
}

/// 展开错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WildcardExpansionError {
    /// 语句中没有可展开的 `*`。
    NoWildcard,
    /// 元数据不完整（无法确定某张表的列），fail closed。
    MetadataIncomplete,
    /// 源表无法唯一确定（ambiguous）。
    AmbiguousSource,
    /// apply 时发现 range 内容已变化（stale）。
    StaleSource,
}

/// 列元数据抽象。
pub trait SqlWildcardMetadata {
    /// 返回某表对象的列名列表；`None` 表示元数据不完整。
    fn columns(&self, object: &SqlObjectRef) -> Option<Vec<String>>;
}

/// 同名列处理方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlWildcardQualifier {
    /// 不加前缀（默认）。
    None,
    /// 始终加表名 / alias 前缀。
    Always,
    /// 仅当列名重复时加前缀。
    OnConflict,
}

/// 解析并计算一次 SELECT 展开。
///
/// `statement` 为单个语句文本（无分隔符），`base_byte` 为其在文档中的起始
/// 字节（用于输出绝对范围）。`metadata` 提供列信息。
pub fn expand_wildcard(
    statement: &str,
    base_byte: usize,
    metadata: &dyn SqlWildcardMetadata,
    qualifier: SqlWildcardQualifier,
) -> Result<SqlWildcardExpansion, WildcardExpansionError> {
    let tokens = SqlTokenizer::new(statement).tokenize();

    // 顶层 SELECT 与 FROM。
    let (select_index, from_index) = find_select_from(&tokens)?;
    if from_index.is_none() {
        // SELECT 没有 FROM：`SELECT *` 无源，无法展开（返回 NoWildcard）。
        return Err(WildcardExpansionError::NoWildcard);
    }
    let from_index = from_index.unwrap();

    // WITH CTE 定义（在 SELECT 之前）。
    let cte_definitions = parse_cte_definitions(&tokens, select_index);

    // 解析 FROM 源表。
    let sources = parse_from_sources(&tokens, from_index)?;

    // 查找 SELECT 列表中的 *。
    let wildcards = find_wildcards(&tokens, select_index, from_index);

    if wildcards.is_empty() {
        return Err(WildcardExpansionError::NoWildcard);
    }

    // 只处理第一个 *（每次展开一个）。
    let (star_range, qualifier_text) = &wildcards[0];
    let resolved = resolve_source(qualifier_text.as_deref(), &sources, &cte_definitions)?;

    let columns = match &resolved {
        ResolvedSource::Table(object) => metadata
            .columns(object)
            .ok_or(WildcardExpansionError::MetadataIncomplete)?,
        ResolvedSource::Projection(columns) => columns.clone(),
        ResolvedSource::Unknown => return Err(WildcardExpansionError::MetadataIncomplete),
    };

    let all_names = all_used_aliases(&sources);
    let replacement = build_column_list(
        &columns,
        qualifier_text.as_deref().or(resolved.table_name()),
        &all_names,
        qualifier,
    );

    let mut required_tables = Vec::new();
    if let ResolvedSource::Table(object) = &resolved {
        required_tables.push(object.clone());
    }

    Ok(SqlWildcardExpansion {
        range: SqlTextRange {
            start_byte: base_byte + star_range.start_byte,
            end_byte: base_byte + star_range.end_byte,
        },
        replacement,
        required_tables,
    })
}

/// 校验 `document` 中 expansion 覆盖的 range 内容与展开时一致，然后应用。
///
/// 用于在真正替换前防御 stale（文档 revision 变化）。
pub fn apply_wildcard_expansion(
    document: &str,
    expansion: &SqlWildcardExpansion,
    expected_wildcard: &str,
) -> Result<String, WildcardExpansionError> {
    let range = expansion.range.to_range();
    if range.end > document.len() {
        return Err(WildcardExpansionError::StaleSource);
    }
    if &document[range.clone()] != expected_wildcard {
        return Err(WildcardExpansionError::StaleSource);
    }
    let mut result = document.to_string();
    result.replace_range(range, &expansion.replacement);
    Ok(result)
}

#[derive(Clone, Debug)]
enum ResolvedSource {
    Table(SqlObjectRef),
    Projection(Vec<String>),
    Unknown,
}

impl ResolvedSource {
    fn table_name(&self) -> Option<&str> {
        match self {
            ResolvedSource::Table(object) => Some(&object.name),
            _ => None,
        }
    }
}

/// 一个 FROM 源：表或 CTE/derived。
#[derive(Clone, Debug)]
struct FromSource {
    object: Option<SqlObjectRef>,
    projection: Option<Vec<String>>,
    /// alias（AS 或裸 alias；无则 None）。
    alias: Option<String>,
}

impl FromSource {
    fn table(&self) -> Option<&SqlObjectRef> {
        self.object.as_ref()
    }
}

/// 查找顶层 SELECT 与 FROM 关键字下标。
fn find_select_from(tokens: &[SqlToken]) -> Result<(usize, Option<usize>), WildcardExpansionError> {
    let mut depth = 0usize;
    let mut select_index = None;
    let mut from_index = None;
    for (index, token) in tokens.iter().enumerate() {
        match &token.kind {
            SqlTokenKind::LParen => depth += 1,
            SqlTokenKind::RParen => depth = depth.saturating_sub(1),
            SqlTokenKind::Keyword(kw) if depth == 0 => {
                if select_index.is_none() {
                    if *kw == SqlKeyword::Select {
                        select_index = Some(index);
                    }
                    // WITH ... SELECT 前跳过的 CTE 内容。
                    if *kw == SqlKeyword::With {
                        continue;
                    }
                }
                if select_index.is_some() && *kw == SqlKeyword::From && depth == 0 {
                    from_index = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    select_index
        .map(|index| (index, from_index))
        .ok_or(WildcardExpansionError::NoWildcard)
}

/// 解析 WITH 中的 CTE 定义：`cte_name AS (SELECT ...)`。
/// 返回 (名字小写化, projection 列名)。
fn parse_cte_definitions(
    tokens: &[SqlToken],
    select_index: usize,
) -> Vec<(String, Vec<String>)> {
    let mut definitions = Vec::new();
    let mut index = 0usize;
    while index < select_index {
        let token = &tokens[index];
        if !matches!(token.kind, SqlTokenKind::Keyword(SqlKeyword::With)) {
            index += 1;
            continue;
        }
        // 解析 CTE 列表直到 SELECT。
        let mut position = index + 1;
        while position < select_index {
            // 找到名字（跳过空白）。
            let Some(name_token) = tokens[position..]
                .iter()
                .find(|token| !matches!(token.kind, SqlTokenKind::Whitespace))
            else {
                break;
            };
            let name_index = tokens
                .iter()
                .position(|token| std::ptr::eq(token, name_token))
                .unwrap_or(position);
            if !matches!(name_token.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent) {
                position = name_index + 1;
                continue;
            }
            let name = unquote_identifier(&name_token.text);
            // 期望 `AS (`（跳过空白）。
            let mut cursor = name_index + 1;
            while cursor < select_index
                && matches!(tokens[cursor].kind, SqlTokenKind::Whitespace)
            {
                cursor += 1;
            }
            let Some(after_name) = tokens.get(cursor) else {
                break;
            };
            if !matches!(after_name.kind, SqlTokenKind::Keyword(SqlKeyword::As)) {
                position = cursor + 1;
                continue;
            }
            cursor += 1;
            while cursor < select_index
                && matches!(tokens[cursor].kind, SqlTokenKind::Whitespace)
            {
                cursor += 1;
            }
            let Some(open) = tokens.get(cursor) else {
                break;
            };
            if !matches!(open.kind, SqlTokenKind::LParen) {
                position = cursor + 1;
                continue;
            }
            // 提取 projection：括号内第一个 SELECT 之后的列清单。
            let projection = extract_projection(&tokens[cursor..]);
            definitions.push((name.to_ascii_lowercase(), projection));
            position = cursor + 1;
        }
        index += 1;
    }
    definitions
}

/// 从 `(SELECT a, b FROM ...)` 内提取 projection 列名（只取简单 Ident）。
fn extract_projection(tokens: &[SqlToken]) -> Vec<String> {
    let mut depth = 0usize;
    let mut in_select = false;
    let mut columns = Vec::new();
    for token in tokens {
        match &token.kind {
            SqlTokenKind::LParen => depth += 1,
            SqlTokenKind::RParen => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            SqlTokenKind::Keyword(SqlKeyword::Select) if depth == 1 => {
                in_select = true;
            }
            SqlTokenKind::Keyword(SqlKeyword::From) if in_select && depth == 1 => {
                break;
            }
            SqlTokenKind::Ident | SqlTokenKind::QuotedIdent if in_select && depth == 1 => {
                columns.push(unquote_identifier(&token.text));
            }
            _ => {}
        }
    }
    columns
}

/// 解析 FROM 之后的源表列表（含逗号与 JOIN）。
fn parse_from_sources(
    tokens: &[SqlToken],
    from_index: usize,
) -> Result<Vec<FromSource>, WildcardExpansionError> {
    let mut sources = Vec::new();
    let mut position = from_index + 1;
    let mut depth = 0usize;
    let mut current: Option<FromSource> = None;

    while position < tokens.len() {
        let token = &tokens[position];
        match &token.kind {
            SqlTokenKind::LParen => {
                depth += 1;
                position += 1;
            }
            SqlTokenKind::RParen => {
                depth = depth.saturating_sub(1);
                position += 1;
            }
            _ if depth > 0 => {
                position += 1;
            }
            SqlTokenKind::Keyword(SqlKeyword::Join)
            | SqlTokenKind::Keyword(SqlKeyword::Inner)
            | SqlTokenKind::Keyword(SqlKeyword::Left)
            | SqlTokenKind::Keyword(SqlKeyword::Right)
            | SqlTokenKind::Keyword(SqlKeyword::Full)
            | SqlTokenKind::Keyword(SqlKeyword::Cross) => {
                if let Some(source) = current.take() {
                    sources.push(source);
                }
                position += 1;
            }
            SqlTokenKind::Keyword(SqlKeyword::On)
            | SqlTokenKind::Keyword(SqlKeyword::Using) => {
                // 跳过 JOIN 条件直到下个 JOIN / WHERE / 结尾。
                skip_join_condition(tokens, position, &mut position, &mut depth);
            }
            SqlTokenKind::Keyword(SqlKeyword::Where)
            | SqlTokenKind::Keyword(SqlKeyword::Group)
            | SqlTokenKind::Keyword(SqlKeyword::Order)
            | SqlTokenKind::Keyword(SqlKeyword::Limit)
            | SqlTokenKind::Keyword(SqlKeyword::Union)
            | SqlTokenKind::Keyword(SqlKeyword::Having) => {
                break;
            }
            SqlTokenKind::Comma => {
                if let Some(source) = current.take() {
                    sources.push(source);
                }
                position += 1;
            }
            SqlTokenKind::Ident | SqlTokenKind::QuotedIdent => {
                // 解析表引用（可能 db.schema.table + AS/裸 alias）。
                let (object, alias, consumed) = parse_table_reference(tokens, position);
                // 若前一个源尚未结束（无逗号/JOIN），它其实是没有被吞掉的裸
                // alias，直接把它作为该源的 alias。
                match current.take() {
                    Some(mut existing) => {
                        existing.alias = Some(unquote_identifier(&object.name));
                        sources.push(existing);
                    }
                    None => {
                        current = Some(FromSource {
                            object: Some(object),
                            projection: None,
                            alias,
                        });
                    }
                }
                position += consumed;
            }
            SqlTokenKind::Keyword(SqlKeyword::As) => {
                position += 1;
            }
            _ => {
                position += 1;
            }
        }
    }
    if let Some(source) = current.take() {
        sources.push(source);
    }
    Ok(sources)
}

/// 解析一个表引用：`[db.]schema.name` 或 `name`，返回 (object, alias, consumed)。
/// `alias` 为 None 时，consumed 只包含对象部分。
fn parse_table_reference(
    tokens: &[SqlToken],
    position: usize,
) -> (SqlObjectRef, Option<String>, usize) {
    let mut parts: Vec<String> = Vec::new();
    let mut consumed = 0usize;
    let mut index = position;

    // 读第一个标识符。
    if let Some(token) = tokens.get(index) {
        if matches!(token.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent) {
            parts.push(unquote_identifier(&token.text));
            index += 1;
            consumed += 1;
        }
    }

    // 读后续 `.part`。
    while let Some(dot) = tokens.get(index) {
        if matches!(dot.kind, SqlTokenKind::Dot) {
            if let Some(part) = tokens.get(index + 1) {
                if matches!(part.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent) {
                    parts.push(unquote_identifier(&part.text));
                    index += 2;
                    consumed += 2;
                    continue;
                }
            }
        }
        break;
    }

    // 处理 AS alias 或裸 alias。
    let mut alias = None;
    if let Some(token) = tokens.get(index) {
        if matches!(token.kind, SqlTokenKind::Keyword(SqlKeyword::As)) {
            if let Some(name) = tokens.get(index + 1) {
                if matches!(name.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent) {
                    alias = Some(unquote_identifier(&name.text));
                    consumed += 2;
                }
            }
        } else if matches!(token.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent) {
            // 裸 alias：紧跟对象，且后面不是 `.`、`(`、`,`、JOIN 等。
            let next = tokens.get(index + 1);
            let is_alias = !matches!(
                next.map(|t| &t.kind),
                Some(SqlTokenKind::Dot)
                    | Some(SqlTokenKind::LParen)
                    | Some(SqlTokenKind::Comma)
                    | Some(SqlTokenKind::Keyword(SqlKeyword::Join))
                    | Some(SqlTokenKind::Keyword(SqlKeyword::Inner))
                    | Some(SqlTokenKind::Keyword(SqlKeyword::Left))
                    | Some(SqlTokenKind::Keyword(SqlKeyword::Right))
                    | Some(SqlTokenKind::Keyword(SqlKeyword::Full))
                    | Some(SqlTokenKind::Keyword(SqlKeyword::Cross))
                    | Some(SqlTokenKind::Keyword(SqlKeyword::On))
                    | Some(SqlTokenKind::Keyword(SqlKeyword::Using))
                    | Some(SqlTokenKind::Keyword(SqlKeyword::Where))
            );
            if is_alias {
                alias = Some(unquote_identifier(&token.text));
                consumed += 1;
            }
        }
    }

    let object = match parts.len() {
        1 => SqlObjectRef {
            database: None,
            schema: None,
            name: parts[0].clone(),
        },
        2 => SqlObjectRef {
            database: None,
            schema: Some(parts[0].clone()),
            name: parts[1].clone(),
        },
        3 => SqlObjectRef {
            database: Some(parts[0].clone()),
            schema: Some(parts[1].clone()),
            name: parts[2].clone(),
        },
        _ => SqlObjectRef {
            database: None,
            schema: None,
            name: parts.first().cloned().unwrap_or_default(),
        },
    };

    (object, alias, consumed)
}

/// 跳过 JOIN 条件（ON ... / USING (...)）直到下一个 JOIN / WHERE / 子句。
fn skip_join_condition(
    tokens: &[SqlToken],
    mut position: usize,
    out_position: &mut usize,
    depth: &mut usize,
) {
    let mut local_depth = *depth;
    while position < tokens.len() {
        let token = &tokens[position];
        match &token.kind {
            SqlTokenKind::LParen => local_depth += 1,
            SqlTokenKind::RParen => {
                local_depth = local_depth.saturating_sub(1);
            }
            SqlTokenKind::Keyword(SqlKeyword::Join)
            | SqlTokenKind::Keyword(SqlKeyword::Inner)
            | SqlTokenKind::Keyword(SqlKeyword::Left)
            | SqlTokenKind::Keyword(SqlKeyword::Right)
            | SqlTokenKind::Keyword(SqlKeyword::Full)
            | SqlTokenKind::Keyword(SqlKeyword::Cross)
                if local_depth == 0 =>
            {
                break;
            }
            SqlTokenKind::Keyword(SqlKeyword::Where)
            | SqlTokenKind::Keyword(SqlKeyword::Group)
            | SqlTokenKind::Keyword(SqlKeyword::Order)
            | SqlTokenKind::Keyword(SqlKeyword::Limit)
                if local_depth == 0 =>
            {
                break;
            }
            _ => {}
        }
        position += 1;
    }
    *out_position = position;
    *depth = local_depth;
}

/// 查找 SELECT 列表中的 `*`。返回 (范围, 可选的 qualifier 文本如 `u` 或
/// `schema.table`)。
fn find_wildcards(
    tokens: &[SqlToken],
    select_index: usize,
    from_index: usize,
) -> Vec<(SqlTextRange, Option<String>)> {
    let mut wildcards = Vec::new();
    let mut depth = 0usize;
    let mut index = select_index + 1;
    while index < from_index {
        let token = &tokens[index];
        match &token.kind {
            SqlTokenKind::LParen => depth += 1,
            SqlTokenKind::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 {
            // `*` 单独出现。
            if token.kind == SqlTokenKind::Operator && token.text == "*" {
                let qualifier = previous_dotted_qualifier(tokens, index);
                wildcards.push((
                    SqlTextRange {
                        start_byte: token.start,
                        end_byte: token.end,
                    },
                    qualifier,
                ));
            }
        }
        index += 1;
    }
    wildcards
}

/// 取 `*` 前最近的 `name.` 链作为 qualifier（如 `u.`、`schema.t.`）。
fn previous_dotted_qualifier(tokens: &[SqlToken], star_index: usize) -> Option<String> {
    // 检查 `ident . *` 模式。
    if star_index >= 2 {
        let dot = &tokens[star_index - 1];
        let ident = &tokens[star_index - 2];
        if matches!(dot.kind, SqlTokenKind::Dot)
            && matches!(ident.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent)
        {
            return Some(unquote_identifier(&ident.text));
        }
    }
    None
}

/// 用 qualifier 解析 FROM 源：alias、裸表名或 CTE 名。
fn resolve_source(
    qualifier: Option<&str>,
    sources: &[FromSource],
    cte_definitions: &[(String, Vec<String>)],
) -> Result<ResolvedSource, WildcardExpansionError> {
    match qualifier {
        None => {
            // 无 qualifier 的 `*`：只能用唯一源。
            if sources.len() == 1 {
                let source = &sources[0];
                if let Some(projection) = &source.projection {
                    return Ok(ResolvedSource::Projection(projection.clone()));
                }
                if let Some(object) = source.table() {
                    // 先检查是否为 CTE（名字匹配 WITH 定义）。
                    let lower = object.name.to_ascii_lowercase();
                    if let Some((_, projection)) =
                        cte_definitions.iter().find(|(name, _)| *name == lower)
                    {
                        return Ok(ResolvedSource::Projection(projection.clone()));
                    }
                    return Ok(ResolvedSource::Table(object.clone()));
                }
                return Ok(ResolvedSource::Unknown);
            }
            // 多个源：`SELECT *` 应展开全部源（多表场景由调用方决定）。
            // 纯算法无法合并 metadata 列，这里返回 AmbiguousSource，由上层
            // 用 SqlSchema 直接合并。为满足“多表 source 顺序”，提供专用入口
            // expand_multi_table_wildcard。
            Err(WildcardExpansionError::AmbiguousSource)
        }
        Some(qualifier) => {
            let lower = qualifier.to_ascii_lowercase();
            // 匹配 alias 或表名。
            for source in sources {
                let matches_alias = source
                    .alias
                    .as_ref()
                    .is_some_and(|alias| alias.to_ascii_lowercase() == lower);
                let matches_name = source
                    .table()
                    .is_some_and(|object| object.name.to_ascii_lowercase() == lower);
                if !(matches_alias || matches_name) {
                    continue;
                }
                if let Some(projection) = &source.projection {
                    return Ok(ResolvedSource::Projection(projection.clone()));
                }
                if let Some(object) = source.table() {
                    // 名字命中 CTE 定义 -> projection。
                    let object_lower = object.name.to_ascii_lowercase();
                    if let Some((_, projection)) = cte_definitions
                        .iter()
                        .find(|(name, _)| *name == object_lower)
                    {
                        return Ok(ResolvedSource::Projection(projection.clone()));
                    }
                    return Ok(ResolvedSource::Table(object.clone()));
                }
                return Ok(ResolvedSource::Unknown);
            }
            // CTE 匹配。
            if let Some((_, projection)) = cte_definitions.iter().find(|(name, _)| *name == lower) {
                return Ok(ResolvedSource::Projection(projection.clone()));
            }
            Err(WildcardExpansionError::AmbiguousSource)
        }
    }
}

/// 构建列清单文本。
fn build_column_list(
    columns: &[String],
    qualifier_text: Option<&str>,
    all_aliases: &[String],
    qualifier_mode: SqlWildcardQualifier,
) -> String {
    let mut counts = std::collections::HashMap::new();
    for column in columns {
        *counts.entry(column.to_ascii_lowercase()).or_insert(0usize) += 1;
    }

    columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let base = match (qualifier_text, qualifier_mode) {
                (Some(prefix), SqlWildcardQualifier::Always) => {
                    format!("{}.{}", prefix, quote_if_needed(column))
                }
                (Some(prefix), SqlWildcardQualifier::OnConflict)
                    if counts.get(&column.to_ascii_lowercase()).copied().unwrap_or(0) > 1 =>
                {
                    format!("{}.{}", prefix, quote_if_needed(column))
                }
                _ => {
                    let _ = all_aliases;
                    let _ = index;
                    quote_if_needed(column)
                }
            };
            base
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 多表展开的列清单构建：OnConflict 依据全局出现次数（跨表）决定前缀。
fn build_multi_table_column_list(
    columns: &[String],
    prefix: Option<&str>,
    global_counts: &std::collections::HashMap<String, usize>,
    qualifier_mode: SqlWildcardQualifier,
) -> String {
    columns
        .iter()
        .map(|column| {
            let is_conflict = global_counts
                .get(&column.to_ascii_lowercase())
                .copied()
                .unwrap_or(0)
                > 1;
            let qualified = match qualifier_mode {
                SqlWildcardQualifier::Always => true,
                SqlWildcardQualifier::OnConflict => is_conflict,
                SqlWildcardQualifier::None => false,
            };
            if qualified {
                if let Some(prefix) = prefix {
                    return format!("{}.{}", prefix, quote_if_needed(column));
                }
            }
            quote_if_needed(column)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 多表 `SELECT *` 展开（无 qualifier）：按 FROM 顺序合并所有表的列。
/// `columns_by_source` 与 `sources` 一一对应。
pub fn expand_multi_table_wildcard(
    statement: &str,
    base_byte: usize,
    sources: &[SqlObjectRef],
    columns_by_source: &[Vec<String>],
    qualifier: SqlWildcardQualifier,
) -> Result<SqlWildcardExpansion, WildcardExpansionError> {
    if columns_by_source.len() != sources.len() {
        return Err(WildcardExpansionError::MetadataIncomplete);
    }
    let tokens = SqlTokenizer::new(statement).tokenize();
    let (select_index, from_index) = find_select_from(&tokens)?;
    let from_index = from_index.ok_or(WildcardExpansionError::NoWildcard)?;
    let wildcards = find_wildcards(&tokens, select_index, from_index);
    let Some((star_range, qualifier_text)) = wildcards.first() else {
        return Err(WildcardExpansionError::NoWildcard);
    };
    if qualifier_text.is_some() {
        return Err(WildcardExpansionError::AmbiguousSource);
    }

    // 全局列名出现次数（用于 OnConflict 前缀判定）。
    let mut global_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for columns in columns_by_source {
        for column in columns {
            *global_counts.entry(column.to_ascii_lowercase()).or_insert(0usize) += 1;
        }
    }

    let mut all = Vec::new();
    for (object, columns) in sources.iter().zip(columns_by_source.iter()) {
        let prefix = match qualifier {
            SqlWildcardQualifier::Always => Some(object.name.as_str()),
            SqlWildcardQualifier::OnConflict => Some(object.name.as_str()),
            SqlWildcardQualifier::None => None,
        };
        let text = build_multi_table_column_list(
            columns,
            prefix,
            &global_counts,
            qualifier,
        );
        if !all.is_empty() {
            all.push(", ".to_string());
        }
        all.push(text);
    }

    Ok(SqlWildcardExpansion {
        range: SqlTextRange {
            start_byte: base_byte + star_range.start_byte,
            end_byte: base_byte + star_range.end_byte,
        },
        replacement: all.concat(),
        required_tables: sources.to_vec(),
    })
}

fn quote_if_needed(identifier: &str) -> String {
    if identifier
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        identifier.to_string()
    } else {
        format!("\"{}\"", identifier.replace('"', "\"\""))
    }
}

fn unquote_identifier(text: &str) -> String {
    let trimmed = text.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"')
            || (first == b'`' && last == b'`')
            || (first == b'[' && last == b']')
        {
            let inner = &trimmed[1..trimmed.len() - 1];
            return inner.replace("\"\"", "\"").replace("``", "`");
        }
    }
    trimmed.to_string()
}

fn all_used_aliases(sources: &[FromSource]) -> Vec<String> {
    sources
        .iter()
        .filter_map(|source| source.alias.clone())
        .collect()
}