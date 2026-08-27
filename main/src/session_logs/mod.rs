mod actions;
mod model;
mod render;
mod row;

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, UniformListScrollHandle, Window,
};
use gpui_component::{
    Icon, IconName,
    input::{InputEvent, InputState},
};
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;
use smol::Timer;
use std::{path::PathBuf, time::Duration};
use terminal::recording::{
    RecordingFileLimits, SessionLogCatalog, SessionLogEntry, SessionLogFavorites,
    load_session_log_favorites, scan_session_logs, session_logs_directory,
};

const SESSION_LOG_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

pub(crate) struct SessionLogsPage {
    focus_handle: FocusHandle,
    search_input: Entity<InputState>,
    directory: Option<PathBuf>,
    catalog: SessionLogCatalog,
    favorites: SessionLogFavorites,
    loading: bool,
    load_error: Option<String>,
    load_generation: u64,
    favorite_saving: bool,
    deleting: bool,
    scroll_handle: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl SessionLogsPage {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("SessionLogs.search_placeholder").to_string())
                .clean_on_escape()
        });
        let subscription = cx.subscribe(&search_input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let directory = one_core::app_dirs::data_dir().map(session_logs_directory);
        let mut page = Self {
            focus_handle: cx.focus_handle(),
            search_input,
            directory,
            catalog: SessionLogCatalog::default(),
            favorites: SessionLogFavorites::default(),
            loading: false,
            load_error: None,
            load_generation: 0,
            favorite_saving: false,
            deleting: false,
            scroll_handle: UniformListScrollHandle::default(),
            _subscriptions: vec![subscription],
        };
        page.refresh(cx);
        Self::start_auto_refresh(cx);
        page
    }

    fn start_auto_refresh(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(SESSION_LOG_REFRESH_INTERVAL).await;
                if this
                    .update(cx, |this, cx| {
                        this.refresh(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.favorite_saving || self.deleting {
            return;
        }
        self.load_generation = self.load_generation.wrapping_add(1);
        let generation = self.load_generation;
        self.loading = true;
        self.load_error = None;
        let Some(directory) = self.directory.clone() else {
            self.loading = false;
            self.load_error = Some(t!("SessionLogs.data_directory_unavailable").to_string());
            cx.notify();
            return;
        };
        let load_task = cx.background_spawn(async move {
            let favorites =
                load_session_log_favorites(&directory).map_err(|error| error.to_string())?;
            let catalog = scan_session_logs(&directory, RecordingFileLimits::default(), &favorites)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((favorites, catalog))
        });
        cx.spawn(async move |this, cx| {
            let result = load_task.await;
            _ = this.update(cx, |this, cx| {
                this.apply_refresh(generation, result, cx);
            });
        })
        .detach();
    }

    fn begin_favorite_save(&mut self) {
        self.load_generation = self.load_generation.wrapping_add(1);
        self.loading = false;
        self.favorite_saving = true;
    }

    pub(super) fn begin_delete(&mut self) {
        self.load_generation = self.load_generation.wrapping_add(1);
        self.loading = false;
        self.deleting = true;
    }

    pub(super) fn finish_delete_state(&mut self) {
        self.deleting = false;
    }

    fn apply_refresh(
        &mut self,
        generation: u64,
        result: Result<(SessionLogFavorites, SessionLogCatalog), String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.load_generation {
            return;
        }
        self.loading = false;
        match result {
            Ok((favorites, catalog)) => {
                self.favorites = favorites;
                self.catalog = catalog;
            }
            Err(error) => self.load_error = Some(error),
        }
        cx.notify();
    }

    fn filtered_entries(&self, cx: &App) -> Vec<SessionLogEntry> {
        let query = self.search_input.read(cx).value();
        self.catalog
            .entries
            .iter()
            .filter(|entry| model::session_log_matches(entry, &query))
            .cloned()
            .collect()
    }
}

impl Focusable for SessionLogsPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for SessionLogsPage {}

impl TabContent for SessionLogsPage {
    fn content_key(&self) -> &'static str {
        "SessionLogs"
    }

    fn title(&self, _cx: &App) -> SharedString {
        t!("SessionLogs.title").to_string().into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::Terminal.color())
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn can_rename(&self, _cx: &App) -> bool {
        false
    }
}
