//! Headless tests for the ink model, smoothing geometry, edit history and
//! dirty regions (Task 6).
//!
//! Legacy references:
//! - undo/redo: `src/Root.cs` (`UndoStrokes[8]`, `UndoDepth <= 7`,
//!   `SaveUndoStrokes` clears `RedoDepth`) and `docs/acceptance/legacy-baseline.md`
//!   §5.4. Erase is an explicit edit command, not the legacy folded behavior
//!   (D-004).
//! - stroke erase: `src/FormCollection.cs` sets
//!   `IC.EraserMode = InkOverlayEraserMode.StrokeErase`, which deletes whole
//!   strokes when the eraser touches their rendered geometry.
//! - highlighter: `Transparency >= 100` (equivalently alpha <= 155) selects the
//!   highlighter icon (`src/FormCollection.cs:383`).

use std::mem::size_of;
use std::time::{Duration, Instant};

use plnk::gfx::Rgba8;
use plnk::ink::{Edit, InkDocument, PenPoint, PenStyle, PhysicalPointF, Stroke, StrokeSample};

/// Deterministic LCG so the stress test never depends on the platform RNG.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
}

fn point_at(x: f32, y: f32, millis: u64) -> PenPoint {
    PenPoint::new(
        PhysicalPointF::new(x, y),
        0.5,
        Duration::from_millis(millis),
    )
}

fn opaque_style(width: f32) -> PenStyle {
    PenStyle::new(Rgba8::new(225, 60, 60, 255), width, false)
}

fn highlighter_style() -> PenStyle {
    PenStyle::new(Rgba8::new(235, 125, 15, 80), 500.0, false)
}

/// Builds a straight stroke sampled `steps + 1` times.
fn line_stroke(style: PenStyle, from: (f32, f32), to: (f32, f32), steps: usize) -> Stroke {
    let mut stroke = Stroke::new(style);
    for index in 0..=steps {
        let t = index as f32 / steps.max(1) as f32;
        let x = from.0 + (to.0 - from.0) * t;
        let y = from.1 + (to.1 - from.1) * t;
        stroke.push(point_at(x, y, index as u64));
    }
    stroke
}

fn assert_finite_sample(sample: &StrokeSample) {
    assert!(
        sample.position.x.is_finite() && sample.position.y.is_finite(),
        "non-finite smoothed position: {sample:?}"
    );
    assert!(
        sample.width.is_finite() && sample.width > 0.0,
        "non-finite or non-positive smoothed width: {sample:?}"
    );
}

#[test]
fn pressure_is_clamped_and_pressureless_devices_use_fixed_width() {
    let pressures = [-2.0f32, 0.0, 0.25, 0.5, 1.0, 2.0, f32::NAN, f32::INFINITY];

    let mut pressure_stroke =
        Stroke::new(PenStyle::new(Rgba8::new(30, 110, 200, 255), 100.0, true));
    for (index, pressure) in pressures.iter().enumerate() {
        pressure_stroke.push(PenPoint::new(
            PhysicalPointF::new(10.0 + index as f32, 20.0),
            *pressure,
            Duration::from_millis(index as u64),
        ));
    }

    for (index, _) in pressures.iter().enumerate() {
        let stored = pressure_stroke.points()[index].pressure;
        assert!(
            stored.is_finite() && (0.0..=1.0).contains(&stored),
            "pressure {index} was not clamped: {stored}"
        );
    }

    let width_zero = pressure_stroke.width_at(1).unwrap();
    let width_half = pressure_stroke.width_at(3).unwrap();
    let width_one = pressure_stroke.width_at(4).unwrap();
    assert!(width_zero < width_half && width_half < width_one);
    assert!((width_zero - 25.0).abs() < 1e-4, "pressure 0 -> 25%");
    assert!((width_one - 100.0).abs() < 1e-4, "pressure 1 -> 100%");
    // Out-of-range and non-finite input clamp to the same bound as the input
    // they are clamped/fallen back to.
    assert!((pressure_stroke.width_at(0).unwrap() - width_zero).abs() < 1e-4);
    assert!((pressure_stroke.width_at(5).unwrap() - width_one).abs() < 1e-4);
    assert!((pressure_stroke.width_at(6).unwrap() - width_one).abs() < 1e-4);
    assert!((pressure_stroke.width_at(7).unwrap() - width_one).abs() < 1e-4);

    let mut mouse_stroke = Stroke::new(PenStyle::new(Rgba8::new(80, 80, 80, 255), 80.0, false));
    for (index, pressure) in pressures.iter().enumerate() {
        mouse_stroke.push(PenPoint::new(
            PhysicalPointF::new(10.0 + index as f32, 40.0),
            *pressure,
            Duration::from_millis(index as u64),
        ));
    }
    for index in 0..pressures.len() {
        let width = mouse_stroke.width_at(index).unwrap();
        assert!(
            (width - 80.0).abs() < 1e-4,
            "pressureless device must use the fixed width, got {width}"
        );
    }
}

