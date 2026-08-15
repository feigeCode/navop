use std::cell::RefCell;
use std::sync::Arc;

use gpui::{AppContext, AsyncApp, Context, Hsla, Image, ImageFormat, WeakEntity};

use super::Block;
use crate::{BlockRenderArtifact, BlockRenderKind, BlockRenderRequest, EditorHostTheme};

const DEFAULT_AVAILABLE_WIDTH: f32 = 160.0;
const MAX_CACHE_ENTRIES: usize = 64;

#[derive(Clone, Copy, Debug)]
pub(super) struct HostRenderEnvironment {
    pub(super) available_width: f32,
    pub(super) scale_factor: f32,
}

impl Default for HostRenderEnvironment {
    fn default() -> Self {
        Self {
            available_width: DEFAULT_AVAILABLE_WIDTH,
            scale_factor: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HostRenderKey {
    kind: BlockRenderKind,
    source: String,
    colors: [[u32; 4]; 5],
    available_width: u32,
    scale_factor: u32,
}

impl HostRenderKey {
    fn from_request(request: &BlockRenderRequest) -> Self {
        Self {
            kind: request.kind,
            source: request.source.clone(),
            colors: [
                color_key(request.background),
                color_key(request.foreground),
                color_key(request.border),
                color_key(request.muted),
                color_key(request.accent),
            ],
            available_width: request.available_width.to_bits(),
            scale_factor: request.scale_factor.to_bits(),
        }
    }
}

fn color_key(color: Hsla) -> [u32; 4] {
    [
        color.hue.into_degrees().to_bits(),
        color.saturation.to_bits(),
        color.lightness.to_bits(),
        color.alpha.to_bits(),
    ]
}

#[derive(Clone, Debug)]
pub struct HostRenderedArtifact {
    pub artifact: Arc<BlockRenderArtifact>,
    pub image: Arc<Image>,
}

enum HostRenderState {
    Pending { request_id: u64 },
    Ready(Arc<HostRenderedArtifact>),
    Failed,
}

struct HostRenderEntry {
    key: HostRenderKey,
    state: HostRenderState,
}

#[derive(Clone, Copy)]
struct HostRenderTicket {
    generation: u64,
    request_id: u64,
}

enum HostRenderLookup {
    Missing,
    Pending,
    Failed,
    Ready(Arc<HostRenderedArtifact>),
}

#[derive(Default)]
pub(super) struct HostRenderRuntime {
    generation: u64,
    next_request_id: u64,
    entries: Vec<HostRenderEntry>,
}

impl HostRenderRuntime {
    fn lookup(&self, key: &HostRenderKey) -> HostRenderLookup {
        let Some(entry) = self.entries.iter().find(|entry| &entry.key == key) else {
            return HostRenderLookup::Missing;
        };
        match &entry.state {
            HostRenderState::Ready(artifact) => HostRenderLookup::Ready(artifact.clone()),
            HostRenderState::Pending { .. } => HostRenderLookup::Pending,
            HostRenderState::Failed => HostRenderLookup::Failed,
        }
    }

    fn begin(&mut self, key: HostRenderKey) -> Option<HostRenderTicket> {
        if !matches!(self.lookup(&key), HostRenderLookup::Missing) {
            return None;
        }
        self.trim_for_insert();
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let ticket = HostRenderTicket {
            generation: self.generation,
            request_id: self.next_request_id,
        };
        self.entries.push(HostRenderEntry {
            key,
            state: HostRenderState::Pending {
                request_id: ticket.request_id,
            },
        });
        Some(ticket)
    }

    fn finish(
        &mut self,
        ticket: HostRenderTicket,
        key: &HostRenderKey,
        rendered: Option<Arc<HostRenderedArtifact>>,
    ) -> bool {
        if ticket.generation != self.generation {
            return false;
        }
        let Some(entry) = self.entries.iter_mut().find(|entry| &entry.key == key) else {
            return false;
        };
        if !matches!(
            entry.state,
            HostRenderState::Pending { request_id }
                if request_id == ticket.request_id
        ) {
            return false;
        }
        entry.state = rendered
            .map(HostRenderState::Ready)
            .unwrap_or(HostRenderState::Failed);
        true
    }

    fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.entries.clear();
    }

    fn trim_for_insert(&mut self) {
        if self.entries.len() < MAX_CACHE_ENTRIES {
            return;
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| !matches!(entry.state, HostRenderState::Pending { .. }))
        {
            self.entries.remove(index);
        }
    }
}

fn ready_artifact(
    result: Result<Option<BlockRenderArtifact>, String>,
) -> Option<Arc<HostRenderedArtifact>> {
    let artifact = result.ok().flatten()?;
    if artifact.bytes.is_empty() || !is_svg_media_type(&artifact.media_type) {
        return None;
    }
    let image = Arc::new(Image::from_bytes(ImageFormat::Svg, artifact.bytes.clone()));
    Some(Arc::new(HostRenderedArtifact {
        artifact: Arc::new(artifact),
        image,
    }))
}

fn is_svg_media_type(media_type: &str) -> bool {
    let media_type = media_type
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default();
    media_type.eq_ignore_ascii_case("image/svg+xml") || media_type.eq_ignore_ascii_case("image/svg")
}

impl Block {
    pub(crate) fn set_host_render_environment(&mut self, available_width: f32, scale_factor: f32) {
        self.host_render_environment = HostRenderEnvironment {
            available_width: positive_or(available_width, DEFAULT_AVAILABLE_WIDTH),
            scale_factor: positive_or(scale_factor, 1.0),
        };
    }

    pub(crate) fn host_render_available_width(&self) -> f32 {
        self.host_render_environment.available_width
    }

    pub(crate) fn host_render_request(
        &self,
        kind: BlockRenderKind,
        source: String,
    ) -> BlockRenderRequest {
        let theme = self.host_services.theme();
        request_from_theme(kind, source, theme, self.host_render_environment)
    }

    pub(crate) fn resolve_host_render(
        &self,
        request: BlockRenderRequest,
        cx: &mut Context<Self>,
    ) -> Option<Arc<HostRenderedArtifact>> {
        let provider = self.host_services.block_renderer()?.clone();
        let key = HostRenderKey::from_request(&request);
        match self.host_render_runtime.borrow().lookup(&key) {
            HostRenderLookup::Ready(artifact) => return Some(artifact),
            HostRenderLookup::Pending | HostRenderLookup::Failed => return None,
            HostRenderLookup::Missing => {}
        }
        let ticket = self.host_render_runtime.borrow_mut().begin(key.clone())?;
        let render_task =
            cx.background_spawn(async move { ready_artifact(provider(request).await) });
        cx.spawn(async move |this: WeakEntity<Block>, cx: &mut AsyncApp| {
            let result = render_task.await;
            let _ = this.update(cx, move |block, cx| {
                if block
                    .host_render_runtime
                    .get_mut()
                    .finish(ticket, &key, result)
                {
                    cx.notify();
                }
            });
        })
        .detach();
        None
    }

    pub(crate) fn host_render_is_pending(&self, request: &BlockRenderRequest) -> bool {
        let key = HostRenderKey::from_request(request);
        matches!(
            self.host_render_runtime.borrow().lookup(&key),
            HostRenderLookup::Pending
        )
    }

    pub(crate) fn has_host_render_provider(&self) -> bool {
        self.host_services.block_renderer().is_some()
    }

    pub(crate) fn reset_host_render_runtime(&mut self) {
        self.host_render_runtime.get_mut().reset();
    }
}

fn request_from_theme(
    kind: BlockRenderKind,
    source: String,
    theme: &EditorHostTheme,
    environment: HostRenderEnvironment,
) -> BlockRenderRequest {
    BlockRenderRequest {
        kind,
        source,
        background: theme.background,
        foreground: theme.foreground,
        border: theme.border,
        muted: theme.muted,
        accent: theme.accent,
        available_width: environment.available_width,
        scale_factor: environment.scale_factor,
    }
}

fn positive_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

pub(super) fn new_host_render_runtime() -> RefCell<HostRenderRuntime> {
    RefCell::new(HostRenderRuntime::default())
}
