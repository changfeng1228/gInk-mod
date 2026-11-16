//! Stroke geometry: physical-pixel points, width from pressure, smoothing and
//! geometric hit testing.
//!
//! # Smoothing
//!
//! Raw pointer samples are noisy, so the rendered path is a quadratic B-spline
//! built through segment midpoints: the curve goes from `mid(p[i-1], p[i])` to
//! `mid(p[i], p[i+1])` with `p[i]` as control point, and straight caps connect
//! the first/last raw point to the first/last midpoint. Every operation here is
//! a midpoint, lerp or quadratic polynomial, so duplicate and zero-distance
//! points produce repeated samples but never a division by zero or NaN.
//!
//! Sampling is capped per segment ([`MAX_SAMPLES_PER_SEGMENT`]) to keep a
//! pathological 1,000-point stroke bounded in time and memory.

use crate::ink::dirty::DirtyRect;
use crate::ink::model::{PenPoint, PenStyle};

/// Coordinates beyond this magnitude are clamped. The design assumes every
/// realistic virtual-screen coordinate is at most 1,000,000 physical pixels
/// (see `platform::dpi`), and clamping keeps all squared-distance and midpoint
/// arithmetic finite.
pub const MAX_COORDINATE: f32 = 1_000_000.0;

/// Target spacing between smoothed polyline samples, in physical pixels.
/// Fast flicks need sub-3px spacing; the old 6px target visibly faceted them.
const SAMPLE_SPACING: f32 = 3.0;

/// Hard cap on samples emitted for one raw segment. This bounds pathological
/// input without flattening ordinary fast curves (the old cap was 12).
const MAX_SAMPLES_PER_SEGMENT: usize = 64;

/// Extra pixels added to dirty/bounds rects for antialiasing and rounding.
const DIRTY_MARGIN: f32 = 1.0;

/// A point in physical screen pixels with sub-pixel precision.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhysicalPointF {
    pub x: f32,
    pub y: f32,
}