#[test]
fn smoothing_has_no_nan_for_duplicate_points() {
    let mut stroke = Stroke::new(opaque_style(8.0));
    let duplicate = PenPoint::new(PhysicalPointF::new(100.0, 100.0), f32::NAN, Duration::ZERO);
    for _ in 0..10 {
        stroke.push(duplicate);
    }
    assert_eq!(stroke.len(), 10);

    let samples = stroke.smoothed_samples();
    assert!(!samples.is_empty(), "a dot still renders one sample");
    for sample in &samples {
        assert_finite_sample(sample);
        assert_clamped_bounds(sample.position);
    }

    let bounds = stroke.bounds();
    assert!(!bounds.is_empty(), "a dot must produce a dirty/bounds rect");
    assert!(bounds.width() > 0 && bounds.height() > 0);

    // Duplicate points interleaved with real movement must stay finite too.
    let mut mixed = Stroke::new(opaque_style(30.0));
    for index in 0..32 {
        let x = 100.0 + (index / 2) as f32 * 3.0;
        mixed.push(point_at(x, 100.0, index as u64));
        if index % 4 == 0 {
            mixed.push(point_at(x, 100.0, index as u64));
        }
    }
    for sample in mixed.smoothed_samples() {
        assert_finite_sample(&sample);
    }
    assert!(mixed.bounds().width() > 0);

    // Non-finite positions are rejected rather than poisoning the geometry.
    let mut poisoned = Stroke::new(opaque_style(10.0));
    let rejected = poisoned.push(PenPoint::new(
        PhysicalPointF::new(f32::NAN, 5.0),
        0.5,
        Duration::ZERO,
    ));
    assert!(rejected.is_empty());
    assert!(poisoned.is_empty());
}

fn assert_clamped_bounds(position: PhysicalPointF) {
    assert!(position.x.abs() <= 1_000_000.0);
    assert!(position.y.abs() <= 1_000_000.0);
}

#[test]
fn stroke_erase_uses_stroke_bounds_and_geometry() {
    let mut document = InkDocument::new();
    let diagonal = line_stroke(opaque_style(10.0), (100.0, 100.0), (400.0, 400.0), 20);
    let diagonal_bounds = diagonal.bounds();
    document.apply(Edit::AddStroke(diagonal));

    // Inside the stroke's bounding box but far from its geometric path: the
    // broad-phase bounds hit must not erase the stroke.
    let miss = document.erase_at(PhysicalPointF::new(350.0, 150.0), 5.0);
    assert!(miss.is_empty(), "bounds-only hit must not erase a stroke");
    assert_eq!(document.len(), 1);

    let hit = document.erase_at(PhysicalPointF::new(250.0, 255.0), 5.0);
    assert!(!hit.is_empty());
    assert_eq!(document.len(), 0, "stroke erase removes the whole stroke");
    assert!(
        hit.contains_rect(diagonal_bounds),
        "the dirty region of an erase must cover the old geometry"
    );

    // A wide highlighter is erased when the eraser touches its rendered width,
    // even when it is off the center line.
    let wide = line_stroke(highlighter_style(), (500.0, 500.0), (700.0, 500.0), 20);
    document.apply(Edit::AddStroke(wide));
    let touch_edge = document.erase_at(PhysicalPointF::new(600.0, 515.0), 5.0);
    assert_eq!(document.len(), 0);
    assert!(!touch_edge.is_empty());

    // A nearby but unrelated stroke is retained.
    let left = line_stroke(opaque_style(10.0), (0.0, 0.0), (50.0, 0.0), 10);
    let right = line_stroke(opaque_style(10.0), (500.0, 0.0), (550.0, 0.0), 10);
    document.apply(Edit::AddStroke(left));
    document.apply(Edit::AddStroke(right));
    let erased = document.erase_at(PhysicalPointF::new(25.0, 0.0), 5.0);
    assert!(!erased.is_empty());
    assert_eq!(document.len(), 1);
}
#[test]
fn undo_redo_preserves_order_and_depth() {
    let mut document = InkDocument::new();
    for (index, x) in [10.0f32, 20.0, 30.0].iter().enumerate() {
        let mut stroke = Stroke::new(opaque_style(4.0));
        stroke.push(point_at(*x, 10.0, index as u64));
        document.apply(Edit::AddStroke(stroke));
    }
    let xs = |document: &InkDocument| -> Vec<f32> {
        document
            .strokes()
            .map(|stroke| stroke.points()[0].position.x)
            .collect()
    };
    assert_eq!(xs(&document), vec![10.0, 20.0, 30.0]);

    assert!(document.undo().is_some());
    assert_eq!(xs(&document), vec![10.0, 20.0]);
    assert!(document.undo().is_some());
    assert_eq!(xs(&document), vec![10.0]);
    assert_eq!(document.undo_depth(), 1);
    assert_eq!(document.redo_depth(), 2);

    assert!(document.redo().is_some());
    assert_eq!(xs(&document), vec![10.0, 20.0]);
    assert!(document.redo().is_some());
    assert_eq!(xs(&document), vec![10.0, 20.0, 30.0]);
    assert!(document.redo().is_none());
    assert_eq!(document.undo_depth(), 3);
    assert_eq!(document.redo_depth(), 0);
}

