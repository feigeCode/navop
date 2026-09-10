use std::collections::HashSet;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;

use gpui::{DevicePixels, DynamicTexture, DynamicTextureId, size};
use remote_desktop::{RemoteDesktopFrameRect, RgbaFramebuffer};

const MAX_TEXTURE_UPLOAD_RECTS: usize = 16;
const SIMPLIFY_TEXTURE_UPLOADS_THRESHOLD: usize = MAX_TEXTURE_UPLOAD_RECTS;
const MAX_MERGED_AREA_FACTOR: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TextureRect {
    pub(super) x: u16,
    pub(super) y: u16,
    pub(super) width: u16,
    pub(super) height: u16,
}

impl TextureRect {
    fn full(width: u16, height: u16) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    fn pixels(self) -> u64 {
        u64::from(self.width).saturating_mul(u64::from(self.height))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingTextureUpdate {
    sequence_ids: Vec<u64>,
    rect: TextureRect,
}

struct PreparedDirtyUpdate {
    rect: TextureRect,
    bytes: Arc<Vec<u8>>,
}

struct PreparedDirtyBatch<'a> {
    framebuffer_bytes: &'a [u8],
    framebuffer_row_bytes: usize,
    updates: Vec<PreparedDirtyUpdate>,
}

#[derive(Debug)]
struct RemoteDesktopSurfaceState {
    backing_bgra: Arc<Vec<u8>>,
    pending_updates: Vec<PendingTextureUpdate>,
    prepared_uploads: Arc<Vec<TextureUpload>>,
    next_sequence_id: u64,
    confirmed_renderer_resource_generation: Option<u64>,
}

#[derive(Clone, Debug)]
pub(super) struct TextureUpload {
    pub(super) rect: TextureRect,
    pub(super) bytes: Arc<Vec<u8>>,
    sequence_ids: Vec<u64>,
    renderer_resource_generation: Option<u64>,
    texture_id: DynamicTextureId,
}

#[derive(Clone)]
pub(super) struct RemoteDesktopSurface {
    id: usize,
    width: u16,
    height: u16,
    texture: Arc<DynamicTexture>,
    state: Arc<Mutex<RemoteDesktopSurfaceState>>,
}

#[derive(Default)]
pub(super) struct RetiredTextureQueue {
    textures: Vec<Arc<DynamicTexture>>,
}

impl RetiredTextureQueue {
    pub(super) fn retire(&mut self, surface: Arc<RemoteDesktopSurface>) {
        let texture = surface.texture().clone();
        if self.textures.iter().all(|current| current.id != texture.id) {
            self.textures.push(texture);
        }
    }

    pub(super) fn retire_all(
        &mut self,
        surfaces: impl IntoIterator<Item = Arc<RemoteDesktopSurface>>,
    ) {
        for surface in surfaces {
            self.retire(surface);
        }
    }

    pub(super) fn take_releasable(&mut self) -> Vec<Arc<DynamicTexture>> {
        let mut pending = Vec::with_capacity(self.textures.len());
        let mut releasable = Vec::new();
        for texture in self.textures.drain(..) {
            if Arc::strong_count(&texture) == 1 {
                releasable.push(texture);
            } else {
                pending.push(texture);
            }
        }
        self.textures = pending;
        releasable
    }

    pub(super) fn take_all(&mut self) -> Vec<Arc<DynamicTexture>> {
        std::mem::take(&mut self.textures)
    }
}

impl PartialEq for RemoteDesktopSurface {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for RemoteDesktopSurface {}

impl RemoteDesktopSurface {
    pub(super) fn from_framebuffer(framebuffer: &RgbaFramebuffer) -> anyhow::Result<Self> {
        let width = framebuffer.width();
        let height = framebuffer.height();
        validate_framebuffer(framebuffer)?;
        let texture = Arc::new(DynamicTexture::new(size(
            DevicePixels(i32::from(width)),
            DevicePixels(i32::from(height)),
        )));
        let full_rect = TextureRect::full(width, height);
        let backing_bgra = Arc::new(framebuffer.as_rgba().to_vec());
        let prepared_uploads = Arc::new(vec![TextureUpload {
            rect: full_rect,
            bytes: Arc::clone(&backing_bgra),
            sequence_ids: vec![0],
            renderer_resource_generation: None,
            texture_id: texture.id,
        }]);

        Ok(Self {
            id: next_surface_id(),
            width,
            height,
            texture,
            state: Arc::new(Mutex::new(RemoteDesktopSurfaceState {
                backing_bgra,
                pending_updates: vec![PendingTextureUpdate {
                    sequence_ids: vec![0],
                    rect: full_rect,
                }],
                prepared_uploads,
                next_sequence_id: 1,
                confirmed_renderer_resource_generation: None,
            })),
        })
    }

