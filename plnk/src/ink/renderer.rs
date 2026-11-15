//! Stroke rasterization with Direct2D.
//!
//! A stroke renders as one filled outline polygon: the smoothed samples define
//! a variable-width ribbon, and the two ends are closed with half-circle caps.
//! The outline is filled with the nonzero winding rule, so a self-overlapping
//! stroke — a fast zigzag, a curled highlighter — is painted exactly once and a
//! translucent highlighter does not darken where the ribbon overlaps itself.
//! That single-fill rule is the reason the renderer does not stamp segment
//! quads or per-point ellipses.
//!
//! Geometry is built in physical pixels; the renderer subtracts the surface
//! origin so strokes can carry screen coordinates independently of the overlay
//! window position. Nothing here uploads pixels: the Direct2D device context
//! target decides where the fill lands (static cache, active layer, or the
//! composition swap chain).

use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_FILL_MODE_WINDING,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Brush, ID2D1DeviceContext, ID2D1Factory1, ID2D1SolidColorBrush,
};
use windows_numerics::Vector2;

use crate::gfx::Rgba8;
use crate::gfx::device::{GfxError, failed};
use crate::ink::geometry::{PhysicalPointF, StrokeSample};
use crate::ink::model::Stroke;

/// Half-circle cap resolution (polyline segments per cap).
const CAP_SEGMENTS: usize = 8;
/// Resolution of the round dot painted for a single sample (a short tap).
const DOT_SEGMENTS: usize = 16;
/// Below this segment length the direction is undefined and the previous
/// direction is carried forward (duplicate samples are legal input).
const MIN_DIRECTION_LENGTH: f32 = 1.0e-4;

/// Renders strokes into one Direct2D device context.
///
/// The brush is cached per color; create one renderer per BeginDraw pass (a
/// brush belongs to the context that created it).
pub struct StrokeRenderer<'a> {
    context: &'a ID2D1DeviceContext,
    factory: &'a ID2D1Factory1,
    origin: PhysicalPointF,
    brush: Option<(Rgba8, ID2D1SolidColorBrush)>,
}

impl<'a> StrokeRenderer<'a> {
    /// `origin` is the screen position of the target surface's top-left
    /// corner, i.e. the value subtracted from physical stroke coordinates.
    pub fn new(
        context: &'a ID2D1DeviceContext,
        factory: &'a ID2D1Factory1,
        origin: PhysicalPointF,
    ) -> Self {
        Self {
            context,
            factory,
            origin,
            brush: None,
        }
    }

    /// Draws one complete stroke (smoothed samples + caps).
    pub fn draw(&mut self, stroke: &Stroke) -> Result<(), GfxError> {
        if stroke.is_empty() {
            return Ok(());
        }
        let samples = stroke.smoothed_samples();
        self.draw_samples(&samples, stroke.style().color)
    }