#[test]
fn dirty_region_contains_old_and_new_geometry() {
    let mut document = InkDocument::new();
    let a = line_stroke(opaque_style(6.0), (10.0, 10.0), (40.0, 10.0), 8);
    let b = line_stroke(opaque_style(6.0), (300.0, 300.0), (340.0, 340.0), 8);
    let a_bounds = a.bounds();
    let b_bounds = b.bounds();

    let added_b = document.apply(Edit::AddStroke(b));
    assert!(
        !added_b.bounds().intersects(a_bounds),
        "adding b must not dirty unrelated old geometry"
    );

    let added_a = document.apply(Edit::AddStroke(a));
    assert!(added_a.contains_rect(a_bounds));
    assert!(
        !added_a.bounds().intersects(b_bounds),
        "adding a must not dirty the existing b"
    );

    // Erasing reports the removed (old) geometry.
    let erased = document.erase_at(PhysicalPointF::new(25.0, 10.0), 3.0);
    assert!(erased.contains_rect(a_bounds));
    assert!(!erased.bounds().intersects(b_bounds));

    // Undo restores the geometry and reports it as the new geometry to redraw.
    let undo = document.undo().expect("erase is undoable");
    assert!(undo.contains_rect(a_bounds));
    assert_eq!(document.len(), 2);

    // Clear reports all old geometry and undo reports all restored geometry.
    let cleared = document.apply(Edit::Clear);
    assert!(cleared.contains_rect(a_bounds));
    assert!(cleared.contains_rect(b_bounds));
    assert!(document.is_empty());

    let undo_clear = document.undo().expect("clear is undoable");
    assert!(undo_clear.contains_rect(a_bounds));
    assert!(undo_clear.contains_rect(b_bounds));
    assert_eq!(document.len(), 2);
}

#[test]
fn redo_is_invalidated_by_a_new_edit() {
    let mut document = InkDocument::new();
    document.apply(Edit::AddStroke(line_stroke(
        opaque_style(4.0),
        (0.0, 0.0),
        (10.0, 0.0),
        4,
    )));
    document.apply(Edit::AddStroke(line_stroke(
        opaque_style(4.0),
        (20.0, 0.0),
        (30.0, 0.0),
        4,
    )));
    assert!(document.undo().is_some());
    assert_eq!(document.redo_depth(), 1);

    document.apply(Edit::AddStroke(line_stroke(
        opaque_style(4.0),
        (40.0, 0.0),
        (50.0, 0.0),
        4,
    )));
    assert_eq!(document.redo_depth(), 0);
    assert!(document.redo().is_none());
    let xs: Vec<f32> = document
        .strokes()
        .map(|stroke| stroke.points()[0].position.x)
        .collect();
    assert_eq!(xs, vec![0.0, 40.0]);
}