    pub(super) fn with_full_framebuffer(
        &self,
        framebuffer: &RgbaFramebuffer,
    ) -> anyhow::Result<Self> {
        self.validate_matching_framebuffer(framebuffer)?;
        let backing_bgra = Arc::new(framebuffer.as_rgba().to_vec());
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("remote desktop texture state is poisoned"))?;
        let sequence_id = take_next_sequence_id(&mut state)?;
        state.backing_bgra = backing_bgra;
        state.pending_updates.clear();
        state.pending_updates.push(PendingTextureUpdate {
            sequence_ids: vec![sequence_id],
            rect: TextureRect::full(self.width, self.height),
        });
        state.prepared_uploads = Arc::new(vec![TextureUpload {
            rect: TextureRect::full(self.width, self.height),
            bytes: Arc::clone(&state.backing_bgra),
            sequence_ids: vec![sequence_id],
            renderer_resource_generation: None,
            texture_id: self.texture.id,
        }]);
        drop(state);

        Ok(self.next_presentation())
    }

    pub(super) fn with_dirty_rects(
        &self,
        framebuffer: &RgbaFramebuffer,
        rects: &[RemoteDesktopFrameRect],
    ) -> anyhow::Result<Self> {
        self.validate_matching_framebuffer(framebuffer)?;
        let prepared_updates = self.prepare_dirty_updates(framebuffer, rects)?;
        if prepared_updates.is_empty() {
            return Ok(self.clone());
        }

        let framebuffer_row_bytes = usize::from(self.width)
            .checked_mul(4)
            .ok_or_else(|| anyhow::anyhow!("remote desktop framebuffer row size overflow"))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("remote desktop texture state is poisoned"))?;
        self.apply_dirty_updates(
            &mut state,
            PreparedDirtyBatch {
                framebuffer_bytes: framebuffer.as_rgba(),
                framebuffer_row_bytes,
                updates: prepared_updates,
            },
        )?;
        drop(state);

        Ok(self.next_presentation())
    }

    fn apply_dirty_updates(
        &self,
        state: &mut RemoteDesktopSurfaceState,
        batch: PreparedDirtyBatch<'_>,
    ) -> anyhow::Result<()> {
        validate_backing_len(self.width, self.height, state.backing_bgra.len())?;
        let reusable_uploads = take_reusable_uploads(state);
        let diagnostics = super::remote_desktop_diagnostics_enabled().then(|| {
            let dirty_pixels = batch
                .updates
                .iter()
                .map(|update| update.rect.pixels())
                .sum::<u64>();
            (
                Instant::now(),
                Arc::strong_count(&state.backing_bgra),
                batch.updates.len(),
                dirty_pixels,
            )
        });
        for prepared in batch.updates {
            let backing_bgra = Arc::make_mut(&mut state.backing_bgra);
            copy_rect_between_bgra_buffers(
                batch.framebuffer_bytes,
                backing_bgra,
                batch.framebuffer_row_bytes,
                prepared.rect,
            );
            let sequence_id = take_next_sequence_id(state)?;
            state.pending_updates.push(PendingTextureUpdate {
                sequence_ids: vec![sequence_id],
                rect: prepared.rect,
            });
            Arc::make_mut(&mut state.prepared_uploads).push(TextureUpload {
                rect: prepared.rect,
                bytes: prepared.bytes,
                sequence_ids: vec![sequence_id],
                renderer_resource_generation: None,
                texture_id: self.texture.id,
            });
        }
        Arc::make_mut(&mut state.prepared_uploads).extend(reusable_uploads.iter().cloned());
        if state.pending_updates.len() > SIMPLIFY_TEXTURE_UPLOADS_THRESHOLD {
            simplify_pending_texture_regions(&mut state.pending_updates, self.width, self.height);
        }
        state.prepared_uploads =
            self.prepare_pending_uploads(&state, state.prepared_uploads.as_slice())?;
        if let Some((apply_started_at, backing_reference_count, dirty_rects, dirty_pixels)) =
            diagnostics
        {
            let framebuffer_pixels = u64::from(self.width).saturating_mul(u64::from(self.height));
            tracing::info!(
                surface_id = self.id,
                surface_width = self.width,
                surface_height = self.height,
                dirty_rects,
                dirty_pixels,
                dirty_ratio_per_mille = dirty_pixels
                    .saturating_mul(1000)
                    .checked_div(framebuffer_pixels)
                    .unwrap_or_default(),
                backing_reference_count,
                backing_cow = backing_reference_count > 1,
                backing_copy_bytes = if backing_reference_count > 1 {
                    state.backing_bgra.len()
                } else {
                    0
                },
                pending_uploads = state.prepared_uploads.len(),
                apply_us = apply_started_at.elapsed().as_micros() as u64,
                "remote desktop surface dirty batch prepared"
            );
        }
        Ok(())
    }

