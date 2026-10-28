//! `navopShell` 的领域实现。
//!
//! 先把最小、依赖最少的 KnownHosts 迁进来，验证
//! 「Domain 注册 → DomainRegistry → TabItem::from_dyn」链路；
//! SSH/SFTP/终端等较重的域后续按同样方式加入。

use domain_api::{Domain, DomainId, DomainRegistry, TabIntent, TabOpener};
use gpui::{App, AppContext, Window};
use known_hosts_view::KnownHostsPage;
use one_core::tab_container::TabItem;
use std::sync::Arc;

pub struct ShellDomain;

impl Domain for ShellDomain {
    fn id(&self) -> DomainId {
        DomainId::SHELL
    }

    fn register(&self, registry: &mut DomainRegistry) {
        registry.register_tab_opener(Arc::new(KnownHostsTabOpener));
    }

    fn init(&self, _cx: &mut App) -> anyhow::Result<()> {
        Ok(())
    }
}

struct KnownHostsTabOpener;

impl KnownHostsTabOpener {
    fn supports(intent: &TabIntent) -> bool {
        matches!(intent, TabIntent::KnownHosts)
    }
}

impl TabOpener for KnownHostsTabOpener {
    fn open(
        &self,
        intent: &TabIntent,
        _host: &mut dyn domain_api::TabHost,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<TabItem> {
        if !Self::supports(intent) {
            return None;
        }
        let page = cx.new(|cx| KnownHostsPage::new(window, cx));
        Some(TabItem::from_dyn(
            "known-hosts",
            "home",
            Arc::new(page),
        ))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_domain_has_stable_id() {
        assert_eq!(ShellDomain.id(), DomainId::SHELL);
    }

    #[test]
    fn known_hosts_opener_only_accepts_known_hosts_intent() {
        assert!(KnownHostsTabOpener::supports(&TabIntent::KnownHosts));
        assert!(!KnownHostsTabOpener::supports(&TabIntent::PortForwarding));
    }
}