#[test]
fn history_keeps_seven_undo_states() {
    let mut document = InkDocument::new();
    for index in 0..8 {
        let mut stroke = Stroke::new(opaque_style(3.0));
        stroke.push(point_at(index as f32 * 10.0, 0.0, index));
        document.apply(Edit::AddStroke(stroke));
    }
    assert_eq!(document.len(), 8);
    assert_eq!(document.undo_depth(), 7);

    for _ in 0..7 {
        assert!(document.undo().is_some());
    }
    assert!(document.undo().is_none(), "only 7 undo states are kept");
    assert_eq!(document.len(), 1, "the oldest state is no longer reachable");
    assert_eq!(
        document.strokes().next().unwrap().points()[0].position.x,
        0.0
    );

    for _ in 0..7 {
        assert!(document.redo().is_some());
    }
    assert!(document.redo().is_none());
    assert_eq!(document.len(), 8);
}

#[test]
fn clear_is_undoable_and_clears_redo() {
    let mut document = InkDocument::new();
    document.apply(Edit::AddStroke(line_stroke(
        opaque_style(4.0),
        (0.0, 0.0),
        (20.0, 0.0),
        4,
    )));
    document.apply(Edit::AddStroke(line_stroke(
        opaque_style(4.0),
        (40.0, 0.0),
        (60.0, 0.0),
        4,
    )));
    assert!(document.undo().is_some());
    assert_eq!(document.redo_depth(), 1);

    document.apply(Edit::Clear);
    assert!(document.is_empty());
    assert_eq!(document.redo_depth(), 0, "clear clears redo");

    let undo = document.undo().expect("clear is undoable");
    assert!(!undo.is_empty());
    assert_eq!(document.len(), 1);
    let redo = document.redo().expect("clear is redoable");
    assert!(!redo.is_empty());
    assert!(document.is_empty());

    // P-056 / legacy btClear_Click: clear invalidates redo even when the
    // canvas is already empty.
    assert!(document.undo().is_some());
    assert!(document.undo().is_some());
    assert!(document.is_empty());
    assert_eq!(document.redo_depth(), 2);
    let empty_clear = document.apply(Edit::Clear);
    assert!(empty_clear.is_empty());
    assert_eq!(
        document.redo_depth(),
        0,
        "empty clear still invalidates redo"
    );
    assert!(document.redo().is_none());
}

#[test]
fn highlighter_overlap_keeps_both_translucent_strokes() {
    let style = highlighter_style();
    assert!(style.is_highlighter());
    assert!(!opaque_style(500.0).is_highlighter());
    assert!(
        PenStyle::new(Rgba8::new(0, 0, 0, 155), 500.0, false).is_highlighter(),
        "legacy Transparency >= 100 boundary"
    );
    assert!(!PenStyle::new(Rgba8::new(0, 0, 0, 156), 500.0, false).is_highlighter());

    let mut document = InkDocument::new();
    let horizontal = line_stroke(style, (100.0, 100.0), (300.0, 100.0), 10);
    let vertical = line_stroke(style, (200.0, 50.0), (200.0, 150.0), 10);
    let horizontal_bounds = horizontal.bounds();
    let vertical_bounds = vertical.bounds();
    let first = document.apply(Edit::AddStroke(horizontal));
    let second = document.apply(Edit::AddStroke(vertical));
    assert!(
        first.bounds().intersects(vertical_bounds),
        "the two highlighter strokes overlap"
    );
    assert!(second.bounds().intersects(horizontal_bounds));
    assert_eq!(document.len(), 2, "overlap does not merge strokes");

    let styles: Vec<PenStyle> = document.strokes().map(Stroke::style).copied().collect();
    assert_eq!(styles[0].color.a, 80, "alpha is per stroke, not baked");
    assert_eq!(styles[1].color.a, 80);
    assert!(styles.iter().all(PenStyle::is_highlighter));

    // Erasing one translucent stroke leaves the other untouched.
    document.undo();
    assert_eq!(document.len(), 1);
    assert!(document.strokes().next().unwrap().style().is_highlighter());
    document.redo();
    assert_eq!(document.len(), 2);
}