    fn prepare_dirty_updates(
        &self,
        framebuffer: &RgbaFramebuffer,
        rects: &[RemoteDesktopFrameRect],
    ) -> anyhow::Result<Vec<PreparedDirtyUpdate>> {
        rects
            .iter()
            .map(|rect| {
                let rect = validate_dirty_rect(self.width, self.height, rect)?;
                let bytes = copy_bgra_rect_from_backing(
                    framebuffer.as_rgba(),
                    self.width,
                    self.height,
                    rect,
                )?;
                Ok(PreparedDirtyUpdate {
                    rect,
                    bytes: Arc::new(bytes),
                })
            })
            .collect()
    }

    pub(super) fn width(&self) -> u16 {
        self.width
    }

    pub(super) fn height(&self) -> u16 {
        self.height
    }

    pub(super) fn texture(&self) -> &Arc<DynamicTexture> {
        &self.texture
    }

    pub(super) fn pending_texture_uploads(
        &self,
        renderer_resource_generation: u64,
    ) -> Arc<Vec<TextureUpload>> {
        let Ok(state) = self.state.lock() else {
            tracing::warn!(
                surface_id = self.id,
                "remote desktop texture state is poisoned while reading uploads"
            );
            return Arc::new(Vec::new());
        };

        let renderer_resource_generation = (state.confirmed_renderer_resource_generation
            != Some(renderer_resource_generation))
        .then_some(renderer_resource_generation);
        if renderer_resource_generation.is_none() {
            return Arc::clone(&state.prepared_uploads);
        }

        Arc::new(vec![TextureUpload {
            rect: TextureRect::full(self.width, self.height),
            bytes: Arc::clone(&state.backing_bgra),
            sequence_ids: pending_sequence_ids(&state.pending_updates),
            renderer_resource_generation,
            texture_id: self.texture.id,
        }])
    }

    #[cfg(test)]
    pub(super) fn acknowledge_texture_upload(&self, upload: &TextureUpload) {
        self.acknowledge_texture_uploads(std::slice::from_ref(upload));
    }

    pub(super) fn acknowledge_texture_uploads(&self, uploads: &[TextureUpload]) {
        let mut has_valid_upload = false;
        let mut uploaded_sequences = HashSet::new();
        let mut renderer_resource_generation = None;
        for upload in uploads {
            if upload.texture_id != self.texture.id {
                continue;
            }
            has_valid_upload = true;
            uploaded_sequences.extend(upload.sequence_ids.iter().copied());
            renderer_resource_generation =
                renderer_resource_generation.max(upload.renderer_resource_generation);
        }
        if !has_valid_upload {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            tracing::warn!(
                surface_id = self.id,
                "remote desktop texture state is poisoned while acknowledging uploads"
            );
            return;
        };
        if !uploaded_sequences.is_empty() {
            remove_uploaded_sequences(&mut state.pending_updates, &uploaded_sequences);
            remove_uploaded_uploads(&mut state.prepared_uploads, &uploaded_sequences);
        }
        if let Some(renderer_resource_generation) = renderer_resource_generation {
            match state.confirmed_renderer_resource_generation {
                Some(confirmed) if renderer_resource_generation < confirmed => {}
                _ => {
                    state.confirmed_renderer_resource_generation =
                        Some(renderer_resource_generation);
                }
            }
        }
    }

    fn validate_matching_framebuffer(&self, framebuffer: &RgbaFramebuffer) -> anyhow::Result<()> {
        validate_framebuffer(framebuffer)?;
        anyhow::ensure!(
            self.width == framebuffer.width() && self.height == framebuffer.height(),
            "remote desktop surface dimensions changed"
        );
        Ok(())
    }

    fn next_presentation(&self) -> Self {
        Self {
            id: next_surface_id(),
            width: self.width,
            height: self.height,
            texture: self.texture.clone(),
            state: self.state.clone(),
        }
    }

