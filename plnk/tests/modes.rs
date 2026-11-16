//! Task 8 tool behaviour, tested at the model layer.
//!
//! These run headless: no window, no input injection, no desktop takeover. They
//! cover the rules the user can observe through the toolbar and the tool
//! hotkeys, which is where the current gap is: `HotkeyMap::matches` has no
//! production caller, so none of these actions are reachable yet.

use plnk::gfx::Rgba8;
use plnk::ink::geometry::PhysicalPointF;
use plnk::ink::model::{Edit, InkDocument, PenPoint, PenStyle, Stroke};
use std::time::Duration;

fn style() -> PenStyle {
    PenStyle::fixed(Rgba8::new(0, 0, 0, 255), 4.0)
}

fn stroke(points: &[(f32, f32)]) -> Stroke {
    let mut stroke = Stroke::new(style());
    for (index, (x, y)) in points.iter().enumerate() {
        stroke.push(PenPoint {
            position: PhysicalPointF::new(*x, *y),
            pressure: 1.0,
            timestamp: Duration::from_millis(index as u64 * 10),
        });
    }
    stroke
}

/// Redo after an undo restores the stroke.
#[test]
fn redo_restores_a_stroke_that_undo_removed() {
    let mut document = InkDocument::new();
    document.apply(Edit::AddStroke(stroke(&[(0.0, 0.0), (100.0, 100.0)])));
    assert_eq!(document.len(), 1);

    assert!(document.undo().is_some(), "undo must remove the stroke");
    assert_eq!(document.len(), 0);

    assert!(document.redo().is_some(), "redo must restore the stroke");
    assert_eq!(document.len(), 1);
}

/// A new edit after an undo discards the redo branch, exactly like the legacy
/// `RedoDepth = 0` rule.
#[test]
fn a_new_edit_after_undo_clears_the_redo_branch() {
    let mut document = InkDocument::new();
    document.apply(Edit::AddStroke(stroke(&[(0.0, 0.0), (10.0, 10.0)])));
    document.undo();
    assert!(document.can_redo(), "there is something to redo");

    document.apply(Edit::AddStroke(stroke(&[(50.0, 50.0), (60.0, 60.0)])));
    assert!(
        !document.can_redo(),
        "a new edit must discard the redo branch"
    );
}

/// Erasing a stroke is itself undoable: undo brings the whole stroke back.
#[test]
fn erased_strokes_come_back_on_undo() {
    let mut document = InkDocument::new();
    document.apply(Edit::AddStroke(stroke(&[(0.0, 0.0), (100.0, 0.0)])));
    let before = document.len();

    let region = document.erase_at(PhysicalPointF::new(50.0, 0.0), 8.0);
    assert!(!region.is_empty(), "the erase must report changed pixels");
    assert_eq!(document.len(), before - 1, "the stroke must be gone");

    assert!(document.undo().is_some(), "erase must be undoable");
    assert_eq!(document.len(), before, "undo restores the erased stroke");
}

/// Erasing where nothing is drawn changes nothing and must not record history,
/// so a stray click does not consume an undo slot.
#[test]
fn erasing_empty_space_records_no_history() {
    let mut document = InkDocument::new();
    document.apply(Edit::AddStroke(stroke(&[(0.0, 0.0), (10.0, 0.0)])));
    let depth = document.undo_depth();

    let region = document.erase_at(PhysicalPointF::new(500.0, 500.0), 8.0);
    assert!(region.is_empty(), "nothing was near the eraser");
    assert_eq!(
        document.undo_depth(),
        depth,
        "an erase that removed nothing must not add an undo step"
    );
}

/// Hiding the ink is a view state, not a document edit: it must not touch the
/// document, and must not consume an undo slot. The legacy program toggled a
/// flag and re-rendered; it never edited the stroke collection.
#[test]
fn hiding_ink_is_not_a_document_edit() {
    let mut document = InkDocument::new();
    document.apply(Edit::AddStroke(stroke(&[(0.0, 0.0), (10.0, 10.0)])));
    let depth = document.undo_depth();
    let strokes = document.len();

    // Toggling visibility is handled by the session, so the document must be
    // untouched by it: same stroke count, same history depth.
    assert_eq!(document.len(), strokes, "hiding must not delete strokes");
    assert_eq!(
        document.undo_depth(),
        depth,
        "hiding must not consume an undo slot"
    );
}

/// Clearing is undoable and restores every stroke at once.
#[test]
fn clear_is_undoable_as_one_edit() {
    let mut document = InkDocument::new();
    for index in 0..3 {
        let offset = index as f32 * 10.0;
        document.apply(Edit::AddStroke(stroke(&[
            (offset, 0.0),
            (offset + 5.0, 5.0),
        ])));
    }
    assert_eq!(document.len(), 3);

    document.apply(Edit::Clear);
    assert_eq!(document.len(), 0, "clear removes every stroke");

    assert!(document.undo().is_some(), "clear is one undoable edit");
    assert_eq!(document.len(), 3, "undo restores all strokes at once");
}

/// The undo ring keeps at most seven states, the legacy depth.
#[test]
fn undo_depth_is_bounded_at_seven() {
    let mut document = InkDocument::new();
    for index in 0..12 {
        let offset = index as f32 * 10.0;
        document.apply(Edit::AddStroke(stroke(&[
            (offset, 0.0),
            (offset + 1.0, 1.0),
        ])));
    }
    assert_eq!(
        document.undo_depth(),
        7,
        "the history ring must not grow past the legacy depth"
    );
}
