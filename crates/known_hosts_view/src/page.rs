use gpui::{App, AppContext, Context, EventEmitter, FocusHandle, Focusable, SharedString, Window};
use gpui_component::{Icon};
use one_assets::IconName;
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;
use ssh::{HostKeyVerifier, KnownHost};

pub struct KnownHostsPage {
    focus_handle: FocusHandle,
    pub(crate) hosts: Vec<KnownHost>,
    pub(crate) loading: bool,
    pub(crate) importing: bool,
    pub(crate) load_error: Option<String>,
    pub(crate) load_generation: u64,
}

impl KnownHostsPage {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            focus_handle: cx.focus_handle(),
            hosts: Vec::new(),
            loading: false,
            importing: false,
            load_error: None,
            load_generation: 0,
        };
        page.refresh(cx);
        page
    }

    /// Reload the Known Hosts list from the app trust store on a background
    /// executor so the UI thread never blocks on file I/O.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.importing {
            return;
        }
        self.load_generation = self.load_generation.wrapping_add(1);
        let generation = self.load_generation;
        self.loading = true;
        self.load_error = None;
        let verifier = HostKeyVerifier::default();
        let load_task = cx.background_spawn(async move {
            verifier
                .list_known_hosts()
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = load_task.await;
            _ = this.update(cx, |this, cx| {
                this.apply_load(generation, result, cx);
            });
        })
        .detach();
    }

    /// Scan and import plain entries from the system OpenSSH `known_hosts`
    /// file, then refresh the list to surface the newly imported hosts.
    pub fn scan_system(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.importing {
            return;
        }
        let verifier = HostKeyVerifier::default();
        let Some(path) = verifier.openssh_known_hosts_path() else {
            self.load_error = Some(t!("KnownHosts.scan_unavailable").to_string());
            cx.notify();
            return;
        };
        self.importing = true;
        self.load_error = None;
        let path = path.to_path_buf();
        let import_task = cx.background_spawn(async move {
            verifier
                .import_system_known_hosts(&path)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = import_task.await;
            _ = this.update(cx, |this, cx| {
                this.importing = false;
                if let Err(error) = result {
                    this.load_error = Some(error);
                }
                this.refresh(cx);
            });
        })
        .detach();
    }

    pub fn remove_host(&mut self, identity: ssh::HostKeyIdentity, cx: &mut Context<Self>) {
        if self.loading || self.importing {
            return;
        }
        let verifier = HostKeyVerifier::default();
        let task = cx.background_spawn(async move { verifier.remove_known_host(&identity) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.load_error = Some(error);
                    cx.notify();
                } else {
                    this.refresh(cx);
                }
            });
        })
        .detach();
    }

    fn apply_load(
        &mut self,
        generation: u64,
        result: Result<Vec<KnownHost>, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.load_generation {
            return;
        }
        self.loading = false;
        match result {
            Ok(hosts) => self.hosts = hosts,
            Err(error) => self.load_error = Some(error),
        }
        cx.notify();
    }
}

impl Focusable for KnownHostsPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for KnownHostsPage {}

impl TabContent for KnownHostsPage {
    fn content_key(&self) -> &'static str {
        "KnownHosts"
    }

    fn title(&self, _cx: &App) -> SharedString {
        t!("KnownHosts.title").to_string().into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::Server.color())
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn can_rename(&self, _cx: &App) -> bool {
        false
    }

    /// Reload the trust list every time the tab becomes active so hosts
    /// accepted while the tab was in the background show up immediately.
    fn on_activate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }
}