#[test]
fn close_and_reopen_preserves_history_for_undo() {
    let mut document = InkDocument::new();
    let a = line_stroke(opaque_style(6.0), (10.0, 10.0), (60.0, 10.0), 8);
    let b = line_stroke(opaque_style(6.0), (100.0, 100.0), (160.0, 100.0), 8);
    let a_bounds = a.bounds();
    let b_bounds = b.bounds();
    document.apply(Edit::AddStroke(a));
    document.apply(Edit::AddStroke(b));

    let closed = document.close_annotation();
    assert!(!closed.is_empty());
    assert!(document.is_empty(), "closing clears the annotation canvas");
    assert_eq!(document.undo_depth(), 3, "close behaves like an edit");

    // Reopening is same-process only; history and the undo command survive.
    document.reopen_annotation();
    assert!(document.is_empty());
    let undo = document
        .undo()
        .expect("undo restores the pre-close strokes");
    assert!(undo.contains_rect(a_bounds));
    assert!(undo.contains_rect(b_bounds));
    assert_eq!(document.len(), 2);
    let xs: Vec<f32> = document
        .strokes()
        .map(|stroke| stroke.points()[0].position.x)
        .collect();
    assert_eq!(xs, vec![10.0, 100.0]);
}

#[test]
fn closing_an_empty_annotation_still_discards_redo() {
    let mut document = InkDocument::new();
    document.apply(Edit::AddStroke(line_stroke(
        opaque_style(4.0),
        (0.0, 0.0),
        (10.0, 0.0),
        4,
    )));
    document.undo();
    assert_eq!(document.redo_depth(), 1);
    assert!(document.close_annotation().is_empty());
    document.reopen_annotation();
    assert_eq!(document.redo_depth(), 0);
    assert!(document.redo().is_none());
}

#[test]
fn erase_undo_restores_original_stroke_order() {
    let mut document = InkDocument::new();
    for x in [10.0f32, 20.0, 30.0] {
        let mut stroke = Stroke::new(opaque_style(4.0));
        stroke.push(point_at(x, 5.0, 0));
        document.apply(Edit::AddStroke(stroke));
    }
    let mid = document.erase_at(PhysicalPointF::new(20.0, 5.0), 3.0);
    assert!(!mid.is_empty());
    let xs_after_erase: Vec<f32> = document
        .strokes()
        .map(|stroke| stroke.points()[0].position.x)
        .collect();
    assert_eq!(xs_after_erase, vec![10.0, 30.0], "one stroke is erased");

    document.undo();
    let xs_after_undo: Vec<f32> = document
        .strokes()
        .map(|stroke| stroke.points()[0].position.x)
        .collect();
    assert_eq!(
        xs_after_undo,
        vec![10.0, 20.0, 30.0],
        "undo re-inserts at the original z-order index"
    );
}

#[test]
fn timestamps_are_monotonic() {
    let mut stroke = Stroke::new(opaque_style(4.0));
    stroke.push(point_at(0.0, 0.0, 100));
    stroke.push(point_at(1.0, 0.0, 40));
    stroke.push(point_at(2.0, 0.0, 100));
    stroke.push(point_at(3.0, 0.0, 120));
    let timestamps: Vec<Duration> = stroke.points().iter().map(|p| p.timestamp).collect();
    assert_eq!(
        timestamps,
        vec![
            Duration::from_millis(100),
            Duration::from_millis(100),
            Duration::from_millis(100),
            Duration::from_millis(120),
        ]
    );
}

