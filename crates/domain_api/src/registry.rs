use crate::DomainId;
use crate::file_open::FileOpenHandler;
use crate::form::{ConnectionFormProvider, ConnectionFormRequest};
use crate::home::HomeContext;
use crate::intent::TabIntent;
use crate::kind::ConnectionKindDescriptor;
use crate::settings::SettingsSectionProvider;
use crate::shell_extras::ShellExtras;
use crate::tab_restore::TabRestorer;
use gpui::{AnyView, App, SharedString, Window};
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

    /// 打开双栏文件视图；只有同时具备文件能力的领域实现。
    fn open_sftp(
        &self,
        _connection: &StoredConnection,
        _home: &mut dyn HomeContext,
        _window: &mut Window,
        _cx: &mut App,
    ) -> bool {
        false
    }
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
    tab_restorers: Vec<Arc<dyn TabRestorer>>,
    form_providers: Vec<Arc<dyn ConnectionFormProvider>>,
    settings_providers: Vec<Arc<dyn SettingsSectionProvider>>,
    file_open_handlers: Vec<Arc<dyn FileOpenHandler>>,
    resource_providers: Vec<Arc<dyn crate::resources::ActiveResourceProvider>>,
    shell_extras: Option<Arc<dyn ShellExtras>>,
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

    /// 登记一个领域级 Tab 恢复器；`app_kit::tabs` 启动时读取这张表。
    pub fn register_tab_restorer(&mut self, restorer: Arc<dyn TabRestorer>) {
        self.tab_restorers.push(restorer);
    }

    pub fn tab_restorers(&self) -> &[Arc<dyn TabRestorer>] {
        &self.tab_restorers
    }

    pub fn register_form_provider(&mut self, provider: Arc<dyn ConnectionFormProvider>) {
        self.form_providers.push(provider);
    }

    pub fn register_shell_extras(&mut self, extras: Arc<dyn ShellExtras>) {
        self.shell_extras = Some(extras);
    }

    pub fn shell_extras(&self) -> Option<Arc<dyn ShellExtras>> {
        self.shell_extras.clone()
    }

    pub fn register_active_resource_provider(
        &mut self,
        provider: Arc<dyn crate::resources::ActiveResourceProvider>,
    ) {
        self.resource_providers.push(provider);
    }

    /// 汇总所有领域当前的活动资源。
    pub fn active_resources(&self, cx: &gpui::App) -> Vec<crate::resources::ActiveResourceIds> {
        self.resource_providers
            .iter()
            .flat_map(|provider| provider.active_resources(cx))
            .collect()
    }

    pub fn register_file_open_handler(&mut self, handler: Arc<dyn FileOpenHandler>) {
        self.file_open_handlers.push(handler);
    }

    /// 让领域处理器接管路径；返回 `true` 表示已接管。
    pub fn open_file(&self, path: &std::path::Path, window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
        for handler in &self.file_open_handlers {
            if handler.handles(path) {
                handler.open(path, window, cx);
                return true;
            }
        }
        false
    }

    pub fn register_settings_provider(&mut self, provider: Arc<dyn SettingsSectionProvider>) {
        self.settings_providers.push(provider);
    }

    /// 汇总各领域声明的连接种类，按分组、组内顺序排序，同 ID 只保留首次注册。
    pub fn kinds(&self, cx: &App) -> Vec<ConnectionKindDescriptor> {
        let mut kinds: Vec<ConnectionKindDescriptor> = Vec::new();
        for provider in &self.form_providers {
            for kind in provider.kinds(cx) {
                if kinds.iter().all(|existing| existing.id != kind.id) {
                    kinds.push(kind);
                }
            }
        }
        kinds.sort_by(|left, right| {
            left.group
                .order()
                .cmp(&right.group.order())
                .then(left.order.cmp(&right.order))
                .then(left.id.cmp(&right.id))
        });
        kinds
    }

    /// 已有连接对应的种类 ID，用于编辑/复制时预选表单。
    pub fn kind_for_connection(
        &self,
        connection: &StoredConnection,
        cx: &App,
    ) -> Option<SharedString> {
        self.form_providers
            .iter()
            .find_map(|provider| provider.kind_for(connection, cx))
    }

    /// 按请求构造连接表单；由声明了该种类的领域负责。
    pub fn build_connection_form(
        &self,
        request: ConnectionFormRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyView> {
        self.form_providers
            .iter()
            .find_map(|provider| provider.build(request.clone(), window, cx))
    }

    /// 汇总各领域提供的设置页，按 order 排序。
    pub fn settings_pages(&self, cx: &mut App) -> Vec<gpui_component::setting::SettingPage> {
        let mut pages: Vec<(i32, gpui_component::setting::SettingPage)> = self
            .settings_providers
            .iter()
            .flat_map(|provider| provider.pages(cx))
            .collect();
        pages.sort_by_key(|(order, _)| *order);
        pages.into_iter().map(|(_, page)| page).collect()
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

    pub fn open_sftp(
        &self,
        connection: &StoredConnection,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        self.connection_openers
            .iter()
            .any(|(_, opener)| opener.open_sftp(connection, home, window, cx))
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

    struct FakeTabRestorer;

    impl TabRestorer for FakeTabRestorer {
        fn content_keys(&self) -> Vec<&'static str> {
            vec!["Terminal", "SFTP"]
        }

        fn restore(
            &self,
            _state: &one_core::tab_container::TabItemState,
            _window: &mut Window,
            _cx: &mut App,
        ) -> Option<Arc<dyn one_core::tab_container::TabContentView>> {
            None
        }
    }

    #[test]
    fn tab_restorers_are_registered_and_expose_their_content_keys() {
        let mut registry = DomainRegistry::default();
        assert!(registry.tab_restorers().is_empty());

        registry.register_tab_restorer(Arc::new(FakeTabRestorer));
        assert_eq!(1, registry.tab_restorers().len());
        assert_eq!(
            vec!["Terminal", "SFTP"],
            registry.tab_restorers()[0].content_keys()
        );
    }
}
