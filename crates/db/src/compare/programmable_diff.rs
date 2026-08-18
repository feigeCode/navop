use std::collections::HashMap;

use super::{
    DiffStatus, RoutineDiff, RoutineKind, RoutineSchema, SchemaCompareError, SchemaCompareOptions,
    TriggerDiff, TriggerSchema,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct RoutineIdentityKey {
    kind: RoutineKind,
    schema: String,
    name: String,
    identity_arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct TriggerIdentityKey {
    schema: String,
    table_name: String,
    name: String,
}

/// Compare functions and procedures without generating synchronization DDL.
pub fn compare_routines(
    source: Vec<RoutineSchema>,
    target: Vec<RoutineSchema>,
    options: &SchemaCompareOptions,
) -> Result<Vec<RoutineDiff>, SchemaCompareError> {
    let source = routine_map(source, options)?;
    let target = routine_map(target, options)?;
    let mut identities = source
        .keys()
        .chain(target.keys())
        .cloned()
        .collect::<Vec<_>>();
    identities.sort();
    identities.dedup();

    let mut diffs = Vec::new();
    for identity in identities {
        match (source.get(&identity), target.get(&identity)) {
            (Some(source), None) => diffs.push(RoutineDiff {
                name: source.name.clone(),
                kind: source.kind,
                status: DiffStatus::Added,
                changes: Vec::new(),
                source: Some(source.clone()),
                target: None,
            }),
            (None, Some(target)) => diffs.push(RoutineDiff {
                name: target.name.clone(),
                kind: target.kind,
                status: DiffStatus::Removed,
                changes: Vec::new(),
                source: None,
                target: Some(target.clone()),
            }),
            (Some(source), Some(target)) => {
                let changes = routine_changes(source, target, options);
                if !changes.is_empty() {
                    diffs.push(RoutineDiff {
                        name: source.name.clone(),
                        kind: source.kind,
                        status: DiffStatus::Modified,
                        changes,
                        source: Some(source.clone()),
                        target: Some(target.clone()),
                    });
                }
            }
            (None, None) => {}
        }
    }

    Ok(diffs)
}

/// Compare triggers without generating synchronization DDL.
pub fn compare_triggers(
    source: Vec<TriggerSchema>,
    target: Vec<TriggerSchema>,
    options: &SchemaCompareOptions,
) -> Result<Vec<TriggerDiff>, SchemaCompareError> {
    let source = trigger_map(source, options)?;
    let target = trigger_map(target, options)?;
    let mut identities = source
        .keys()
        .chain(target.keys())
        .cloned()
        .collect::<Vec<_>>();
    identities.sort();
    identities.dedup();

    let mut diffs = Vec::new();
    for identity in identities {
        match (source.get(&identity), target.get(&identity)) {
            (Some(source), None) => diffs.push(TriggerDiff {
                name: source.name.clone(),
                status: DiffStatus::Added,
                changes: Vec::new(),
                source: Some(source.clone()),
                target: None,
            }),
            (None, Some(target)) => diffs.push(TriggerDiff {
                name: target.name.clone(),
                status: DiffStatus::Removed,
                changes: Vec::new(),
                source: None,
                target: Some(target.clone()),
            }),
            (Some(source), Some(target)) => {
                let changes = trigger_changes(source, target);
                if !changes.is_empty() {
                    diffs.push(TriggerDiff {
                        name: source.name.clone(),
                        status: DiffStatus::Modified,
                        changes,
                        source: Some(source.clone()),
                        target: Some(target.clone()),
                    });
                }
            }
            (None, None) => {}
        }
    }

    Ok(diffs)
}

fn routine_map(
    routines: Vec<RoutineSchema>,
    options: &SchemaCompareOptions,
) -> Result<HashMap<RoutineIdentityKey, RoutineSchema>, SchemaCompareError> {
    let mut map = HashMap::with_capacity(routines.len());
    for routine in routines {
        let key = routine_identity(&routine, options);
        if map.insert(key.clone(), routine).is_some() {
            return Err(SchemaCompareError::DuplicateIdentifier(
                routine_identity_label(&key),
            ));
        }
    }
    Ok(map)
}

fn trigger_map(
    triggers: Vec<TriggerSchema>,
    options: &SchemaCompareOptions,
) -> Result<HashMap<TriggerIdentityKey, TriggerSchema>, SchemaCompareError> {
    let mut map = HashMap::with_capacity(triggers.len());
    for trigger in triggers {
        let key = trigger_identity(&trigger, options);
        if map.insert(key.clone(), trigger).is_some() {
            return Err(SchemaCompareError::DuplicateIdentifier(
                trigger_identity_label(&key),
            ));
        }
    }
    Ok(map)
}

fn routine_identity(routine: &RoutineSchema, options: &SchemaCompareOptions) -> RoutineIdentityKey {
    RoutineIdentityKey {
        kind: routine.kind,
        schema: identifier_component(routine.schema.as_deref().unwrap_or_default(), options),
        name: identifier_component(&routine.name, options),
        identity_arguments: normalize_whitespace(
            routine.identity_arguments.as_deref().unwrap_or_default(),
        ),
    }
}

fn trigger_identity(trigger: &TriggerSchema, options: &SchemaCompareOptions) -> TriggerIdentityKey {
    TriggerIdentityKey {
        schema: identifier_component(trigger.schema.as_deref().unwrap_or_default(), options),
        table_name: identifier_component(&trigger.table_name, options),
        name: identifier_component(&trigger.name, options),
    }
}

fn routine_changes(
    source: &RoutineSchema,
    target: &RoutineSchema,
    options: &SchemaCompareOptions,
) -> Vec<String> {
    let mut changes = Vec::new();
    if normalize_optional_metadata(source.return_type.as_deref())
        != normalize_optional_metadata(target.return_type.as_deref())
    {
        changes.push(format!(
            "return type: {} → {}",
            display_optional(target.return_type.as_deref()),
            display_optional(source.return_type.as_deref())
        ));
    }
    if normalize_parameters(&source.parameters) != normalize_parameters(&target.parameters) {
        changes.push(format!(
            "parameters: {} → {}",
            display_parameters(&target.parameters),
            display_parameters(&source.parameters)
        ));
    }
    if normalize_definition(source.definition.as_deref())
        != normalize_definition(target.definition.as_deref())
    {
        changes.push("definition changed".to_string());
    }
    if !options.ignore_comments
        && normalize_optional_metadata(source.comment.as_deref())
            != normalize_optional_metadata(target.comment.as_deref())
    {
        changes.push(format!(
            "comment: {} → {}",
            display_optional(target.comment.as_deref()),
            display_optional(source.comment.as_deref())
        ));
    }
    changes
}

fn trigger_changes(source: &TriggerSchema, target: &TriggerSchema) -> Vec<String> {
    let mut changes = Vec::new();
    if normalize_keyword_metadata(&source.event) != normalize_keyword_metadata(&target.event) {
        changes.push(format!("event: {} → {}", target.event, source.event));
    }
    if normalize_keyword_metadata(&source.timing) != normalize_keyword_metadata(&target.timing) {
        changes.push(format!("timing: {} → {}", target.timing, source.timing));
    }
    if normalize_definition(source.definition.as_deref())
        != normalize_definition(target.definition.as_deref())
    {
        changes.push("definition changed".to_string());
    }
    changes
}

fn identifier_component(value: &str, options: &SchemaCompareOptions) -> String {
    let value = value.trim();
    if options.case_sensitive_identifiers {
        value.to_string()
    } else {
        value.to_lowercase()
    }
}

fn normalize_parameters(parameters: &[String]) -> Vec<String> {
    parameters
        .iter()
        .map(|parameter| normalize_whitespace(parameter))
        .collect()
}

fn normalize_optional_metadata(value: Option<&str>) -> String {
    normalize_whitespace(value.unwrap_or_default())
}

fn normalize_keyword_metadata(value: &str) -> String {
    normalize_whitespace(value).to_lowercase()
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_definition(value: Option<&str>) -> String {
    value
        .unwrap_or_default()
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn display_optional(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("<none>")
        .to_string()
}

fn display_parameters(parameters: &[String]) -> String {
    if parameters.is_empty() {
        "<none>".to_string()
    } else {
        parameters.join(", ")
    }
}

fn routine_identity_label(identity: &RoutineIdentityKey) -> String {
    let kind = match identity.kind {
        RoutineKind::Function => "function",
        RoutineKind::Procedure => "procedure",
    };
    let qualified_name = if identity.schema.is_empty() {
        identity.name.clone()
    } else {
        format!("{}.{}", identity.schema, identity.name)
    };
    format!("{kind} {qualified_name}({})", identity.identity_arguments)
}

fn trigger_identity_label(identity: &TriggerIdentityKey) -> String {
    let table = if identity.schema.is_empty() {
        identity.table_name.clone()
    } else {
        format!("{}.{}", identity.schema, identity.table_name)
    };
    format!("trigger {table}.{}", identity.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routine(kind: RoutineKind, name: &str) -> RoutineSchema {
        RoutineSchema {
            kind,
            name: name.to_string(),
            schema: Some("public".to_string()),
            ..Default::default()
        }
    }

    fn trigger(name: &str, table_name: &str) -> TriggerSchema {
        TriggerSchema {
            name: name.to_string(),
            schema: Some("public".to_string()),
            table_name: table_name.to_string(),
            event: "INSERT".to_string(),
            timing: "BEFORE".to_string(),
            definition: None,
        }
    }

    #[test]
    fn routines_report_added_removed_and_modified() {
        let mut modified_source = routine(RoutineKind::Function, "modified");
        modified_source.return_type = Some("bigint".to_string());
        let mut modified_target = modified_source.clone();
        modified_target.return_type = Some("integer".to_string());

        let diffs = compare_routines(
            vec![routine(RoutineKind::Function, "added"), modified_source],
            vec![routine(RoutineKind::Function, "removed"), modified_target],
            &SchemaCompareOptions::default(),
        )
        .unwrap();

        assert_eq!(diffs.len(), 3);
        assert!(
            diffs
                .iter()
                .any(|diff| { diff.name == "added" && diff.status == DiffStatus::Added })
        );
        assert!(
            diffs
                .iter()
                .any(|diff| { diff.name == "removed" && diff.status == DiffStatus::Removed })
        );
        assert!(diffs.iter().any(|diff| {
            diff.name == "modified"
                && diff.status == DiffStatus::Modified
                && diff
                    .changes
                    .iter()
                    .any(|change| change.contains("return type"))
        }));
    }

    #[test]
    fn function_and_procedure_with_same_name_do_not_collide() {
        let source = vec![
            routine(RoutineKind::Function, "refresh"),
            routine(RoutineKind::Procedure, "refresh"),
        ];

        let diffs =
            compare_routines(source.clone(), source, &SchemaCompareOptions::default()).unwrap();

        assert!(diffs.is_empty());
    }

    #[test]
    fn postgres_overloads_use_identity_arguments() {
        let mut integer = routine(RoutineKind::Function, "calculate");
        integer.identity_arguments = Some("integer".to_string());
        let mut numeric = integer.clone();
        numeric.identity_arguments = Some("numeric".to_string());

        let diffs = compare_routines(
            vec![integer.clone(), numeric.clone()],
            vec![integer, numeric],
            &SchemaCompareOptions::default(),
        )
        .unwrap();

        assert!(diffs.is_empty());
    }

    #[test]
    fn triggers_report_added_removed_and_modified() {
        let mut modified_source = trigger("modified", "orders");
        modified_source.timing = "AFTER".to_string();
        let modified_target = trigger("modified", "orders");

        let diffs = compare_triggers(
            vec![trigger("added", "orders"), modified_source],
            vec![trigger("removed", "orders"), modified_target],
            &SchemaCompareOptions::default(),
        )
        .unwrap();

        assert_eq!(diffs.len(), 3);
        assert!(
            diffs
                .iter()
                .any(|diff| { diff.name == "added" && diff.status == DiffStatus::Added })
        );
        assert!(
            diffs
                .iter()
                .any(|diff| { diff.name == "removed" && diff.status == DiffStatus::Removed })
        );
        assert!(diffs.iter().any(|diff| {
            diff.name == "modified"
                && diff.status == DiffStatus::Modified
                && diff.changes.iter().any(|change| change.contains("timing"))
        }));
    }

    #[test]
    fn same_trigger_name_on_different_tables_does_not_collide() {
        let source = vec![trigger("audit", "orders"), trigger("audit", "users")];

        let diffs =
            compare_triggers(source.clone(), source, &SchemaCompareOptions::default()).unwrap();

        assert!(diffs.is_empty());
    }

    #[test]
    fn identifier_case_policy_is_applied_to_routines_and_triggers() {
        let insensitive = SchemaCompareOptions::default();
        assert!(
            compare_routines(
                vec![routine(RoutineKind::Function, "Calculate")],
                vec![routine(RoutineKind::Function, "calculate")],
                &insensitive,
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            compare_triggers(
                vec![trigger("Audit", "Orders")],
                vec![trigger("audit", "orders")],
                &insensitive,
            )
            .unwrap()
            .is_empty()
        );

        let sensitive = SchemaCompareOptions {
            case_sensitive_identifiers: true,
            ..SchemaCompareOptions::default()
        };
        assert_eq!(
            compare_routines(
                vec![routine(RoutineKind::Function, "Calculate")],
                vec![routine(RoutineKind::Function, "calculate")],
                &sensitive,
            )
            .unwrap()
            .len(),
            2
        );
    }

    #[test]
    fn definition_line_endings_and_trailing_whitespace_are_ignored() {
        let mut source = routine(RoutineKind::Function, "calculate");
        source.definition = Some("BEGIN\r\n  RETURN 1;  \r\nEND\r\n".to_string());
        let mut target = source.clone();
        target.definition = Some("BEGIN\n  RETURN 1;\nEND\n".to_string());

        assert!(
            compare_routines(vec![source], vec![target], &SchemaCompareOptions::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn duplicate_identities_return_an_error() {
        let duplicate = routine(RoutineKind::Function, "calculate");
        let error = compare_routines(
            vec![duplicate.clone(), duplicate],
            Vec::new(),
            &SchemaCompareOptions::default(),
        )
        .unwrap_err();

        assert!(matches!(error, SchemaCompareError::DuplicateIdentifier(_)));
    }

    #[test]
    fn duplicate_trigger_identities_return_an_error() {
        let duplicate = trigger("audit", "orders");
        let error = compare_triggers(
            vec![duplicate.clone(), duplicate],
            Vec::new(),
            &SchemaCompareOptions::default(),
        )
        .unwrap_err();

        assert!(matches!(error, SchemaCompareError::DuplicateIdentifier(_)));
    }

    #[test]
    fn output_order_is_stable() {
        let diffs = compare_routines(
            vec![
                routine(RoutineKind::Procedure, "zeta"),
                routine(RoutineKind::Function, "alpha"),
            ],
            Vec::new(),
            &SchemaCompareOptions::default(),
        )
        .unwrap();

        assert_eq!(
            diffs
                .iter()
                .map(|diff| (diff.kind, diff.name.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (RoutineKind::Function, "alpha"),
                (RoutineKind::Procedure, "zeta")
            ]
        );
    }

    #[test]
    fn ignore_comments_suppresses_routine_comment_changes() {
        let mut source = routine(RoutineKind::Function, "calculate");
        source.comment = Some("source".to_string());
        let mut target = source.clone();
        target.comment = Some("target".to_string());
        let options = SchemaCompareOptions {
            ignore_comments: true,
            ..SchemaCompareOptions::default()
        };

        assert!(
            compare_routines(vec![source], vec![target], &options)
                .unwrap()
                .is_empty()
        );
    }
}
