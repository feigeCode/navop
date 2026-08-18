mod family;
mod mapping;
mod model;
mod parser;

#[cfg(test)]
mod tests;

use one_core::storage::DatabaseType;

pub(crate) use family::{DatabaseFamily, database_family};
mod type_mapping_override;
use mapping::{map_canonical_type, unsupported_mapping};
pub use model::{MappedColumnType, TypeCompatibility};
use parser::{normalized_type_declaration, parse_canonical_type};
pub use type_mapping_override::{TypeMappingOverride, TypeMappingOverrides};

/// Database context used while comparing source and target schemas.
#[derive(Debug, Clone)]
pub struct SchemaTypeMappingContext<'a> {
    pub source_database_type: &'a DatabaseType,
    pub target_database_type: &'a DatabaseType,
    pub overrides: Option<&'a TypeMappingOverrides>,
}

impl<'a> SchemaTypeMappingContext<'a> {
    pub fn new(
        source_database_type: &'a DatabaseType,
        target_database_type: &'a DatabaseType,
    ) -> Self {
        Self {
            source_database_type,
            target_database_type,
            overrides: None,
        }
    }

    pub fn with_overrides(
        source_database_type: &'a DatabaseType,
        target_database_type: &'a DatabaseType,
        overrides: &'a TypeMappingOverrides,
    ) -> Self {
        Self {
            source_database_type,
            target_database_type,
            overrides: Some(overrides),
        }
    }
}

/// Returns whether the source type matches the target after built-in mapping.
///
/// Comparisons within the same concrete database type retain declaration-level
/// behavior so aliases do not hide real changes inside one database.
pub fn column_types_equivalent(
    source_type: &str,
    target_type: &str,
    context: SchemaTypeMappingContext<'_>,
) -> bool {
    let source_family = database_family(context.source_database_type);
    let target_family = database_family(context.target_database_type);
    let declarations_match = normalized_type_declaration(source_type)
        .eq_ignore_ascii_case(&normalized_type_declaration(target_type));

    // User overrides: if the source type has a user-defined override for the
    // target database, compare against the override's target type.
    if let Some(overrides) = context.overrides {
        if let Some(override_entry) = overrides.find(
            source_type,
            context.target_database_type.storage_key().as_str(),
        ) {
            let mapped = TypeMappingOverrides::apply_override(override_entry);
            return mapped.compatibility.is_safe_for_automatic_sync()
                && normalized_type_declaration(&mapped.target_type)
                    .eq_ignore_ascii_case(&normalized_type_declaration(target_type));
        }
    }

    if context.source_database_type == context.target_database_type {
        return declarations_match;
    }
    if source_family == DatabaseFamily::Other || target_family == DatabaseFamily::Other {
        return false;
    }

    mapped_type_matches_target(source_type, target_type, context)
}

fn mapped_type_matches_target(
    source_type: &str,
    target_type: &str,
    context: SchemaTypeMappingContext<'_>,
) -> bool {
    let mapped = map_column_type_with_overrides(
        source_type,
        context.source_database_type,
        context.target_database_type,
        context.overrides,
    );
    if !mapped.compatibility.is_safe_for_automatic_sync() {
        return false;
    }
    if normalized_type_declaration(&mapped.target_type)
        .eq_ignore_ascii_case(&normalized_type_declaration(target_type))
    {
        return true;
    }

    match (
        parse_canonical_type(
            &mapped.target_type,
            database_family(context.target_database_type),
        ),
        parse_canonical_type(target_type, database_family(context.target_database_type)),
    ) {
        (Some(mapped), Some(target)) if mapped.supports_alias_equivalence() => mapped == target,
        _ => false,
    }
}

/// Maps a source column declaration into a valid target database type.
///
/// Unsupported results keep the source declaration only for diagnostics.
/// Callers must not emit it as target DDL.
pub fn map_column_type(
    source_type: &str,
    source_database_type: &DatabaseType,
    target_database_type: &DatabaseType,
) -> MappedColumnType {
    map_column_type_with_overrides(
        source_type,
        source_database_type,
        target_database_type,
        None,
    )
}

/// Maps a source column declaration into a valid target database type,
/// consulting user-defined overrides first.
pub fn map_column_type_with_overrides(
    source_type: &str,
    source_database_type: &DatabaseType,
    target_database_type: &DatabaseType,
    overrides: Option<&TypeMappingOverrides>,
) -> MappedColumnType {
    // Check user overrides first.
    if let Some(overrides) = overrides {
        if let Some(override_entry) =
            overrides.find(source_type, target_database_type.storage_key().as_str())
        {
            return TypeMappingOverrides::apply_override(override_entry);
        }
    }

    let source_declaration = normalized_type_declaration(source_type);
    let source_family = database_family(source_database_type);
    let target_family = database_family(target_database_type);

    if source_database_type == target_database_type {
        return MappedColumnType::new(source_declaration, TypeCompatibility::Exact);
    }
    if source_family == DatabaseFamily::Other || target_family == DatabaseFamily::Other {
        return unsupported_mapping(
            source_type,
            target_database_type,
            "无法识别外部数据库驱动所属的数据库类型",
        );
    }

    let Some(canonical) = parse_canonical_type(source_type, source_family) else {
        return unsupported_mapping(
            source_type,
            target_database_type,
            "该字段类型属于复杂类型、用户自定义类型或尚未支持的类型",
        );
    };
    map_canonical_type(canonical, source_type, target_database_type)
}