    fn prepare_pending_uploads(
        &self,
        state: &RemoteDesktopSurfaceState,
        reusable_uploads: &[TextureUpload],
    ) -> anyhow::Result<Arc<Vec<TextureUpload>>> {
        state
            .pending_updates
            .iter()
            .map(|update| {
                let bytes = reusable_uploads
                    .iter()
                    .find(|upload| {
                        !upload.bytes.is_empty()
                            && upload.rect == update.rect
                            && upload.sequence_ids == update.sequence_ids
                    })
                    .map(|upload| Arc::clone(&upload.bytes))
                    .map(Ok)
                    .unwrap_or_else(|| self.prepare_upload_bytes(state, update))?;
                Ok(TextureUpload {
                    rect: update.rect,
                    bytes,
                    sequence_ids: update.sequence_ids.clone(),
                    renderer_resource_generation: None,
                    texture_id: self.texture.id,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .map(Arc::new)
    }

    fn prepare_upload_bytes(
        &self,
        state: &RemoteDesktopSurfaceState,
        update: &PendingTextureUpdate,
    ) -> anyhow::Result<Arc<Vec<u8>>> {
        if update.rect == TextureRect::full(self.width, self.height) {
            return Ok(Arc::clone(&state.backing_bgra));
        }
        copy_bgra_rect_from_backing(
            state.backing_bgra.as_slice(),
            self.width,
            self.height,
            update.rect,
        )
        .map(Arc::new)
    }
}

fn next_surface_id() -> usize {
    static NEXT_SURFACE_ID: AtomicUsize = AtomicUsize::new(0);
    NEXT_SURFACE_ID
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |id| id.checked_add(1))
        .expect("remote desktop surface identifier space exhausted")
}

fn validate_framebuffer(framebuffer: &RgbaFramebuffer) -> anyhow::Result<()> {
    anyhow::ensure!(
        framebuffer.width() > 0 && framebuffer.height() > 0,
        "remote desktop frame is empty"
    );
    validate_backing_len(
        framebuffer.width(),
        framebuffer.height(),
        framebuffer.as_rgba().len(),
    )
}

fn validate_backing_len(width: u16, height: u16, actual_len: usize) -> anyhow::Result<()> {
    let expected_len = usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("remote desktop framebuffer size overflow"))?;
    anyhow::ensure!(
        actual_len == expected_len,
        "remote desktop framebuffer byte length is invalid"
    );
    Ok(())
}

fn validate_dirty_rect(
    width: u16,
    height: u16,
    rect: &RemoteDesktopFrameRect,
) -> anyhow::Result<TextureRect> {
    anyhow::ensure!(
        rect.width > 0 && rect.height > 0,
        "dirty rectangle is empty"
    );
    let right = usize::from(rect.x)
        .checked_add(usize::from(rect.width))
        .ok_or_else(|| anyhow::anyhow!("dirty rectangle width overflows framebuffer"))?;
    let bottom = usize::from(rect.y)
        .checked_add(usize::from(rect.height))
        .ok_or_else(|| anyhow::anyhow!("dirty rectangle height overflows framebuffer"))?;
    anyhow::ensure!(
        right <= usize::from(width),
        "dirty rectangle width exceeds framebuffer"
    );
    anyhow::ensure!(
        bottom <= usize::from(height),
        "dirty rectangle height exceeds framebuffer"
    );
    let expected_len = usize::from(rect.width)
        .checked_mul(usize::from(rect.height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("dirty rectangle size overflow"))?;
    anyhow::ensure!(
        rect.byte_len == expected_len,
        "dirty rectangle declared length is invalid"
    );
    Ok(TextureRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    })
}

fn take_next_sequence_id(state: &mut RemoteDesktopSurfaceState) -> anyhow::Result<u64> {
    let sequence_id = state.next_sequence_id;
    state.next_sequence_id = state
        .next_sequence_id
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("remote desktop texture update sequence exhausted"))?;
    Ok(sequence_id)
}

fn take_reusable_uploads(state: &mut RemoteDesktopSurfaceState) -> Arc<Vec<TextureUpload>> {
    let mut reusable_uploads = std::mem::replace(&mut state.prepared_uploads, Arc::new(Vec::new()));
    for upload in Arc::make_mut(&mut reusable_uploads) {
        if Arc::ptr_eq(&upload.bytes, &state.backing_bgra) {
            upload.bytes = Arc::new(Vec::new());
        }
    }
    reusable_uploads
}

fn copy_rect_between_bgra_buffers(
    source: &[u8],
    destination: &mut [u8],
    framebuffer_row_bytes: usize,
    rect: TextureRect,
) {
    let row_bytes = usize::from(rect.width) * 4;
    for row in 0..usize::from(rect.height) {
        let start = (usize::from(rect.y) + row) * framebuffer_row_bytes + usize::from(rect.x) * 4;
        destination[start..start + row_bytes].copy_from_slice(&source[start..start + row_bytes]);
    }
}

fn copy_bgra_rect_from_backing(
    backing_bgra: &[u8],
    width: u16,
    height: u16,
    rect: TextureRect,
) -> anyhow::Result<Vec<u8>> {
    validate_backing_len(width, height, backing_bgra.len())?;
    anyhow::ensure!(
        rect.width > 0 && rect.height > 0,
        "texture update rectangle is empty"
    );
    let right = usize::from(rect.x)
        .checked_add(usize::from(rect.width))
        .ok_or_else(|| anyhow::anyhow!("texture update width overflow"))?;
    let bottom = usize::from(rect.y)
        .checked_add(usize::from(rect.height))
        .ok_or_else(|| anyhow::anyhow!("texture update height overflow"))?;
    anyhow::ensure!(
        right <= usize::from(width) && bottom <= usize::from(height),
        "texture update exceeds framebuffer"
    );

    let row_bytes = usize::from(rect.width)
        .checked_mul(4)
        .ok_or_else(|| anyhow::anyhow!("texture update row size overflow"))?;
    let capacity = row_bytes
        .checked_mul(usize::from(rect.height))
        .ok_or_else(|| anyhow::anyhow!("texture update size overflow"))?;
    let framebuffer_row_bytes = usize::from(width) * 4;
    let mut bytes = Vec::with_capacity(capacity);
    for row in 0..usize::from(rect.height) {
        let start = (usize::from(rect.y) + row) * framebuffer_row_bytes + usize::from(rect.x) * 4;
        bytes.extend_from_slice(&backing_bgra[start..start + row_bytes]);
    }
    Ok(bytes)
}

fn pending_sequence_ids(updates: &[PendingTextureUpdate]) -> Vec<u64> {
    let mut sequence_ids = updates
        .iter()
        .flat_map(|update| update.sequence_ids.iter().copied())
        .collect::<Vec<_>>();
    sequence_ids.sort_unstable();
    sequence_ids.dedup();
    sequence_ids
}

fn remove_uploaded_sequences(
    updates: &mut Vec<PendingTextureUpdate>,
    uploaded_sequences: &HashSet<u64>,
) {
    for update in updates.iter_mut() {
        update
            .sequence_ids
            .retain(|sequence_id| !uploaded_sequences.contains(sequence_id));
    }
    updates.retain(|update| !update.sequence_ids.is_empty());
}

fn remove_uploaded_uploads(
    uploads: &mut Arc<Vec<TextureUpload>>,
    uploaded_sequences: &HashSet<u64>,
) {
    let uploads = Arc::make_mut(uploads);
    for upload in uploads.iter_mut() {
        upload
            .sequence_ids
            .retain(|sequence_id| !uploaded_sequences.contains(sequence_id));
    }
    uploads.retain(|upload| !upload.sequence_ids.is_empty());
}

fn simplify_pending_texture_regions(
    pending_updates: &mut Vec<PendingTextureUpdate>,
    width: u16,
    height: u16,
) {
    if pending_updates.len() <= 1 {
        return;
    }
    merge_touching_texture_rects(pending_updates);
    if pending_updates.len() > MAX_TEXTURE_UPLOAD_RECTS {
        merge_texture_rects_to_limit(pending_updates, MAX_TEXTURE_UPLOAD_RECTS);
    }

    let pending_pixels = pending_updates
        .iter()
        .map(|update| update.rect.pixels())
        .sum::<u64>();
    let frame_pixels = u64::from(width).saturating_mul(u64::from(height));
    if pending_pixels >= frame_pixels {
        let sequence_ids = pending_sequence_ids(pending_updates);
        pending_updates.clear();
        pending_updates.push(PendingTextureUpdate {
            sequence_ids,
            rect: TextureRect::full(width, height),
        });
    }
}

fn merge_touching_texture_rects(updates: &mut Vec<PendingTextureUpdate>) {
    sort_texture_updates(updates);
    let mut index = 0;
    while index < updates.len() {
        let mut next_index = index + 1;
        while next_index < updates.len() {
            if can_merge_texture_rects(updates[index].rect, updates[next_index].rect) {
                updates[index] =
                    merged_texture_update(updates[index].clone(), updates[next_index].clone());
                updates.remove(next_index);
                sort_texture_updates(updates);
                index = 0;
                next_index = 1;
            } else {
                next_index += 1;
            }
        }
        index += 1;
    }
}

fn merge_texture_rects_to_limit(updates: &mut Vec<PendingTextureUpdate>, limit: usize) {
    if limit == 0 {
        updates.clear();
        return;
    }
    sort_texture_updates(updates);
    while updates.len() > limit {
        let Some(best_index) = best_neighbor_merge_index(updates) else {
            break;
        };
        updates[best_index] =
            merged_texture_update(updates[best_index].clone(), updates[best_index + 1].clone());
        updates.remove(best_index + 1);
        merge_touching_texture_rects(updates);
    }
}

fn best_neighbor_merge_index(updates: &[PendingTextureUpdate]) -> Option<usize> {
    updates
        .windows(2)
        .enumerate()
        .min_by_key(|(_, pair)| {
            let merged = bounding_texture_rect(pair[0].rect, pair[1].rect);
            let merged_area = merged.pixels();
            let pair_area = pair[0].rect.pixels().saturating_add(pair[1].rect.pixels());
            (
                merged_area.saturating_sub(pair_area),
                merged_area,
                merged.y,
                merged.x,
            )
        })
        .map(|(index, _)| index)
}

fn sort_texture_updates(updates: &mut [PendingTextureUpdate]) {
    updates.sort_by_key(|update| {
        (
            update.rect.y,
            update.rect.x,
            update.rect.height,
            update.rect.width,
        )
    });
}

fn merged_texture_update(
    mut first: PendingTextureUpdate,
    second: PendingTextureUpdate,
) -> PendingTextureUpdate {
    first.rect = bounding_texture_rect(first.rect, second.rect);
    first.sequence_ids.extend(second.sequence_ids);
    first.sequence_ids.sort_unstable();
    first.sequence_ids.dedup();
    first
}

fn can_merge_texture_rects(first: TextureRect, second: TextureRect) -> bool {
    if !texture_rects_touch_or_overlap(first, second) {
        return false;
    }
    let merged_area = bounding_texture_rect(first, second).pixels();
    let source_area = first.pixels().saturating_add(second.pixels());
    merged_area <= source_area.saturating_mul(MAX_MERGED_AREA_FACTOR)
}

fn texture_rects_touch_or_overlap(first: TextureRect, second: TextureRect) -> bool {
    let first_right = u64::from(first.x) + u64::from(first.width);
    let second_right = u64::from(second.x) + u64::from(second.width);
    let first_bottom = u64::from(first.y) + u64::from(first.height);
    let second_bottom = u64::from(second.y) + u64::from(second.height);
    u64::from(first.x) <= second_right
        && u64::from(second.x) <= first_right
        && u64::from(first.y) <= second_bottom
        && u64::from(second.y) <= first_bottom
}

fn bounding_texture_rect(first: TextureRect, second: TextureRect) -> TextureRect {
    let x = first.x.min(second.x);
    let y = first.y.min(second.y);
    let right = first
        .x
        .saturating_add(first.width)
        .max(second.x.saturating_add(second.width));
    let bottom = first
        .y
        .saturating_add(first.height)
        .max(second.y.saturating_add(second.height));
    TextureRect {
        x,
        y,
        width: right.saturating_sub(x),
        height: bottom.saturating_sub(y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framebuffer(width: u16, height: u16) -> RgbaFramebuffer {
        let mut bytes = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
        for index in 0..usize::from(width) * usize::from(height) {
            bytes.extend_from_slice(&[index as u8, 0, 0, 255]);
        }
        RgbaFramebuffer::from_bgra(width, height, bytes).unwrap()
    }

    fn acknowledge_all(surface: &RemoteDesktopSurface, renderer_generation: u64) {
        let uploads = surface.pending_texture_uploads(renderer_generation);
        surface.acknowledge_texture_uploads(uploads.as_slice());
    }

    #[test]
    fn full_surface_uses_one_dynamic_texture_and_full_upload() {
        let framebuffer = framebuffer(257, 2);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();

        assert_eq!((257, 2), (surface.width(), surface.height()));
        assert_eq!(
            size(DevicePixels(257), DevicePixels(2)),
            surface.texture().size()
        );
        let uploads = surface.pending_texture_uploads(0);
        assert_eq!(1, uploads.len());
        assert_eq!(TextureRect::full(257, 2), uploads[0].rect);
        assert_eq!(framebuffer.as_rgba(), uploads[0].bytes.as_slice());
    }

    #[test]
    fn dirty_update_reuses_texture_and_uploads_only_the_dirty_region() {
        let mut framebuffer = framebuffer(3, 2);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        acknowledge_all(&surface, 0);
        let texture_id = surface.texture().id;
        let rect = RemoteDesktopFrameRect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
            byte_len: 16,
        };
        framebuffer
            .patch_rgba_rect(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                &[9, 8, 7, 255, 6, 5, 4, 255, 3, 2, 1, 255, 2, 3, 4, 255],
            )
            .unwrap();

        let updated = surface.with_dirty_rects(&framebuffer, &[rect]).unwrap();
        let uploads = updated.pending_texture_uploads(0);

        assert_eq!(texture_id, updated.texture().id);
        assert_eq!(1, uploads.len());
        assert_eq!(
            TextureRect {
                x: 1,
                y: 0,
                width: 2,
                height: 2,
            },
            uploads[0].rect
        );
        assert_eq!(
            &[9, 8, 7, 255, 6, 5, 4, 255, 3, 2, 1, 255, 2, 3, 4, 255],
            uploads[0].bytes.as_slice()
        );
    }

    #[test]
    fn texture_upload_payload_is_reused_between_paint_queries() {
        let mut framebuffer = framebuffer(2, 1);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        acknowledge_all(&surface, 0);
        let rect = RemoteDesktopFrameRect {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        };
        framebuffer
            .patch_rgba_rect(rect.x, rect.y, rect.width, rect.height, &[9, 8, 7, 255])
            .unwrap();
        let updated = surface.with_dirty_rects(&framebuffer, &[rect]).unwrap();

        let first_uploads = updated.pending_texture_uploads(0);
        let second_uploads = updated.pending_texture_uploads(0);

        assert!(
            Arc::ptr_eq(&first_uploads, &second_uploads),
            "paint queries must reuse the prepared upload batch"
        );
        assert_eq!(
            first_uploads[0].bytes.as_ptr(),
            second_uploads[0].bytes.as_ptr(),
            "paint queries must reuse payloads prepared by the presentation worker"
        );
    }

    #[test]
    fn renderer_reset_full_upload_reuses_backing_payload() {
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer(2, 1)).unwrap();

        let first_uploads = surface.pending_texture_uploads(7);
        let second_uploads = surface.pending_texture_uploads(7);

        assert_eq!(1, first_uploads.len());
        assert_eq!(TextureRect::full(2, 1), first_uploads[0].rect);
        assert!(Arc::ptr_eq(
            &first_uploads[0].bytes,
            &second_uploads[0].bytes
        ));
    }

    #[test]
    fn old_upload_acknowledgement_does_not_clear_newer_dirty_updates() {
        let mut framebuffer = framebuffer(2, 1);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        let old_upload = surface.pending_texture_uploads(7)[0].clone();
        let rect = RemoteDesktopFrameRect {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        };
        framebuffer
            .patch_rgba_rect(rect.x, rect.y, rect.width, rect.height, &[9, 8, 7, 255])
            .unwrap();
        let updated = surface.with_dirty_rects(&framebuffer, &[rect]).unwrap();

        updated.acknowledge_texture_upload(&old_upload);
        let uploads = updated.pending_texture_uploads(7);

        assert_eq!(1, uploads.len());
        assert_eq!(
            TextureRect {
                x: 1,
                y: 0,
                width: 1,
                height: 1,
            },
            uploads[0].rect
        );
        assert_eq!(&[9, 8, 7, 255], uploads[0].bytes.as_slice());
    }

    #[test]
    fn live_dirty_snapshot_stays_consistent_while_new_damage_is_patched() {
        let mut framebuffer = framebuffer(2, 1);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        acknowledge_all(&surface, 0);
        let first_rect = RemoteDesktopFrameRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        };
        framebuffer
            .patch_rgba_rect(
                first_rect.x,
                first_rect.y,
                first_rect.width,
                first_rect.height,
                &[9, 8, 7, 255],
            )
            .unwrap();
        let first_update = surface
            .with_dirty_rects(&framebuffer, &[first_rect])
            .unwrap();
        let old_uploads = first_update.pending_texture_uploads(0);

        let second_rect = RemoteDesktopFrameRect {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        };
        framebuffer
            .patch_rgba_rect(
                second_rect.x,
                second_rect.y,
                second_rect.width,
                second_rect.height,
                &[6, 5, 4, 255],
            )
            .unwrap();
        let second_update = first_update
            .with_dirty_rects(&framebuffer, &[second_rect])
            .unwrap();

        assert_eq!(1, old_uploads.len());
        assert_eq!(
            TextureRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            old_uploads[0].rect
        );
        assert_eq!(&[9, 8, 7, 255], old_uploads[0].bytes.as_slice());

        second_update.acknowledge_texture_upload(&old_uploads[0]);
        let new_uploads = second_update.pending_texture_uploads(0);
        assert_eq!(1, new_uploads.len());
        assert_eq!(
            TextureRect {
                x: 1,
                y: 0,
                width: 1,
                height: 1,
            },
            new_uploads[0].rect
        );
        assert_eq!(&[6, 5, 4, 255], new_uploads[0].bytes.as_slice());
    }

