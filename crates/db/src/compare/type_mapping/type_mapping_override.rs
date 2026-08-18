/// User-customizable type mapping overrides.
///
/// When a user adjusts the target type for a particular source-type to
/// target-database combination, the override is stored here and consulted
/// **before** the built-in canonical mapping in [`crate::compare::map_column_type`].
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{MappedColumnType, TypeCompatibility};

/// A single user-defined type mapping override.
///
/// The key for matching is `(source_type_pattern, target_database_storage_key)`.
/// An exact normalized declaration is preferred, then the base type name is
/// used as a fallback. Both forms are matched case-insensitively.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeMappingOverride {
    /// Source type pattern, e.g. `"varchar"`, `"decimal(10,2)"`,
    /// `"timestamp with time zone"`. Matching is case-insensitive. A complete
    /// declaration is preferred over a base type such as `"decimal"`.
    pub source_type: String,
    /// Storage key of the target database, e.g. `"MySQL"`, `"PostgreSQL"`.
    pub target_database: String,
    /// The target type the user wants to use instead of the built-in mapping.
    pub target_type: String,
    /// Whether the override is active.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Optional note the user can attach.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn default_enabled() -> bool {
    true
}

/// A collection of overrides that can be looked up efficiently.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeMappingOverrides {
    /// Keyed by `"{source_type_lower}::{target_database_storage_key_lower}"`.
    #[serde(default)]
    pub overrides: Vec<TypeMappingOverride>,
}

impl TypeMappingOverrides {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a fast-lookup map from `"{source_type_lower}::{target_db_key}"` to override.
    fn lookup_map(&self) -> HashMap<String, &TypeMappingOverride> {
        self.overrides
            .iter()
            .filter(|o| o.enabled)
            .map(|o| {
                let key = Self::lookup_key(&o.source_type, &o.target_database);
                (key, o)
            })
            .collect()
    }

    /// Returns the override matching the given source type and target database,
    /// if any.
    ///
    /// Matching strategy:
    /// 1. Exact match on the normalized full declaration.
    /// 2. Fallback match on the normalized base type name.
    pub fn find(
        &self,
        source_type: &str,
        target_database_storage_key: &str,
    ) -> Option<&TypeMappingOverride> {
        let map = self.lookup_map();

        // Prefer a complete declaration, so an override for `decimal(10,2)`
        // can take precedence over a broad `decimal` override.
        let normalized = Self::normalize_source_type(source_type);
        if let Some(o) = map.get(&Self::lookup_key(&normalized, target_database_storage_key)) {
            return Some(o);
        }

        // Fall back to a base type override such as `decimal`.
        let base = Self::base_type_name(source_type);
        if base != normalized {
            if let Some(o) = map.get(&Self::lookup_key(&base, target_database_storage_key)) {
                return Some(o);
            }
        }

        None
    }

    /// Apply an override to produce a [`MappedColumnType`].
    ///
    /// Valid overrides are marked [`TypeCompatibility::Equivalent`] since the
    /// user explicitly chose the mapping. Empty or obviously unsafe SQL
    /// fragments are returned as [`TypeCompatibility::Unsupported`].
    pub fn apply_override(override_entry: &TypeMappingOverride) -> MappedColumnType {
        let raw_target_type = override_entry.target_type.as_str();
        if !is_safe_target_type(raw_target_type) {
            return MappedColumnType::new(
                raw_target_type.trim().to_string(),
                TypeCompatibility::Unsupported,
            )
            .with_warning(format!(
                "用户自定义类型映射的目标类型无效或包含不安全 SQL：{}",
                override_entry.target_type
            ));
        }
        let target_type = raw_target_type.trim();
        MappedColumnType::new(target_type.to_string(), TypeCompatibility::Equivalent).with_warning(
            format!(
                "用户自定义类型映射：{} → {}",
                override_entry.source_type, override_entry.target_type
            ),
        )
    }

    /// Add or replace an override.
    pub fn upsert(&mut self, override_entry: TypeMappingOverride) {
        let key = Self::lookup_key(&override_entry.source_type, &override_entry.target_database);
        if let Some(existing) = self
            .overrides
            .iter_mut()
            .find(|o| Self::lookup_key(&o.source_type, &o.target_database) == key)
        {
            *existing = override_entry;
        } else {
            self.overrides.push(override_entry);
        }
    }

    /// Remove an override by source type and target database.
    pub fn remove(&mut self, source_type: &str, target_database: &str) {
        let key = Self::lookup_key(source_type, target_database);
        self.overrides
            .retain(|o| Self::lookup_key(&o.source_type, &o.target_database) != key);
    }

    fn lookup_key(source_type: &str, target_database: &str) -> String {
        format!(
            "{}::{}",
            Self::normalize_source_type(source_type),
            target_database.trim().to_ascii_lowercase()
        )
    }

    fn normalize_source_type(source_type: &str) -> String {
        source_type
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    }

    /// Extract the base type name (text before `(`) and normalize.
    fn base_type_name(source_type: &str) -> String {
        let normalized = Self::normalize_source_type(source_type);
        normalized
            .split('(')
            .next()
            .unwrap_or(&normalized)
            .trim()
            .to_string()
    }
}

