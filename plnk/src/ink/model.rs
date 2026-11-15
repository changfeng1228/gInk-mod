//! Ink document: strokes, tool style and command-based editing.
//!
//! Coordinates are physical screen pixels. A [`Stroke`] stores its raw pointer
//! samples plus the tool style; smoothing geometry is derived
//! ([`crate::ink::geometry`]) and the cached bounds/dirty rects cover the
//! rendered path. [`InkDocument`] applies [`Edit`] commands and keeps a bounded
//! undo/redo history of invertible command records, never whole-document
//! copies per frame.

use std::time::Duration;

use crate::gfx::Rgba8;
use crate::ink::dirty::{DirtyRect, DirtyRegion};
use crate::ink::geometry::{self, PhysicalPointF, StrokeSample};
use crate::ink::history::{AddedStroke, History, HistoryEntry, RemovedStroke};

/// Largest accepted stroke width in physical pixels (legacy pen range 30–3000).
pub const MAX_STROKE_WIDTH: f32 = 3000.0;
/// Smallest accepted stroke width; keeps every sample visible.
pub const MIN_STROKE_WIDTH: f32 = 1.0;
/// Width used when a configured width is non-finite.
pub const DEFAULT_STROKE_WIDTH: f32 = 80.0;
/// Width ratio at pressure 0 for pressure-capable devices.
pub const MIN_PRESSURE_WIDTH_RATIO: f32 = 0.25;
/// Legacy highlighter threshold: alpha `<= 155` means `Transparency >= 100`.
pub const HIGHLIGHTER_MAX_ALPHA: u8 = 155;
/// Upper bound for an eraser radius; protects narrow-phase arithmetic.
pub const MAX_ERASER_RADIUS: f32 = 2000.0;

/// One raw pointer sample. Pressure is stored clamped to `[0, 1]`; timestamps
/// are forced monotonically non-decreasing by [`Stroke::push`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenPoint {
    pub position: PhysicalPointF,
    pub pressure: f32,
    pub timestamp: Duration,
}

impl PenPoint {
    pub const fn new(position: PhysicalPointF, pressure: f32, timestamp: Duration) -> Self {
        Self {
            position,
            pressure,
            timestamp,
        }
    }
}

/// Tool style for one stroke. Width is in physical pixels; `pressure_enabled`
/// distinguishes a pressure-capable pen from a mouse/touch device, which always
/// uses the fixed width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenStyle {
    pub color: Rgba8,
    pub width: f32,
    pub pressure_enabled: bool,
}

impl PenStyle {
    /// Builds a style with a sanitized width (non-finite falls back to 80 px,
    /// out-of-range clamps to 1–3000 px).
    pub fn new(color: Rgba8, width: f32, pressure_enabled: bool) -> Self {
        Self {
            color,
            width: sanitize_width(width),
            pressure_enabled,
        }
    }

    pub fn fixed(color: Rgba8, width: f32) -> Self {
        Self::new(color, width, false)
    }

    pub fn pressure(color: Rgba8, width: f32) -> Self {
        Self::new(color, width, true)
    }

    /// Builds a style from the legacy Microsoft.Ink HIMETRIC width (0.01 mm).
    ///
    /// `pen.width` and the pen-width slider stay in HIMETRIC; only rendering
    /// converts to physical pixels with `width * dpi / 2540`. Configured legacy
    /// widths are clamped to 30–3000 HIMETRIC before conversion; the resulting
    /// physical width is then sanitized by `new`.
    pub fn himetric(color: Rgba8, width: f32, dpi: f32, pressure_enabled: bool) -> Self {
        const MIN_WIDTH_HIMETRIC: f32 = 30.0;
        const MAX_WIDTH_HIMETRIC: f32 = 3000.0;
        const HIMETRIC_PER_INCH: f32 = 2540.0;
        let width = if width.is_finite() {
            width.clamp(MIN_WIDTH_HIMETRIC, MAX_WIDTH_HIMETRIC)
        } else {
            DEFAULT_STROKE_WIDTH
        };
        let dpi = if dpi.is_finite() && dpi > 0.0 {
            dpi
        } else {
            96.0
        };
        Self::new(color, width * dpi / HIMETRIC_PER_INCH, pressure_enabled)
    }

    /// Legacy `Transparency >= 100` (`Alpha <= 155`) highlighter rule.
    pub fn is_highlighter(&self) -> bool {
        self.color.a <= HIGHLIGHTER_MAX_ALPHA
    }

    /// Maximum width this style can draw, used for dirty bounds and hit tests.
    pub fn max_width(&self) -> f32 {
        sanitize_width(self.width)
    }