    #[test]
    fn full_replacement_does_not_mutate_a_live_dirty_snapshot() {
        let mut framebuffer = framebuffer(2, 1);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        acknowledge_all(&surface, 0);
        let rect = RemoteDesktopFrameRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        };
        framebuffer
            .patch_rgba_rect(rect.x, rect.y, rect.width, rect.height, &[9, 8, 7, 255])
            .unwrap();
        let dirty_update = surface.with_dirty_rects(&framebuffer, &[rect]).unwrap();
        let old_uploads = dirty_update.pending_texture_uploads(0);

        let replacement_bytes = vec![1, 2, 3, 255, 4, 5, 6, 255];
        let replacement_framebuffer =
            RgbaFramebuffer::from_bgra(2, 1, replacement_bytes.clone()).unwrap();
        let full_update = dirty_update
            .with_full_framebuffer(&replacement_framebuffer)
            .unwrap();

        assert_eq!(1, old_uploads.len());
        assert_eq!(
            TextureRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            old_uploads[0].rect
        );
        assert_eq!(&[9, 8, 7, 255], old_uploads[0].bytes.as_slice());

        full_update.acknowledge_texture_upload(&old_uploads[0]);
        let replacement_uploads = full_update.pending_texture_uploads(0);
        assert_eq!(1, replacement_uploads.len());
        assert_eq!(TextureRect::full(2, 1), replacement_uploads[0].rect);
        assert_eq!(
            replacement_bytes.as_slice(),
            replacement_uploads[0].bytes.as_slice()
        );
    }

    #[test]
    fn renderer_resource_reset_forces_a_full_upload() {
        let mut framebuffer = framebuffer(3, 1);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        acknowledge_all(&surface, 4);
        let rect = RemoteDesktopFrameRect {
            x: 2,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        };
        framebuffer
            .patch_rgba_rect(rect.x, rect.y, rect.width, rect.height, &[9, 8, 7, 255])
            .unwrap();
        let updated = surface.with_dirty_rects(&framebuffer, &[rect]).unwrap();

        let uploads = updated.pending_texture_uploads(5);

        assert_eq!(1, uploads.len());
        assert_eq!(TextureRect::full(3, 1), uploads[0].rect);
        assert_eq!(framebuffer.as_rgba(), uploads[0].bytes.as_slice());
    }

    #[test]
    fn stale_renderer_generation_ack_does_not_replace_newer_confirmation() {
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer(2, 1)).unwrap();
        let generation_four = surface.pending_texture_uploads(4)[0].clone();
        let generation_five = surface.pending_texture_uploads(5)[0].clone();

        surface.acknowledge_texture_upload(&generation_five);
        surface.acknowledge_texture_upload(&generation_four);

        assert!(surface.pending_texture_uploads(5).is_empty());
    }

    #[test]
    fn resize_creates_a_new_texture() {
        let first = RemoteDesktopSurface::from_framebuffer(&framebuffer(2, 1)).unwrap();
        let resized = RemoteDesktopSurface::from_framebuffer(&framebuffer(3, 1)).unwrap();

        assert_ne!(first.texture().id, resized.texture().id);
    }

    #[test]
    fn retired_texture_waits_until_all_surface_owners_are_gone() {
        let first = Arc::new(RemoteDesktopSurface::from_framebuffer(&framebuffer(2, 1)).unwrap());
        let second = Arc::new(first.with_full_framebuffer(&framebuffer(2, 1)).unwrap());
        let texture_id = first.texture().id;
        let mut retired = RetiredTextureQueue::default();

        retired.retire(first);
        retired.retire(second.clone());

        assert!(retired.take_releasable().is_empty());
        assert_eq!(
            1,
            retired.textures.len(),
            "the texture must be deduplicated"
        );

        drop(second);
        let releasable = retired.take_releasable();

        assert_eq!(1, releasable.len());
        assert_eq!(texture_id, releasable[0].id);
        assert!(retired.textures.is_empty());
    }

    #[test]
    fn excessive_damage_is_bounded_and_preserves_all_sequences() {
        let mut framebuffer = framebuffer(256, 2);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        acknowledge_all(&surface, 0);
        let mut current = surface;
        for x in (0..256).step_by(2) {
            let rect = RemoteDesktopFrameRect {
                x,
                y: 0,
                width: 1,
                height: 1,
                byte_len: 4,
            };
            framebuffer
                .patch_rgba_rect(rect.x, rect.y, rect.width, rect.height, &[9, 8, 7, 255])
                .unwrap();
            current = current.with_dirty_rects(&framebuffer, &[rect]).unwrap();
        }

        let uploads = current.pending_texture_uploads(0);

        assert!(uploads.len() <= MAX_TEXTURE_UPLOAD_RECTS);
        assert_eq!(
            128,
            uploads
                .iter()
                .map(|upload| upload.sequence_ids.len())
                .sum::<usize>()
        );
    }

    #[test]
    fn invalid_dirty_rect_is_rejected_without_panicking() {
        let framebuffer = framebuffer(2, 1);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        let rect = RemoteDesktopFrameRect {
            x: 2,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        };

        assert!(surface.with_dirty_rects(&framebuffer, &[rect]).is_err());
    }

    #[test]
    fn empty_dirty_rect_is_rejected_without_panicking() {
        let framebuffer = framebuffer(2, 1);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        let rect = RemoteDesktopFrameRect {
            x: 0,
            y: 0,
            width: 0,
            height: 1,
            byte_len: 0,
        };

        assert!(surface.with_dirty_rects(&framebuffer, &[rect]).is_err());
    }
}