fn is_safe_target_type(target_type: &str) -> bool {
    !target_type.is_empty()
        && !target_type
            .chars()
            .any(|character| matches!(character, ';' | '\r' | '\n' | '\0'))
        && !target_type.contains("--")
        && !target_type.contains("/*")
        && !target_type.contains("*/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_exact_match() {
        let overrides = TypeMappingOverrides {
            overrides: vec![TypeMappingOverride {
                source_type: "varchar".to_string(),
                target_database: "PostgreSQL".to_string(),
                target_type: "TEXT".to_string(),
                enabled: true,
                note: None,
            }],
        };
        let found = overrides.find("varchar(255)", "PostgreSQL");
        assert!(found.is_some());
        assert_eq!(found.unwrap().target_type, "TEXT");
    }

    #[test]
    fn exact_declaration_match_precedes_base_type_match() {
        let overrides = TypeMappingOverrides {
            overrides: vec![
                TypeMappingOverride {
                    source_type: "decimal".to_string(),
                    target_database: "PostgreSQL".to_string(),
                    target_type: "NUMERIC".to_string(),
                    enabled: true,
                    note: None,
                },
                TypeMappingOverride {
                    source_type: " DECIMAL(10,2) ".to_string(),
                    target_database: "postgresql".to_string(),
                    target_type: "DECIMAL(20,4)".to_string(),
                    enabled: true,
                    note: None,
                },
            ],
        };

        let found = overrides.find("decimal(10,2)", "POSTGRESQL").unwrap();
        assert_eq!(found.target_type, "DECIMAL(20,4)");
    }

    #[test]
    fn find_no_match() {
        let overrides = TypeMappingOverrides::new();
        assert!(overrides.find("varchar", "MySQL").is_none());
    }

    #[test]
    fn find_skips_disabled() {
        let overrides = TypeMappingOverrides {
            overrides: vec![TypeMappingOverride {
                source_type: "varchar".to_string(),
                target_database: "PostgreSQL".to_string(),
                target_type: "TEXT".to_string(),
                enabled: false,
                note: None,
            }],
        };
        assert!(overrides.find("varchar", "PostgreSQL").is_none());
    }

    #[test]
    fn upsert_replaces() {
        let mut overrides = TypeMappingOverrides::new();
        overrides.upsert(TypeMappingOverride {
            source_type: "int".to_string(),
            target_database: "PostgreSQL".to_string(),
            target_type: "INTEGER".to_string(),
            enabled: true,
            note: None,
        });
        overrides.upsert(TypeMappingOverride {
            source_type: "int".to_string(),
            target_database: "PostgreSQL".to_string(),
            target_type: "BIGINT".to_string(),
            enabled: true,
            note: None,
        });
        assert_eq!(overrides.overrides.len(), 1);
        assert_eq!(overrides.overrides[0].target_type, "BIGINT");
    }

    #[test]
    fn upsert_uses_canonical_source_and_database_keys() {
        let mut overrides = TypeMappingOverrides::new();
        overrides.upsert(TypeMappingOverride {
            source_type: "  DECIMAL   (10,2) ".to_string(),
            target_database: " PostgreSQL ".to_string(),
            target_type: "NUMERIC(10,2)".to_string(),
            enabled: true,
            note: None,
        });
        overrides.upsert(TypeMappingOverride {
            source_type: "decimal (10,2)".to_string(),
            target_database: "postgresql".to_string(),
            target_type: "DECIMAL(20,4)".to_string(),
            enabled: true,
            note: None,
        });

        assert_eq!(overrides.overrides.len(), 1);
        assert_eq!(overrides.overrides[0].target_type, "DECIMAL(20,4)");
    }

    #[test]
    fn remove_works() {
        let mut overrides = TypeMappingOverrides::new();
        overrides.upsert(TypeMappingOverride {
            source_type: "int".to_string(),
            target_database: "PostgreSQL".to_string(),
            target_type: "BIGINT".to_string(),
            enabled: true,
            note: None,
        });
        overrides.remove("int", "PostgreSQL");
        assert!(overrides.overrides.is_empty());
    }

    #[test]
    fn apply_override_produces_warning() {
        let o = TypeMappingOverride {
            source_type: "varchar".to_string(),
            target_database: "PostgreSQL".to_string(),
            target_type: "TEXT".to_string(),
            enabled: true,
            note: None,
        };
        let mapped = TypeMappingOverrides::apply_override(&o);
        assert_eq!(mapped.target_type, "TEXT");
        assert_eq!(mapped.compatibility, TypeCompatibility::Equivalent);
        assert!(mapped.warning.is_some());
    }

    #[test]
    fn invalid_override_target_is_not_safe_for_sync() {
        for target_type in ["", "TEXT; DROP TABLE users", "TEXT -- comment", "TEXT\n"] {
            let mapped = TypeMappingOverrides::apply_override(&TypeMappingOverride {
                source_type: "varchar".to_string(),
                target_database: "PostgreSQL".to_string(),
                target_type: target_type.to_string(),
                enabled: true,
                note: None,
            });
            assert_eq!(mapped.compatibility, TypeCompatibility::Unsupported);
        }
    }
}
