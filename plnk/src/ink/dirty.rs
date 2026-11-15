//! Dirty rectangles and regions in physical screen pixels.
//!
//! Rendering only needs to repaint the pixels an edit or a live stroke point
//! touched. Rects use exclusive `right`/`bottom` edges like
//! [`crate::platform::dpi::PhysicalRect`], but are independent of the Windows
//! crate so the ink model stays headless.
//!
//! A [`DirtyRegion`] coalesces overlapping or touching rects on insertion. That
//! keeps one edit's region proportional to the affected strokes instead of
//! accumulating an unbounded list for a growing document.

/// A rectangle in physical pixels (`right`/`bottom` exclusive).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirtyRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl DirtyRect {
    /// An empty rectangle that can be unioned with geometry.
    pub const EMPTY: Self = Self {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };

    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Builds the enclosing pixel rect for floating-point geometry. A
    /// non-finite bound degrades to [`DirtyRect::EMPTY`] rather than a bogus
    /// rect; callers sanitize positions before they reach here.
    pub fn from_f32_bounds(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Self {
        if ![min_x, min_y, max_x, max_y]
            .iter()
            .all(|value| value.is_finite())
        {
            return Self::EMPTY;
        }
        Self {
            left: floor_to_i32(min_x),
            top: floor_to_i32(min_y),
            right: ceil_to_i32(max_x),
            bottom: ceil_to_i32(max_y),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    pub fn width(&self) -> i32 {
        (self.right - self.left).max(0)
    }

    pub fn height(&self) -> i32 {
        (self.bottom - self.top).max(0)
    }

    pub fn area(&self) -> u64 {
        if self.is_empty() {
            return 0;
        }
        self.width() as u64 * self.height() as u64
    }

    /// Union of two rects, ignoring empty inputs.
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    /// Grows the rect by `pixels` on every side.
    pub fn expand(self, pixels: i32) -> Self {
        if self.is_empty() {
            return Self::EMPTY;
        }
        Self {
            left: self.left.saturating_sub(pixels),
            top: self.top.saturating_sub(pixels),
            right: self.right.saturating_add(pixels),
            bottom: self.bottom.saturating_add(pixels),
        }
    }

    pub fn contains(&self, other: Self) -> bool {
        if other.is_empty() {
            return true;
        }
        !self.is_empty()
            && self.left <= other.left
            && self.top <= other.top
            && self.right >= other.right
            && self.bottom >= other.bottom
    }

    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        f64::from(x) >= f64::from(self.left)
            && f64::from(x) < f64::from(self.right)
            && f64::from(y) >= f64::from(self.top)
            && f64::from(y) < f64::from(self.bottom)
    }

    pub fn intersects(&self, other: Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    /// True when the rects overlap or share an edge; used to coalesce regions.
    pub fn intersects_or_touches(&self, other: Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.left <= other.right
            && other.left <= self.right
            && self.top <= other.bottom
            && other.top <= self.bottom
    }
}

fn floor_to_i32(value: f32) -> i32 {
    let value = value.floor();
    if value <= i32::MIN as f32 {
        i32::MIN
    } else if value >= i32::MAX as f32 {
        i32::MAX
    } else {
        value as i32
    }
}

fn ceil_to_i32(value: f32) -> i32 {
    let value = value.ceil();
    if value <= i32::MIN as f32 {
        i32::MIN
    } else if value >= i32::MAX as f32 {
        i32::MAX
    } else {
        value as i32
    }
}

/// A set of dirty rectangles. Rects are coalesced as they are added so a
/// region stays bounded by the affected geometry, not by how many points or
/// edits produced it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirtyRegion {
    rects: Vec<DirtyRect>,
}

impl DirtyRegion {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_rect(rect: DirtyRect) -> Self {
        let mut region = Self::new();
        region.push(rect);
        region
    }

    pub fn push(&mut self, rect: DirtyRect) {
        if rect.is_empty() {
            return;
        }
        let mut merged = rect;
        let mut index = 0;
        while index < self.rects.len() {
            if self.rects[index].intersects_or_touches(merged) {
                merged = merged.union(self.rects.remove(index));
                // A union can now touch an earlier rect; restart the scan.
                index = 0;
            } else {
                index += 1;
            }
        }
        self.rects.push(merged);
    }

    pub fn union(&mut self, other: &Self) {
        for &rect in &other.rects {
            self.push(rect);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn rects(&self) -> &[DirtyRect] {
        &self.rects
    }

    pub fn bounds(&self) -> DirtyRect {
        self.rects
            .iter()
            .copied()
            .fold(DirtyRect::EMPTY, DirtyRect::union)
    }

    /// True when one component of the region fully contains `rect`. Callers
    /// that only need conservative coverage use [`Self::bounds`].
    pub fn contains_rect(&self, rect: DirtyRect) -> bool {
        if rect.is_empty() {
            return true;
        }
        self.rects.iter().any(|component| component.contains(rect))
    }

    pub fn area(&self) -> u64 {
        self.rects.iter().map(DirtyRect::area).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_skips_empty_and_merges_touching_rects() {
        let a = DirtyRect::new(0, 0, 10, 10);
        let b = DirtyRect::new(10, 0, 20, 10);
        assert_eq!(a.union(DirtyRect::EMPTY), a);
        assert!(a.intersects_or_touches(b));
        assert!(!a.intersects(b));
        let mut region = DirtyRegion::new();
        region.push(a);
        region.push(b);
        region.push(DirtyRect::EMPTY);
        assert_eq!(region.rects().len(), 1);
        assert_eq!(region.bounds(), DirtyRect::new(0, 0, 20, 10));
    }

    #[test]
    fn separate_rects_stay_separate() {
        let mut region = DirtyRegion::new();
        region.push(DirtyRect::new(0, 0, 10, 10));
        region.push(DirtyRect::new(100, 100, 110, 110));
        assert_eq!(region.rects().len(), 2);
        assert_eq!(region.area(), 200);
        assert!(region.contains_rect(DirtyRect::new(100, 100, 110, 110)));
        assert!(!region.contains_rect(DirtyRect::new(0, 0, 110, 110)));
    }
}
