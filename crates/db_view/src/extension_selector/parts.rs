use db::GlobalDbState;
use extension_component::{DbSelectorSource, FieldSource, UiNode, ViewSpec};

use crate::db_object_selector::{
    DbObjectSelectorPolicy, selector_parts_for_source_with_policy, selector_source_part,
};
use crate::extension_menu::DbTreeExtensionActionContext;

use super::{
    ExtensionSelectorPolicies, SelectorRequest, request_policy, selector_request_for_source,
};

pub(super) fn selector_database(
    request: &SelectorRequest,
    policy: DbObjectSelectorPolicy,
) -> Option<String> {
    if policy.schema_as_database {
        request.database.clone().or_else(|| request.schema.clone())
    } else {
        request.database.clone()
    }
}

pub(super) fn selector_schema(
    request: &SelectorRequest,
    policy: DbObjectSelectorPolicy,
) -> Option<String> {
    if policy.schema_as_database {
        request.schema.clone().or_else(|| request.database.clone())
    } else if policy.show_schema {
        request.schema.clone()
    } else {
        None
    }
}

#[cfg(test)]
pub(super) fn db_selector_sources(spec: &ViewSpec) -> Vec<(String, DbSelectorSource)> {
    db_selector_sources_with_policies(spec, &Default::default())
}

pub(super) fn db_selector_sources_with_policies(
    spec: &ViewSpec,
    policies: &ExtensionSelectorPolicies,
) -> Vec<(String, DbSelectorSource)> {
    spec.nodes
        .iter()
        .flat_map(|node| match node {
            UiNode::Text { .. } => Vec::new(),
            UiNode::Form { fields } => fields
                .iter()
                .flat_map(|field| match field.source.as_ref() {
                    Some(FieldSource::DbSelector(source)) => {
                        db_selector_parts(&field.id, source, policy_for_field(&field.id, policies))
                    }
                    _ => Vec::new(),
                })
                .collect(),
        })
        .collect()
}

fn db_selector_parts(
    field_id: &str,
    source: &DbSelectorSource,
    policy: DbObjectSelectorPolicy,
) -> Vec<(String, DbSelectorSource)> {
    selector_parts_for_source_with_policy(source, policy)
        .into_iter()
        .map(|part| {
            (
                format!("{}.{}", field_id, part.suffix),
                selector_source_part(source, &part),
            )
        })
        .collect()
}

pub(super) fn db_selector_policies(
    spec: &ViewSpec,
    db_state: &GlobalDbState,
    context: &DbTreeExtensionActionContext,
) -> ExtensionSelectorPolicies {
    db_selector_base_sources(spec)
        .into_iter()
        .map(|(field_id, source)| {
            let request = selector_request_for_source(&source, context);
            (field_id, request_policy(db_state, &request).into())
        })
        .collect()
}

fn db_selector_base_sources(spec: &ViewSpec) -> Vec<(String, DbSelectorSource)> {
    spec.nodes
        .iter()
        .flat_map(|node| match node {
            UiNode::Text { .. } => Vec::new(),
            UiNode::Form { fields } => fields
                .iter()
                .filter_map(|field| match field.source.as_ref() {
                    Some(FieldSource::DbSelector(source)) => {
                        Some((field.id.clone(), source.clone()))
                    }
                    _ => None,
                })
                .collect(),
        })
        .collect()
}

fn policy_for_field(
    field_id: &str,
    policies: &ExtensionSelectorPolicies,
) -> DbObjectSelectorPolicy {
    policies
        .get(field_id)
        .copied()
        .unwrap_or_else(DbObjectSelectorPolicy::generic)
}