    /// Draws a prepared sample polyline. Exposed for the acceptance tests and
    /// for callers that already own the smoothed samples.
    pub fn draw_samples(&mut self, samples: &[StrokeSample], color: Rgba8) -> Result<(), GfxError> {
        if color.a == 0 {
            return Ok(());
        }
        let points = outline_points(samples, self.origin);
        if points.len() < 3 {
            return Ok(());
        }
        let geometry = unsafe { self.factory.CreatePathGeometry() }
            .map_err(failed("ID2D1Factory::CreatePathGeometry"))?;
        let sink = unsafe { geometry.Open() }.map_err(failed("ID2D1PathGeometry::Open"))?;
        unsafe {
            sink.SetFillMode(D2D1_FILL_MODE_WINDING);
            sink.BeginFigure(
                Vector2::new(points[0][0], points[0][1]),
                D2D1_FIGURE_BEGIN_FILLED,
            );
            for point in &points[1..] {
                sink.AddLine(Vector2::new(point[0], point[1]));
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }
        unsafe { sink.Close() }.map_err(failed("ID2D1GeometrySink::Close"))?;
        let brush = self.brush(color)?;
        // SAFETY: the geometry is a local object; the interface reference keeps
        // the brush alive for the draw call. Fill failure surfaces at EndDraw.
        unsafe {
            self.context
                .FillGeometry(&geometry, &brush, None::<&ID2D1Brush>);
        }
        Ok(())
    }

    /// Returns a brush for `color`, reusing the cached one when possible. The
    /// interface is cloned on return so the caller does not borrow the whole
    /// renderer (the context is borrowed separately).
    fn brush(&mut self, color: Rgba8) -> Result<ID2D1SolidColorBrush, GfxError> {
        if self
            .brush
            .as_ref()
            .is_none_or(|(cached, _)| *cached != color)
        {
            let d2d_color = D2D1_COLOR_F {
                r: f32::from(color.r) / 255.0,
                g: f32::from(color.g) / 255.0,
                b: f32::from(color.b) / 255.0,
                a: f32::from(color.a) / 255.0,
            };
            let brush = unsafe { self.context.CreateSolidColorBrush(&d2d_color, None) }
                .map_err(failed("ID2D1RenderTarget::CreateSolidColorBrush"))?;
            self.brush = Some((color, brush));
        }
        Ok(self
            .brush
            .as_ref()
            .expect("the brush was just created when absent")
            .1
            .clone())
    }
}

/// Builds the closed outline polygon for a stroke's smoothed samples.
///
/// Points are emitted in the surface's coordinate space (screen minus
/// `origin`). The polygon walks the left offset curve forward, half-circles the
/// end, walks the right offset curve back, and half-circles the start. Empty or
/// wholly non-finite input yields an empty polygon; a single sample yields a
/// round dot. Every returned coordinate is finite.
pub fn outline_points(samples: &[StrokeSample], origin: PhysicalPointF) -> Vec<[f32; 2]> {
    let samples: Vec<StrokeSample> = samples
        .iter()
        .filter(|sample| {
            sample.position.is_finite() && sample.width.is_finite() && sample.width > 0.0
        })
        .copied()
        .collect();
    let Some(first) = samples.first().copied() else {
        return Vec::new();
    };
    let centre = PhysicalPointF::new(first.position.x - origin.x, first.position.y - origin.y);
    if samples.len() == 1 {
        return dot_points(centre, first.width * 0.5);
    }

    let positions: Vec<PhysicalPointF> = samples
        .iter()
        .map(|sample| {
            PhysicalPointF::new(sample.position.x - origin.x, sample.position.y - origin.y)
        })
        .collect();
    let radii: Vec<f32> = samples.iter().map(|sample| sample.width * 0.5).collect();
    let directions = directions(&positions);

    let mut left = Vec::with_capacity(positions.len());
    let mut right = Vec::with_capacity(positions.len());
    for index in 0..positions.len() {
        let (dx, dy) = directions[index];
        let (nx, ny) = (-dy, dx);
        left.push([
            positions[index].x + nx * radii[index],
            positions[index].y + ny * radii[index],
        ]);
        right.push([
            positions[index].x - nx * radii[index],
            positions[index].y - ny * radii[index],
        ]);
    }

    let mut points = Vec::with_capacity(2 * positions.len() + 2 * CAP_SEGMENTS);
    points.extend_from_slice(&left);

    // End cap: walk the half circle around the last sample from the left
    // offset to the right offset through the forward direction.
    let last = positions.len() - 1;
    let (dx, dy) = directions[last];
    let end_base = dy.atan2(dx) + std::f32::consts::FRAC_PI_2;
    for step in 1..CAP_SEGMENTS {
        let angle = end_base - std::f32::consts::PI * step as f32 / CAP_SEGMENTS as f32;
        points.push([
            positions[last].x + radii[last] * angle.cos(),
            positions[last].y + radii[last] * angle.sin(),
        ]);
    }

    points.extend(right.iter().rev().copied());

    // Start cap: the same construction around the first sample, passing
    // through the backward direction.
    let (dx, dy) = directions[0];
    let start_base = dy.atan2(dx) - std::f32::consts::FRAC_PI_2;
    for step in 1..CAP_SEGMENTS {
        let angle = start_base - std::f32::consts::PI * step as f32 / CAP_SEGMENTS as f32;
        points.push([
            positions[0].x + radii[0] * angle.cos(),
            positions[0].y + radii[0] * angle.sin(),
        ]);
    }
    points
}

fn dot_points(centre: PhysicalPointF, radius: f32) -> Vec<[f32; 2]> {
    (0..DOT_SEGMENTS)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / DOT_SEGMENTS as f32;
            [
                centre.x + radius * angle.cos(),
                centre.y + radius * angle.sin(),
            ]
        })
        .collect()
}