    /// Width for one raw sample. Pressureless devices always use the configured
    /// width. Pressure-capable devices clamp pressure to `[0, 1]` and scale
    /// between 25% and 100% of the configured width; non-finite pressure is
    /// treated as full pressure so a broken device still draws.
    pub fn width_at(&self, pressure: f32) -> f32 {
        let width = sanitize_width(self.width);
        if !self.pressure_enabled {
            return width;
        }
        let pressure = sanitize_pressure(pressure);
        width * (MIN_PRESSURE_WIDTH_RATIO + (1.0 - MIN_PRESSURE_WIDTH_RATIO) * pressure)
    }
}

fn sanitize_width(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(MIN_STROKE_WIDTH, MAX_STROKE_WIDTH)
    } else {
        DEFAULT_STROKE_WIDTH
    }
}

/// Clamps a pressure sample to `[0, 1]`; non-finite input becomes full pressure.
pub fn sanitize_pressure(pressure: f32) -> f32 {
    if pressure.is_finite() {
        pressure.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Stable stroke identity inside one [`InkDocument`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StrokeId(u64);

impl StrokeId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One completed stroke: raw samples plus tool style and cached bounds.
#[derive(Clone, Debug)]
pub struct Stroke {
    points: Vec<PenPoint>,
    style: PenStyle,
    bounds: DirtyRect,
}

impl Stroke {
    pub fn new(style: PenStyle) -> Self {
        Self {
            points: Vec::new(),
            style: PenStyle::new(style.color, style.width, style.pressure_enabled),
            bounds: DirtyRect::EMPTY,
        }
    }

    /// Appends a sample and returns the dirty rect of the newly rendered local
    /// segment only. Non-finite positions are rejected (the stroke is
    /// unchanged); pressure and timestamps are sanitized before storage.
    pub fn push(&mut self, mut point: PenPoint) -> DirtyRect {
        let Some(position) = geometry::sanitize_position(point.position) else {
            return DirtyRect::EMPTY;
        };
        point.position = position;
        point.pressure = sanitize_pressure(point.pressure);
        if let Some(last) = self.points.last()
            && point.timestamp < last.timestamp
        {
            point.timestamp = last.timestamp;
        }
        self.points.push(point);
        let dirty = geometry::push_dirty_rect(&self.points, &self.style);
        self.bounds = self.bounds.union(dirty);
        dirty
    }

    pub fn points(&self) -> &[PenPoint] {
        &self.points
    }

    pub fn style(&self) -> &PenStyle {
        &self.style
    }

    /// Bounds of all rendered geometry, padded by the maximum half width and
    /// one antialiasing pixel.
    pub fn bounds(&self) -> DirtyRect {
        self.bounds
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Effective width of sample `index`, or `None` when out of range.
    pub fn width_at(&self, index: usize) -> Option<f32> {
        self.points
            .get(index)
            .map(|point| self.style.width_at(point.pressure))
    }

    /// Smoothed polyline to render (see [`crate::ink::geometry`]).
    pub fn smoothed_samples(&self) -> Vec<StrokeSample> {
        geometry::smooth_samples(&self.points, &self.style)
    }

    pub(crate) fn intersects_circle(&self, center: PhysicalPointF, radius: f32) -> bool {
        geometry::stroke_intersects_circle(&self.points, &self.style, center, radius)
    }
}

/// A document edit. Commands are the unit of undo/redo.
#[derive(Debug)]
pub enum Edit {
    /// Appends one completed stroke at the top of the z-order.
    AddStroke(Stroke),
    /// Removes the named strokes (whole-stroke erase, Microsoft.Ink
    /// `StrokeErase` semantics).
    EraseStrokes(Vec<StrokeId>),
    /// Removes all strokes; a following undo restores the whole document.
    Clear,
}

#[derive(Debug)]
struct StrokeEntry {
    id: StrokeId,
    stroke: Stroke,
}

/// Ordered ink strokes plus bounded command history.
#[derive(Debug)]
pub struct InkDocument {
    strokes: Vec<StrokeEntry>,
    next_id: u64,
    history: History,
    closed: bool,
}

impl Default for InkDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl InkDocument {
    pub fn new() -> Self {
        Self {
            strokes: Vec::new(),
            next_id: 0,
            history: History::new(),
            closed: false,
        }
    }

    /// Applies an edit and returns exactly the pixels that changed (old
    /// geometry for removals, new geometry for additions).
    pub fn apply(&mut self, edit: Edit) -> DirtyRegion {
        match edit {
            Edit::AddStroke(stroke) => self.add_stroke(stroke),
            Edit::EraseStrokes(ids) => self.erase_ids(ids),
            Edit::Clear => self.clear(),
        }
    }

    fn add_stroke(&mut self, stroke: Stroke) -> DirtyRegion {
        if stroke.is_empty() {
            return DirtyRegion::new();
        }
        let id = self.allocate_id();
        let index = self.strokes.len();
        let region = DirtyRegion::from_rect(stroke.bounds());
        let history_stroke = stroke.clone();
        self.strokes.push(StrokeEntry { id, stroke });
        self.history.record(HistoryEntry {
            added: Some(AddedStroke {
                index,
                id,
                stroke: history_stroke,
            }),
            removed: Vec::new(),
        });
        region
    }

    fn erase_ids(&mut self, mut ids: Vec<StrokeId>) -> DirtyRegion {
        if ids.is_empty() || self.strokes.is_empty() {
            return DirtyRegion::new();
        }
        ids.sort_unstable();
        ids.dedup();
        let indices: Vec<usize> = self
            .strokes
            .iter()
            .enumerate()
            .filter(|(_, entry)| ids.binary_search(&entry.id).is_ok())
            .map(|(index, _)| index)
            .collect();
        if indices.is_empty() {
            return DirtyRegion::new();
        }

        // Remove from the back so earlier indices stay valid, then keep the
        // removed list in ascending original order for undo insertion.
        let mut removed = Vec::with_capacity(indices.len());
        for &index in indices.iter().rev() {
            let entry = self.strokes.remove(index);
            removed.push(RemovedStroke {
                index,
                id: entry.id,
                stroke: entry.stroke,
            });
        }
        removed.reverse();
        let region = region_of_removed(&removed);
        self.history.record(HistoryEntry {
            added: None,
            removed,
        });
        region
    }

    fn clear(&mut self) -> DirtyRegion {
        if self.strokes.is_empty() {
            // P-056: clear invalidates redo even when the canvas is already
            // empty. Legacy `btClear_Click` always calls `SaveUndoStrokes`,
            // which sets `RedoDepth = 0` before doing anything else.
            self.history.discard_redo();
            return DirtyRegion::new();
        }
        let removed: Vec<RemovedStroke> = self
            .strokes
            .drain(..)
            .enumerate()
            .map(|(index, entry)| RemovedStroke {
                index,
                id: entry.id,
                stroke: entry.stroke,
            })
            .collect();
        let region = region_of_removed(&removed);
        self.history.record(HistoryEntry {
            added: None,
            removed,
        });
        region
    }

    /// Undoes the last edit and returns the pixels to repaint. `None` when
    /// there is no undo state.
    pub fn undo(&mut self) -> Option<DirtyRegion> {
        let entry = self.history.undo_entry()?;
        let mut region = DirtyRegion::new();
        if let Some(added) = &entry.added
            && let Some(index) = self.strokes.iter().position(|stroke| stroke.id == added.id)
        {
            let removed = self.strokes.remove(index);
            region.push(removed.stroke.bounds());
        }
        for removed in &entry.removed {
            let index = removed.index.min(self.strokes.len());
            self.strokes.insert(
                index,
                StrokeEntry {
                    id: removed.id,
                    stroke: removed.stroke.clone(),
                },
            );
            region.push(removed.stroke.bounds());
        }
        self.history.finish_undo();
        Some(region)
    }

    /// Replays the last undone edit and returns the pixels to repaint. `None`
    /// when there is no redo state.
    pub fn redo(&mut self) -> Option<DirtyRegion> {
        let entry = self.history.redo_entry()?;
        let mut region = DirtyRegion::new();
        for removed in &entry.removed {
            if let Some(index) = self
                .strokes
                .iter()
                .position(|stroke| stroke.id == removed.id)
            {
                let removed_now = self.strokes.remove(index);
                region.push(removed_now.stroke.bounds());
            }
        }
        if let Some(added) = &entry.added {
            let index = added.index.min(self.strokes.len());
            self.strokes.insert(
                index,
                StrokeEntry {
                    id: added.id,
                    stroke: added.stroke.clone(),
                },
            );
            region.push(added.stroke.bounds());
        }
        self.history.finish_redo();
        Some(region)
    }

    /// Whole-stroke erase at a pointer position, Microsoft.Ink
    /// `InkOverlayEraserMode.StrokeErase` semantics: a stroke whose rendered
    /// geometry (path plus half its width) touches the eraser circle is removed
    /// entirely. Bounds provide the broad phase; the polyline distance test is
    /// the narrow phase.
    pub fn erase_at(&mut self, position: PhysicalPointF, radius: f32) -> DirtyRegion {
        let Some(center) = geometry::sanitize_position(position) else {
            return DirtyRegion::new();
        };
        let radius = if radius.is_finite() {
            radius.clamp(0.0, MAX_ERASER_RADIUS)
        } else {
            0.0
        };
        let search_margin = radius.ceil() as i32 + 1;
        let mut hits = Vec::new();
        for entry in &self.strokes {
            if !entry
                .stroke
                .bounds()
                .expand(search_margin)
                .contains_point(center.x, center.y)
            {
                continue;
            }
            if entry.stroke.intersects_circle(center, radius) {
                hits.push(entry.id);
            }
        }
        if hits.is_empty() {
            DirtyRegion::new()
        } else {
            self.apply(Edit::EraseStrokes(hits))
        }
    }

    /// Legacy exit path (P-057): clears the canvas through the history as a
    /// normal edit, so the next undo after reopening restores the pre-close
    /// strokes. Closing always invalidates redo, even when the canvas is
    /// already empty (legacy `RetreatAndExit` -> `SaveUndoStrokes`). Same
    /// process only; nothing is written to disk.
    pub fn close_annotation(&mut self) -> DirtyRegion {
        self.closed = true;
        if self.strokes.is_empty() {
            self.history.discard_redo();
            DirtyRegion::new()
        } else {
            self.clear()
        }
    }

    /// Reopens the annotation overlay without touching the in-memory history.
    pub fn reopen_annotation(&mut self) {
        self.closed = false;
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Strokes in z-order (oldest first).
    pub fn strokes(&self) -> impl ExactSizeIterator<Item = &Stroke> + '_ {
        self.strokes.iter().map(|entry| &entry.stroke)
    }

    /// Stroke ids aligned with [`Self::strokes`].
    pub fn stroke_ids(&self) -> impl ExactSizeIterator<Item = StrokeId> + '_ {
        self.strokes.iter().map(|entry| entry.id)
    }

    pub fn len(&self) -> usize {
        self.strokes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty()
    }

    /// Bounds of every stroke, or [`DirtyRect::EMPTY`].
    pub fn bounds(&self) -> DirtyRect {
        self.strokes
            .iter()
            .map(|entry| entry.stroke.bounds())
            .fold(DirtyRect::EMPTY, DirtyRect::union)
    }

    pub fn undo_depth(&self) -> usize {
        self.history.undo_depth()
    }

    pub fn redo_depth(&self) -> usize {
        self.history.redo_depth()
    }

    pub fn can_undo(&self) -> bool {
        self.undo_depth() > 0
    }

    pub fn can_redo(&self) -> bool {
        self.redo_depth() > 0
    }

    fn allocate_id(&mut self) -> StrokeId {
        let id = StrokeId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        id
    }
}

fn region_of_removed(removed: &[RemovedStroke]) -> DirtyRegion {
    let mut region = DirtyRegion::new();
    for stroke in removed {
        region.push(stroke.stroke.bounds());
    }
    region
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_sanitizes_width_and_pressure() {
        let style = PenStyle::new(Rgba8::new(0, 0, 0, 255), f32::NAN, false);
        assert!((style.width - DEFAULT_STROKE_WIDTH).abs() < f32::EPSILON);
        assert!((style.width_at(0.5) - DEFAULT_STROKE_WIDTH).abs() < f32::EPSILON);

        let pressure_style = PenStyle::new(Rgba8::new(0, 0, 0, 255), 100.0, true);
        assert!((pressure_style.width_at(f32::NAN) - 100.0).abs() < f32::EPSILON);
        assert!((pressure_style.width_at(0.0) - 25.0).abs() < 1e-4);

        // A struct-literal style bypasses `new`; width consumers still sanitize.
        let hostile = PenStyle {
            color: Rgba8::new(0, 0, 0, 255),
            width: f32::NAN,
            pressure_enabled: true,
        };
        assert!(hostile.width_at(0.5).is_finite());
        assert!(hostile.max_width().is_finite());
    }

    #[test]
    fn empty_stroke_and_empty_edits_are_not_recorded() {
        let mut document = InkDocument::new();
        assert!(
            document
                .apply(Edit::AddStroke(Stroke::new(PenStyle::fixed(
                    Rgba8::new(0, 0, 0, 255),
                    5.0
                ))))
                .is_empty()
        );
        assert_eq!(document.undo_depth(), 0);
        assert!(document.apply(Edit::Clear).is_empty());
        assert_eq!(document.undo_depth(), 0);
        assert!(
            document
                .erase_at(PhysicalPointF::new(0.0, 0.0), 10.0)
                .is_empty()
        );
    }
}