#[test]
fn thousand_mixed_strokes_have_finite_geometry_and_bounded_dirty_regions() {
    const STROKES: usize = 1000;
    const POINTS: usize = 1000;
    const MAX_STEP: f32 = 4.0;

    let started = Instant::now();
    let mut rng = Lcg::new(0x5eed_2026);
    let mut document = InkDocument::new();
    let mut x = 500.0f32;
    let mut y = 500.0f32;
    let mut max_push_dirty_area = 0u64;
    let mut max_push_dirty_side = 0i32;
    let mut max_region_rects = 0usize;
    let mut total_samples = 0usize;
    let mut skipped_non_finite = 0usize;

    for stroke_index in 0..STROKES {
        let style = match stroke_index % 6 {
            0 => PenStyle::new(Rgba8::new(225, 60, 60, 255), 80.0, false),
            1 => PenStyle::new(Rgba8::new(30, 110, 200, 255), 30.0, true),
            2 => PenStyle::new(Rgba8::new(235, 125, 15, 80), 500.0, false),
            3 => PenStyle::new(Rgba8::new(145, 70, 160, 80), 500.0, true),
            4 => PenStyle::new(Rgba8::new(80, 80, 80, 255), 30.0, false),
            _ => PenStyle::new(Rgba8::new(120, 175, 70, 255), 80.0, true),
        };
        let mut stroke = Stroke::new(style);

        for point_index in 0..POINTS {
            let value = rng.next_u32();
            // Every 7th point duplicates the previous position to exercise the
            // zero-distance path; the rest walk by at most MAX_STEP.
            if point_index % 7 != 0 {
                let dx = (value % 9) as f32 - MAX_STEP;
                let dy = ((value / 9) % 9) as f32 - MAX_STEP;
                x = (x + dx).clamp(0.0, 4000.0);
                y = (y + dy).clamp(0.0, 4000.0);
            }
            let pressure = match value % 11 {
                0 => f32::NAN,
                1 => 2.0,
                _ => ((value >> 8) & 1023) as f32 / 1023.0,
            };
            let position = if point_index % 97 == 0 && stroke_index % 5 == 0 {
                PhysicalPointF::new(f32::NAN, y)
            } else {
                PhysicalPointF::new(x, y)
            };
            let dirty = stroke.push(PenPoint::new(
                position,
                pressure,
                Duration::from_micros((point_index * 1000 + stroke_index * 10) as u64),
            ));

            if !position.is_finite() {
                skipped_non_finite += 1;
                assert!(dirty.is_empty(), "invalid input must not dirty pixels");
                continue;
            }
            assert!(!dirty.is_empty(), "a valid point must dirty its new pixels");
            let tail = &stroke.points()[stroke.len().saturating_sub(3)..];
            let (mut min_x, mut max_x) = (tail[0].position.x, tail[0].position.x);
            let (mut min_y, mut max_y) = (tail[0].position.y, tail[0].position.y);
            for tail_point in tail {
                min_x = min_x.min(tail_point.position.x);
                max_x = max_x.max(tail_point.position.x);
                min_y = min_y.min(tail_point.position.y);
                max_y = max_y.max(tail_point.position.y);
            }
            let width_bound = style.width as i32 + (max_x - min_x).ceil() as i32 + 4;
            let height_bound = style.width as i32 + (max_y - min_y).ceil() as i32 + 4;
            assert!(
                dirty.width() <= width_bound && dirty.height() <= height_bound,
                "per-push dirty rect grew beyond its three-point tail: {dirty:?} \
                 bounds {width_bound}x{height_bound} style {style:?} tail {tail:?}"
            );
            max_push_dirty_area = max_push_dirty_area.max(dirty.area());
            max_push_dirty_side = max_push_dirty_side.max(dirty.width().max(dirty.height()));
        }

        let bounds = stroke.bounds();
        assert!(!bounds.is_empty());

        let samples = stroke.smoothed_samples();
        assert!(!samples.is_empty());
        for sample in samples {
            assert_finite_sample(&sample);
            assert_clamped_bounds(sample.position);
            total_samples += 1;
        }

        let region = document.apply(Edit::AddStroke(stroke));
        assert!(!region.is_empty());
        assert_eq!(
            region.rects().len(),
            1,
            "adding one stroke coalesces into one dirty rect"
        );
        max_region_rects = max_region_rects.max(region.rects().len());
        assert!(region.bounds().width() <= 4600);
        assert!(region.bounds().height() <= 4600);
    }

    let elapsed = started.elapsed();
    assert_eq!(document.len(), STROKES);
    assert_eq!(document.undo_depth(), 7);
    assert_eq!(document.redo_depth(), 0);
    assert!(document.bounds().width() > 0);
    assert_eq!(skipped_non_finite, STROKES / 5 * POINTS.div_ceil(97));
    assert!(total_samples >= STROKES * POINTS / 2);

    let raw_point_bytes = STROKES * POINTS * size_of::<PenPoint>();
    let history_copy_bytes = 7 * POINTS * size_of::<PenPoint>();
    println!(
        "ink stress: {} strokes x {} points in {elapsed:?}; \
         raw points ~{:.1} MiB, 7 undo copies ~{:.1} KiB, \
         smoothed samples {total_samples}, max per-push dirty {max_push_dirty_side}px \
         ({max_push_dirty_area}px^2), max region rects {max_region_rects}, \
         skipped non-finite {skipped_non_finite}",
        STROKES,
        POINTS,
        raw_point_bytes as f64 / (1024.0 * 1024.0),
        history_copy_bytes as f64 / 1024.0,
    );
}