fn directions(points: &[PhysicalPointF]) -> Vec<(f32, f32)> {
    let mut directions: Vec<Option<(f32, f32)>> = vec![None; points.len()];
    for index in 0..points.len().saturating_sub(1) {
        let dx = points[index + 1].x - points[index].x;
        let dy = points[index + 1].y - points[index].y;
        let length = (dx * dx + dy * dy).sqrt();
        if length > MIN_DIRECTION_LENGTH && length.is_finite() {
            directions[index] = Some((dx / length, dy / length));
        }
    }
    let mut last = (1.0f32, 0.0f32);
    for direction in &mut directions {
        match *direction {
            Some(current) => last = current,
            None => *direction = Some(last),
        }
    }
    directions
        .into_iter()
        .map(|direction| direction.unwrap_or((1.0, 0.0)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(x: f32, y: f32, width: f32) -> StrokeSample {
        StrokeSample {
            position: PhysicalPointF::new(x, y),
            width,
        }
    }

    #[test]
    fn straight_line_outline_is_a_closed_finite_ribbon() {
        let samples = [sample(10.0, 50.0, 20.0), sample(90.0, 50.0, 20.0)];
        let points = outline_points(&samples, PhysicalPointF::new(0.0, 0.0));
        assert!(points.len() >= 2 * 2 + 2 * (CAP_SEGMENTS - 1));
        assert!(
            points
                .iter()
                .all(|point| point[0].is_finite() && point[1].is_finite())
        );
        // Left side is +y (y grows downwards), right side is -y, both offset
        // by half the width.
        assert!(
            points
                .iter()
                .any(|p| (p[0] - 10.0).abs() < 1e-3 && (p[1] - 60.0).abs() < 1e-3)
        );
        assert!(
            points
                .iter()
                .any(|p| (p[0] - 10.0).abs() < 1e-3 && (p[1] - 40.0).abs() < 1e-3)
        );
        // Caps extend past both ends by the half width.
        let min_x = points.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
        let max_x = points
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            min_x <= 0.5 && max_x >= 99.5,
            "caps must extend past the ends"
        );
    }

    #[test]
    fn duplicate_samples_stay_finite_and_origin_is_subtracted() {
        let samples = [
            sample(100.0, 100.0, 8.0),
            sample(100.0, 100.0, 8.0),
            sample(100.0, 100.0, 8.0),
            sample(140.0, 130.0, 8.0),
        ];
        let points = outline_points(&samples, PhysicalPointF::new(100.0, 100.0));
        assert!(!points.is_empty());
        assert!(
            points
                .iter()
                .all(|point| point[0].is_finite() && point[1].is_finite())
        );
        assert!(
            points
                .iter()
                .all(|point| point[0] > -20.0 && point[1] > -20.0),
            "the surface origin must be subtracted from screen coordinates"
        );
    }

    #[test]
    fn single_sample_becomes_a_round_dot_and_bad_input_is_empty() {
        let dot = outline_points(&[sample(30.0, 30.0, 10.0)], PhysicalPointF::new(0.0, 0.0));
        assert_eq!(dot.len(), DOT_SEGMENTS);
        for point in &dot {
            let dx = point[0] - 30.0;
            let dy = point[1] - 30.0;
            assert!((dx * dx + dy * dy - 25.0).abs() < 1e-2, "dot radius");
        }
        assert!(outline_points(&[], PhysicalPointF::new(0.0, 0.0)).is_empty());
        assert!(
            outline_points(
                &[sample(f32::NAN, 0.0, 4.0), sample(0.0, 0.0, f32::NAN)],
                PhysicalPointF::new(0.0, 0.0)
            )
            .is_empty()
        );
    }
}