impl PhysicalPointF {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    pub fn distance(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// Rejects a non-finite pointer position and clamps a finite one to
/// [`MAX_COORDINATE`].
pub fn sanitize_position(position: PhysicalPointF) -> Option<PhysicalPointF> {
    if !position.is_finite() {
        return None;
    }
    Some(PhysicalPointF::new(
        position.x.clamp(-MAX_COORDINATE, MAX_COORDINATE),
        position.y.clamp(-MAX_COORDINATE, MAX_COORDINATE),
    ))
}

/// One sample of a rendered stroke polyline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeSample {
    pub position: PhysicalPointF,
    pub width: f32,
}

fn sample(point: &PenPoint, style: &PenStyle) -> StrokeSample {
    StrokeSample {
        position: point.position,
        width: style.width_at(point.pressure),
    }
}

fn midpoint(a: &PenPoint, b: &PenPoint, style: &PenStyle) -> StrokeSample {
    let left = sample(a, style);
    let right = sample(b, style);
    StrokeSample {
        position: PhysicalPointF::new(
            (left.position.x + right.position.x) * 0.5,
            (left.position.y + right.position.y) * 0.5,
        ),
        width: (left.width + right.width) * 0.5,
    }
}

/// Builds the smoothed, width-varying polyline for a raw point sequence.
pub fn smooth_samples(points: &[PenPoint], style: &PenStyle) -> Vec<StrokeSample> {
    let mut out = Vec::with_capacity(points.len().saturating_mul(2));
    match points.len() {
        0 => return out,
        1 => {
            out.push(sample(&points[0], style));
            return out;
        }
        2 => {
            emit_line(
                sample(&points[0], style),
                sample(&points[1], style),
                &mut out,
            );
            return out;
        }
        _ => {}
    }

    let start = sample(&points[0], style);
    let first_mid = midpoint(&points[0], &points[1], style);
    emit_line(start, first_mid, &mut out);

    for index in 1..points.len() - 1 {
        emit_quadratic(
            midpoint(&points[index - 1], &points[index], style),
            sample(&points[index], style),
            midpoint(&points[index], &points[index + 1], style),
            &mut out,
        );
    }

    let last_index = points.len() - 1;
    emit_line(
        midpoint(&points[last_index - 1], &points[last_index], style),
        sample(&points[last_index], style),
        &mut out,
    );
    out
}

/// Builds the geometry that has to be repainted on the active layer for the
/// new raw points. `rendered_points` is the raw-point count already on screen.
///
/// The dirty rect of a new point covers the last three raw points
/// ([`push_dirty_rect`]), so clearing it also erases the tail of the curve that
/// was already rendered for the previous point count. The repaint therefore
/// starts two controls back — one quadratic earlier than the new point — so
/// every pixel the clear removed inside the region is drawn again. The tail
/// still never re-tessellates the whole active stroke; a 1,000-point stroke
/// repaints at most the last two quadratics per message.
pub fn smooth_samples_increment(
    points: &[PenPoint],
    style: &PenStyle,
    rendered_points: usize,
) -> Vec<StrokeSample> {
    if points.len() < 3 || rendered_points < 2 || rendered_points >= points.len() {
        return smooth_samples(points, style);
    }
    // `rendered_points - 1` is the control of the first new quadratic; one
    // control earlier is already on the layer but its dirty region is cleared,
    // so it must be part of the repaint (see the module's smoothing notes).
    let first_control = rendered_points.saturating_sub(2).max(1);
    let last_control = points.len() - 2;
    let mut out = Vec::new();
    for index in first_control..=last_control {
        emit_quadratic(
            midpoint(&points[index - 1], &points[index], style),
            sample(&points[index], style),
            midpoint(&points[index], &points[index + 1], style),
            &mut out,
        );
    }
    let last = points.len() - 1;
    emit_line(
        midpoint(&points[last - 1], &points[last], style),
        sample(&points[last], style),
        &mut out,
    );
    out
}

fn emit_line(start: StrokeSample, end: StrokeSample, out: &mut Vec<StrokeSample>) {
    push_if_changed(out, start);
    push_if_changed(out, end);
}

fn emit_quadratic(
    start: StrokeSample,
    control: StrokeSample,
    end: StrokeSample,
    out: &mut Vec<StrokeSample>,
) {
    push_if_changed(out, start);
    let count = sample_count(start.position, control.position, end.position);
    for step in 1..=count {
        let t = step as f32 / count as f32;
        let position = quadratic(start.position, control.position, end.position, t);
        let inverse = 1.0 - t;
        let width =
            inverse * inverse * start.width + 2.0 * inverse * t * control.width + t * t * end.width;
        push_if_changed(out, StrokeSample { position, width });
    }
}

fn push_if_changed(out: &mut Vec<StrokeSample>, sample: StrokeSample) {
    if out.last() != Some(&sample) {
        out.push(sample);
    }
}

fn sample_count(a: PhysicalPointF, control: PhysicalPointF, end: PhysicalPointF) -> usize {
    let length = a.distance(control) + control.distance(end);
    let estimate = (length / SAMPLE_SPACING).ceil();
    if !estimate.is_finite() {
        return MAX_SAMPLES_PER_SEGMENT;
    }
    (estimate as usize).clamp(1, MAX_SAMPLES_PER_SEGMENT)
}

fn quadratic(
    start: PhysicalPointF,
    control: PhysicalPointF,
    end: PhysicalPointF,
    t: f32,
) -> PhysicalPointF {
    let inverse = 1.0 - t;
    let a = inverse * inverse;
    let b = 2.0 * inverse * t;
    let c = t * t;
    PhysicalPointF::new(
        a * start.x + b * control.x + c * end.x,
        a * start.y + b * control.y + c * end.y,
    )
}

/// The dirty rect for the most recently pushed point.
///
/// The newest quadratic is controlled by the previous sample and spans the
/// last three raw points; the straight cap ends at the new point. Both fit in
/// the bounding box of the last three points expanded by the style's maximum
/// half width. Only this local segment is returned, so a 1,000-point stroke
/// does not dirty its whole bounds on every point.
pub fn push_dirty_rect(points: &[PenPoint], style: &PenStyle) -> DirtyRect {
    let Some(last) = points.last() else {
        return DirtyRect::EMPTY;
    };
    let margin = style.max_width() * 0.5 + DIRTY_MARGIN;
    let start = points.len().saturating_sub(3);
    let mut min_x = last.position.x;
    let mut max_x = last.position.x;
    let mut min_y = last.position.y;
    let mut max_y = last.position.y;
    for point in &points[start..] {
        min_x = min_x.min(point.position.x);
        max_x = max_x.max(point.position.x);
        min_y = min_y.min(point.position.y);
        max_y = max_y.max(point.position.y);
    }
    DirtyRect::from_f32_bounds(
        min_x - margin,
        min_y - margin,
        max_x + margin,
        max_y + margin,
    )
}

/// Distance from `center` to the stroke's raw polyline, widened by the
/// largest possible half width. Used for Microsoft.Ink-style stroke erase:
/// the whole stroke is removed when the eraser touches its geometry.
pub fn stroke_intersects_circle(
    points: &[PenPoint],
    style: &PenStyle,
    center: PhysicalPointF,
    radius: f32,
) -> bool {
    let Some(first) = points.first() else {
        return false;
    };
    let threshold = radius + style.max_width() * 0.5;
    let threshold_sq = threshold * threshold;
    if points.len() == 1 {
        return distance_squared(center, first.position) <= threshold_sq;
    }
    points.windows(2).any(|pair| {
        point_segment_distance_squared(center, pair[0].position, pair[1].position) <= threshold_sq
    })
}

fn distance_squared(a: PhysicalPointF, b: PhysicalPointF) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

fn point_segment_distance_squared(
    point: PhysicalPointF,
    start: PhysicalPointF,
    end: PhysicalPointF,
) -> f32 {
    let ab_x = end.x - start.x;
    let ab_y = end.y - start.y;
    let length_sq = ab_x * ab_x + ab_y * ab_y;
    if length_sq <= f32::EPSILON {
        return distance_squared(point, start);
    }
    let projection = ((point.x - start.x) * ab_x + (point.y - start.y) * ab_y) / length_sq;
    let t = projection.clamp(0.0, 1.0);
    let closest = PhysicalPointF::new(start.x + ab_x * t, start.y + ab_y * t);
    distance_squared(point, closest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Rgba8;
    use std::time::Duration;

    fn point(x: f32, y: f32) -> PenPoint {
        PenPoint::new(PhysicalPointF::new(x, y), 0.5, Duration::from_millis(1))
    }

    #[test]
    fn duplicate_points_produce_finite_samples() {
        let style = PenStyle::new(Rgba8::new(0, 0, 0, 255), 8.0, true);
        let points = vec![point(10.0, 10.0), point(10.0, 10.0), point(10.0, 10.0)];
        let samples = smooth_samples(&points, &style);
        assert!(!samples.is_empty());
        for sample in samples {
            assert!(sample.position.is_finite());
            assert!(sample.width.is_finite() && sample.width > 0.0);
        }
    }

    #[test]
    fn point_segment_distance_handles_degenerate_segment() {
        let a = PhysicalPointF::new(10.0, 10.0);
        let distance = point_segment_distance_squared(a, a, a);
        assert!(distance.is_finite());
        assert!(distance.abs() < f32::EPSILON);
    }
}
