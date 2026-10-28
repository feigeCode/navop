//! `navopShell` 的领域实现。
//!
//! 当前覆盖 KnownHosts 与本地终端；SSH/SFTP/端口转发等较重的域
//! 后续按同样方式加入。

use domain_api::{
    Domain, DomainId, DomainRegistry, HomeContext, NotifyLevel, TabIntent, TabOpener,
    TerminalProfileKind,
};
use gpui::{App, AppContext, Window};
use known_hosts_view::KnownHostsPage;
use one_core::settings::{AppSettings, LocalTerminalProfileKind};
use one_core::tab_container::{TabItem, TabOpenMode};
use std::sync::Arc;
use terminal::{local_config_from_settings, local_config_from_settings_with_profile};
use terminal_view::TerminalWorkspace;

pub struct ShellDomain;

impl Domain for ShellDomain {
    fn id(&self) -> DomainId {
        DomainId::SHELL
    }

    fn register(&self, registry: &mut DomainRegistry) {
        registry.register_tab_opener(Arc::new(KnownHostsTabOpener));
        registry.register_tab_opener(Arc::new(TerminalTabOpener));
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
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !Self::supports(intent) {
            return false;
        }
        let page = cx.new(|cx| KnownHostsPage::new(window, cx));
        let item = TabItem::from_dyn("known-hosts", "home", Arc::new(page));
        home.open_tab(item, mode, window, cx)
    }
}

struct TerminalTabOpener;

impl TerminalTabOpener {
    fn supports(intent: &TabIntent) -> bool {
        matches!(intent, TabIntent::Terminal { .. })
    }
}

impl TabOpener for TerminalTabOpener {
    fn open(
        &self,
        intent: &TabIntent,
        _mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !Self::supports(intent) {
            return false;
        }
        let TabIntent::Terminal { profile } = intent else {
            return false;
        };
        let settings = AppSettings::global(cx);
        let config = match profile.as_ref().map(|profile| profile.kind) {
            Some(kind) => {
                local_config_from_settings_with_profile(settings, local_profile_kind(kind), None)
            }
            None => local_config_from_settings(settings, None),
        };
        let config = match config {
            Ok(config) => config,
            Err(error) => {
                home.notify(
                    NotifyLevel::Error,
                    &format!("本地终端配置无效：{error}"),
                    window,
                    cx,
                );
                return false;
            }
        };
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let tab_id = format!("terminal-{timestamp}");
        let tab_index = home.next_tab_index("Terminal", cx);
        let view = cx.new(|cx| TerminalWorkspace::new_with_index(config, tab_index, window, cx));
        let item = TabItem::from_dyn(tab_id, "home", Arc::new(view));
        home.open_tab_with_focus(item, window, cx)
    }
}

fn local_profile_kind(kind: TerminalProfileKind) -> LocalTerminalProfileKind {
    match kind {
        TerminalProfileKind::System => LocalTerminalProfileKind::System,
        TerminalProfileKind::PowerShell => LocalTerminalProfileKind::PowerShell,
        TerminalProfileKind::Cmd => LocalTerminalProfileKind::Cmd,
        TerminalProfileKind::Wsl => LocalTerminalProfileKind::Wsl,
        TerminalProfileKind::GitBash => LocalTerminalProfileKind::GitBash,
        TerminalProfileKind::Zsh => LocalTerminalProfileKind::Zsh,
        TerminalProfileKind::Bash => LocalTerminalProfileKind::Bash,
        TerminalProfileKind::Fish => LocalTerminalProfileKind::Fish,
        TerminalProfileKind::Nushell => LocalTerminalProfileKind::Nushell,
        TerminalProfileKind::Custom => LocalTerminalProfileKind::Custom,
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

    #[test]
    fn terminal_opener_only_accepts_terminal_intent() {
        assert!(TerminalTabOpener::supports(&TabIntent::Terminal {
            profile: None
        }));
        assert!(!TerminalTabOpener::supports(&TabIntent::KnownHosts));
    }
}
