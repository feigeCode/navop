use crate::DomainId;
use crate::home::HomeContext;
use crate::intent::TabIntent;
use gpui::{App, Window};
use one_core::storage::{ConnectionType, StoredConnection};
use one_core::tab_container::TabOpenMode;
use std::sync::Arc;

/// 连接类型对应的打开策略，替代 `HomePage` 里的 13 路 `match`。
pub trait ConnectionOpener: Send + Sync {
    fn open(
        &self,
        connection: &StoredConnection,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool;
}

/// 不依赖具体连接类型的 Tab 打开器（本地终端、端口转发、known_hosts 等）。
pub trait TabOpener: Send + Sync {
    fn open(
        &self,
        intent: &TabIntent,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool;
}

/// 应用启动时由各领域填充的注册表。
#[derive(Default)]
pub struct DomainRegistry {
    domains: Vec<DomainId>,
    connection_openers: Vec<(ConnectionType, Arc<dyn ConnectionOpener>)>,
    tab_openers: Vec<Arc<dyn TabOpener>>,
}

impl DomainRegistry {
    pub fn register_domain(&mut self, id: DomainId) {
        if !self.domains.contains(&id) {
            self.domains.push(id);
        }
    }

    pub fn register_connection_opener(
        &mut self,
        connection_type: ConnectionType,
        opener: Arc<dyn ConnectionOpener>,
    ) {
        self.connection_openers
            .retain(|(existing, _)| *existing != connection_type);
        self.connection_openers.push((connection_type, opener));
    }

    pub fn register_tab_opener(&mut self, opener: Arc<dyn TabOpener>) {
        self.tab_openers.push(opener);
    }

    pub fn domains(&self) -> &[DomainId] {
        &self.domains
    }

    pub fn has_connection_opener(&self, connection_type: ConnectionType) -> bool {
        self.connection_openers
            .iter()
            .any(|(existing, _)| *existing == connection_type)
    }

    pub fn open_connection(
        &self,
        connection: &StoredConnection,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let opener = self
            .connection_openers
            .iter()
            .find(|(connection_type, _)| *connection_type == connection.connection_type)
            .or_else(|| {
                self.connection_openers
                    .iter()
                    .find(|(connection_type, _)| *connection_type == ConnectionType::All)
            });
        opener.is_some_and(|(_, opener)| opener.open(connection, mode, home, window, cx))
    }

    pub fn open_tab(
        &self,
        intent: &TabIntent,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        self.tab_openers
            .iter()
            .any(|opener| opener.open(intent, mode, home, window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home::NotifyLevel;

    struct FakeConnectionOpener;

    impl ConnectionOpener for FakeConnectionOpener {
        fn open(
            &self,
            _connection: &StoredConnection,
            _mode: TabOpenMode,
            _home: &mut dyn HomeContext,
            _window: &mut Window,
            _cx: &mut App,
        ) -> bool {
            true
        }
    }

    #[test]
    fn connection_opener_is_registered_by_type_and_replaced_on_conflict() {
        let mut registry = DomainRegistry::default();
        assert!(!registry.has_connection_opener(ConnectionType::Database));

        registry
            .register_connection_opener(ConnectionType::Database, Arc::new(FakeConnectionOpener));
        assert!(registry.has_connection_opener(ConnectionType::Database));

        registry
            .register_connection_opener(ConnectionType::Database, Arc::new(FakeConnectionOpener));
        assert_eq!(
            1,
            registry
                .connection_openers
                .iter()
                .filter(|(connection_type, _)| *connection_type == ConnectionType::Database)
                .count()
        );
    }

    #[test]
    fn domains_are_registered_once() {
        let mut registry = DomainRegistry::default();
        registry.register_domain(DomainId::DATABASE);
        registry.register_domain(DomainId::DATABASE);
        assert_eq!(&[DomainId::DATABASE], registry.domains());
    }

    #[test]
    fn notify_level_is_product_independent() {
        assert_ne!(NotifyLevel::Info, NotifyLevel::Error);
    }
}
