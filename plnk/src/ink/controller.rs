//! Active-stroke controller: the live stroke between `begin` and `end`.
//!
//! The controller owns the [`InkDocument`] and one active [`Stroke`]. The
//! active stroke is deliberately *not* part of the document: only `end`
//! commits it as an [`Edit::AddStroke`] command (one undo step), while
//! `cancel` drops it without touching the document, the undo/redo history or
//! the static cache. That is the rule the "cancel must not commit a partial
//! stroke" acceptance test enforces, whether the cancellation comes from lost
//! pointer capture, the pointer leaving the window, or an Esc press.

use crate::ink::dirty::DirtyRegion;
use crate::ink::model::{Edit, InkDocument, PenPoint, PenStyle, Stroke};

/// Drives one document through live pointer contacts.
#[derive(Debug)]
pub struct InkController {
    document: InkDocument,
    active: Option<Stroke>,
    style: PenStyle,
}

impl Default for InkController {
    fn default() -> Self {
        Self::new()
    }
}

impl InkController {
    pub fn new() -> Self {
        Self::with_document(InkDocument::new())
    }

    pub fn with_document(document: InkDocument) -> Self {
        Self {
            document,
            active: None,
            style: PenStyle::fixed(crate::gfx::Rgba8::new(0, 0, 0, 255), 80.0),
        }
    }

    pub fn document(&self) -> &InkDocument {
        &self.document
    }

    pub fn document_mut(&mut self) -> &mut InkDocument {
        &mut self.document
    }

    pub fn style(&self) -> &PenStyle {
        &self.style
    }

    /// Sets the style applied to the next `begin`. An active stroke keeps the
    /// style it started with.
    pub fn set_style(&mut self, style: PenStyle) {
        self.style = PenStyle::new(style.color, style.width, style.pressure_enabled);
    }

    pub fn active(&self) -> Option<&Stroke> {
        self.active.as_ref()
    }

    pub fn is_drawing(&self) -> bool {
        self.active.is_some()
    }

    /// Starts a stroke at `point`. A previous unfinished contact is cancelled
    /// (never committed): one pointer id draws one stroke at a time.
    pub fn begin(&mut self, point: PenPoint) -> DirtyRegion {
        let mut region = self.cancel();
        let mut stroke = Stroke::new(self.style);
        region.push(stroke.push(point));
        self.active = Some(stroke);
        region
    }

    /// Appends coalesced samples to the active stroke and returns the union of
    /// the local segment rects they repainted. Without an active stroke the
    /// points are ignored (they belong to no contact).
    pub fn append(&mut self, points: &[PenPoint]) -> DirtyRegion {
        let mut region = DirtyRegion::new();
        let Some(stroke) = self.active.as_mut() else {
            return region;
        };
        for point in points {
            region.push(stroke.push(*point));
        }
        region
    }

    /// Commits the active stroke to the document as one undoable edit.
    /// Returns the pixels the static layer has to repaint, or `None` when no
    /// stroke was active or every sample was rejected.
    pub fn end(&mut self) -> Option<DirtyRegion> {
        let stroke = self.active.take()?;
        if stroke.is_empty() {
            return None;
        }
        let region = self.document.apply(Edit::AddStroke(stroke));
        (!region.is_empty()).then_some(region)
    }

    /// Discards the active stroke. Returns the pixels the active layer has to
    /// clear; the document and its history are unchanged.
    pub fn cancel(&mut self) -> DirtyRegion {
        match self.active.take() {
            Some(stroke) if !stroke.is_empty() => DirtyRegion::from_rect(stroke.bounds()),
            _ => DirtyRegion::new(),
        }
    }

    /// Loads a document (tests and session restore); any active stroke is
    /// discarded without committing.
    pub fn replace_document(&mut self, document: InkDocument) {
        self.active = None;
        self.document = document;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ink::geometry::PhysicalPointF;
    use std::time::Duration;

    fn point(x: f32, y: f32) -> PenPoint {
        PenPoint::new(PhysicalPointF::new(x, y), 1.0, Duration::ZERO)
    }

    #[test]
    fn rejected_begin_does_not_commit_an_empty_stroke() {
        let mut controller = InkController::new();
        let dirty = controller.begin(PenPoint::new(
            PhysicalPointF::new(f32::NAN, 0.0),
            1.0,
            Duration::ZERO,
        ));
        assert!(dirty.is_empty());
        assert!(controller.active().is_some());
        assert!(controller.end().is_none());
        assert!(controller.document().is_empty());
        assert_eq!(controller.document().undo_depth(), 0);
    }

    #[test]
    fn style_is_sanitized_and_applies_to_the_next_stroke() {
        let mut controller = InkController::new();
        controller.set_style(PenStyle::new(
            crate::gfx::Rgba8::new(1, 2, 3, 4),
            f32::NAN,
            true,
        ));
        controller.begin(point(10.0, 10.0));
        controller.end();
        let stroke = controller.document().strokes().last().expect("stroke");
        assert!(stroke.style().width.is_finite());
        assert_eq!(stroke.style().color, crate::gfx::Rgba8::new(1, 2, 3, 4));
    }
}
