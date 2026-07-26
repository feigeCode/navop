use super::{
    MarkdownEditSurface, MarkdownInputMode, MarkdownSurfaceKey, SurfaceProjectionUpdate, mode_for,
    projection_for, surface_specs,
};
use crate::MarkdownProjection;
use gpui::{Context, Window};
use markdown_source::SourceSelection;
use std::collections::HashSet;

use super::super::{
    MarkdownEditor,
    projection_styles::projection_highlights,
    setup::{apply_projection_styles, apply_surface_mode, create_input, subscribe_to_input},
    text_diff::minimal_text_patch,
};

struct SurfaceSpec {
    key: MarkdownSurfaceKey,
    projection: MarkdownProjection,
    mode: MarkdownInputMode,
    selection: Option<SourceSelection>,
}

impl MarkdownEditor {
    pub(in crate::editor) fn surface(
        &self,
        key: MarkdownSurfaceKey,
    ) -> Option<&MarkdownEditSurface> {
        self.surfaces.get(&key)
    }

    pub(in crate::editor) fn active_surface_key(&self) -> MarkdownSurfaceKey {
        self.active_surface
            .filter(|key| self.surfaces.contains_key(key))
            .or_else(|| self.active_table_cell.map(MarkdownSurfaceKey::table_cell))
            .or_else(|| self.active_block.map(MarkdownSurfaceKey::block))
            .filter(|key| self.surfaces.contains_key(key))
            .unwrap_or(MarkdownSurfaceKey::Empty)
    }

    pub(in crate::editor) fn set_active_surface(&mut self, key: MarkdownSurfaceKey) -> bool {
        if !self.surfaces.contains_key(&key) {
            return false;
        }
        self.active_surface = Some(key);
        match key {
            MarkdownSurfaceKey::Empty => {
                self.active_block = None;
                self.active_table_cell = None;
            }
            MarkdownSurfaceKey::Block(block_id) => {
                self.active_block = Some(block_id);
                self.active_table_cell = None;
            }
            MarkdownSurfaceKey::TableCell { block_id, .. } => {
                self.active_block = Some(block_id);
                self.active_table_cell = key.table_address();
            }
        }
        self.sync_compatibility_alias();
        true
    }

    pub(in crate::editor) fn initialize_surfaces(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let projection = projection_for(
            self.history.document(),
            MarkdownSurfaceKey::Empty,
            None,
            self.empty_surface_range.clone(),
        )
        .unwrap_or_else(|| self.projection.clone());
        let mode = mode_for(self.history.document(), MarkdownSurfaceKey::Empty);
        apply_surface_mode(&self.input, &mode, window, cx);
        apply_projection_styles(&self.input, &projection, &self.theme, cx);
        let subscriptions = subscribe_to_input(&self.input, MarkdownSurfaceKey::Empty, window, cx);
        self.surfaces.insert(
            MarkdownSurfaceKey::Empty,
            MarkdownEditSurface {
                input: self.input.clone(),
                projection,
                mode,
                _subscriptions: subscriptions,
            },
        );
        self.reconcile_surfaces(window, cx);
    }

    pub(in crate::editor) fn reconcile_surfaces(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let specs = self.desired_surface_specs();
        let desired = specs.iter().map(|spec| spec.key).collect::<HashSet<_>>();
        self.surfaces.retain(|key, _| desired.contains(key));
        self.syncing_input = true;
        for spec in specs {
            self.apply_surface_spec(spec, window, cx);
        }
        self.syncing_input = false;
        self.sync_compatibility_alias();
    }

    pub(in crate::editor) fn update_surface_projection(
        &mut self,
        update: SurfaceProjectionUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = mode_for(self.history.document(), update.key);
        let spec = SurfaceSpec {
            key: update.key,
            projection: update.projection,
            mode,
            selection: update.selection,
        };
        self.syncing_input = true;
        self.apply_surface_spec(spec, window, cx);
        self.syncing_input = false;
        self.sync_compatibility_alias();
    }

    pub(in crate::editor) fn sync_compatibility_alias(&mut self) {
        let Some((input, active_inline, source_range)) = self
            .surfaces
            .get(&self.active_surface_key())
            .map(|surface| {
                (
                    surface.input.clone(),
                    surface.projection.active_inline,
                    surface.projection.source_range.clone(),
                )
            })
        else {
            return;
        };
        self.input = input;
        self.projection =
            MarkdownProjection::build_range(self.history.document(), active_inline, source_range);
    }

    fn desired_surface_specs(&self) -> Vec<SurfaceSpec> {
        let active_key = self.active_surface;
        let active_inline = active_key
            .and_then(|key| self.surfaces.get(&key))
            .and_then(|surface| surface.projection.active_inline);
        let document = self.history.document();
        surface_specs(document, self.empty_surface_range.clone())
            .into_iter()
            .map(|(key, range)| SurfaceSpec {
                key,
                projection: MarkdownProjection::build_surface_range(
                    document,
                    (active_key == Some(key)).then_some(active_inline).flatten(),
                    range,
                ),
                mode: mode_for(document, key),
                selection: None,
            })
            .collect()
    }

    fn apply_surface_spec(
        &mut self,
        spec: SurfaceSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.surfaces.contains_key(&spec.key) {
            self.insert_surface(spec, window, cx);
            return;
        }
        let surface = &self.surfaces[&spec.key];
        let input = surface.input.clone();
        let mode_changed = surface.mode != spec.mode;
        if mode_changed {
            apply_surface_mode(&input, &spec.mode, window, cx);
        }
        let current = input.read(cx).value().to_owned();
        let patch = minimal_text_patch(&current, &spec.projection.text);
        let highlights = projection_highlights(&spec.projection, &self.theme);
        input.update(cx, |input, cx| {
            if let Some((range, replacement)) = patch {
                input.replace_text_range(range, &replacement, window, cx);
            }
            input.set_text_highlights(highlights, cx);
            apply_source_selection(input, &spec.projection, spec.selection, window, cx);
        });
        if let Some(surface) = self.surfaces.get_mut(&spec.key) {
            surface.projection = spec.projection;
            surface.mode = spec.mode;
        }
    }

    fn insert_surface(&mut self, spec: SurfaceSpec, window: &mut Window, cx: &mut Context<Self>) {
        let input = create_input(&spec.projection.text, window, cx);
        apply_surface_mode(&input, &spec.mode, window, cx);
        apply_projection_styles(&input, &spec.projection, &self.theme, cx);
        input.update(cx, |input, cx| {
            apply_source_selection(input, &spec.projection, spec.selection, window, cx);
        });
        let subscriptions = subscribe_to_input(&input, spec.key, window, cx);
        self.surfaces.insert(
            spec.key,
            MarkdownEditSurface {
                input,
                projection: spec.projection,
                mode: spec.mode,
                _subscriptions: subscriptions,
            },
        );
    }
}

fn apply_source_selection(
    input: &mut gpui_component::input::InputState,
    projection: &MarkdownProjection,
    selection: Option<SourceSelection>,
    window: &mut Window,
    cx: &mut gpui::Context<gpui_component::input::InputState>,
) {
    let Some(selection) = selection else {
        return;
    };
    let anchor = projection.source_to_display(selection.anchor);
    let head = projection.source_to_display(selection.head);
    let range = anchor.min(head)..anchor.max(head);
    if input.selected_range() != range {
        input.set_selected_range(range, anchor > head, window, cx);
    }
}
