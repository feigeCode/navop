use crate::resource::{ResourceContext, ResourceId, ResourceRef};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceCatalog {
    pub resources: Vec<ResourceRef>,
}

impl ResourceCatalog {
    pub fn new(resources: Vec<ResourceRef>) -> Self {
        Self { resources }
    }

    pub fn get(&self, id: &ResourceId) -> Option<&ResourceRef> {
        self.resources.iter().find(|resource| &resource.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentResourceScope {
    pub selected: Vec<ResourceRef>,
    pub default_target: Option<DefaultTarget>,
}

impl AgentResourceScope {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn single_default(resource: ResourceRef, reason: DefaultTargetReason) -> Self {
        let target = DefaultTarget {
            resource_id: resource.id.clone(),
            reason,
        };
        Self {
            selected: vec![resource],
            default_target: Some(target),
        }
    }

    pub fn from_resource_context(context: ResourceContext, reason: DefaultTargetReason) -> Self {
        let default_target = context.current.map(|id| DefaultTarget {
            resource_id: id,
            reason,
        });
        Self {
            selected: context.resources,
            default_target,
        }
    }

    pub fn to_resource_context(&self) -> ResourceContext {
        ResourceContext {
            current: self
                .default_target
                .as_ref()
                .map(|target| target.resource_id.clone()),
            resources: self.selected.clone(),
        }
    }

    pub fn add_from_catalog(
        &mut self,
        catalog: &ResourceCatalog,
        id: &ResourceId,
        reason: DefaultTargetReason,
    ) -> bool {
        if self.selected.iter().any(|resource| &resource.id == id) {
            return false;
        }
        let Some(resource) = catalog.get(id).cloned() else {
            return false;
        };
        self.selected.push(resource);
        if self.default_target.is_none() {
            self.default_target = Some(DefaultTarget {
                resource_id: id.clone(),
                reason,
            });
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultTarget {
    pub resource_id: ResourceId,
    pub reason: DefaultTargetReason,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultTargetReason {
    CurrentTerminal,
    CurrentDatabase,
    CurrentConnection,
    UserSelected,
    MentionedFirst,
    RestoredSession,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::ResourceKind;

    fn resource(id: &str, kind: ResourceKind, label: &str) -> ResourceRef {
        ResourceRef::new(id, kind, label)
    }

    #[test]
    fn workbench_scope_can_start_empty_while_catalog_has_resources() {
        let catalog = ResourceCatalog::new(vec![
            resource("ssh-a", ResourceKind::Ssh, "prod-a"),
            resource("db-a", ResourceKind::Mysql, "prod-db"),
        ]);
        let scope = AgentResourceScope::empty();

        assert_eq!(2, catalog.resources.len());
        assert!(scope.selected.is_empty());
        assert!(scope.default_target.is_none());
        assert!(scope.to_resource_context().is_empty());
    }

    #[test]
    fn current_connection_scope_sets_explicit_default_target() {
        let current = resource("ssh-a", ResourceKind::Ssh, "prod-a");

        let scope = AgentResourceScope::single_default(
            current.clone(),
            DefaultTargetReason::CurrentConnection,
        );

        assert_eq!(vec![current], scope.selected);
        assert_eq!(
            Some(&ResourceId::new("ssh-a")),
            scope
                .default_target
                .as_ref()
                .map(|target| &target.resource_id)
        );
        assert_eq!(
            Some("prod-a"),
            scope
                .to_resource_context()
                .current()
                .map(|resource| resource.label.as_str())
        );
    }

    #[test]
    fn adding_mentioned_resource_sets_default_only_when_scope_has_no_default() {
        let catalog = ResourceCatalog::new(vec![
            resource("ssh-a", ResourceKind::Ssh, "prod-a"),
            resource("db-a", ResourceKind::Mysql, "prod-db"),
        ]);
        let mut scope = AgentResourceScope::empty();

        assert!(scope.add_from_catalog(
            &catalog,
            &ResourceId::new("db-a"),
            DefaultTargetReason::MentionedFirst
        ));

        assert_eq!(1, scope.selected.len());
        assert_eq!(
            Some(&ResourceId::new("db-a")),
            scope
                .default_target
                .as_ref()
                .map(|target| &target.resource_id)
        );
    }

    #[test]
    fn adding_second_mentioned_resource_keeps_existing_default() {
        let catalog = ResourceCatalog::new(vec![
            resource("ssh-a", ResourceKind::Ssh, "prod-a"),
            resource("db-a", ResourceKind::Mysql, "prod-db"),
        ]);
        let mut scope = AgentResourceScope::single_default(
            resource("ssh-a", ResourceKind::Ssh, "prod-a"),
            DefaultTargetReason::CurrentConnection,
        );

        assert!(scope.add_from_catalog(
            &catalog,
            &ResourceId::new("db-a"),
            DefaultTargetReason::MentionedFirst
        ));

        assert_eq!(2, scope.selected.len());
        assert_eq!(
            Some(&ResourceId::new("ssh-a")),
            scope
                .default_target
                .as_ref()
                .map(|target| &target.resource_id)
        );
    }
}
