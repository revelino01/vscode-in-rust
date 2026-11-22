//! 2D geometry primitives matching GPUI layout and rendering conventions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn zero() -> Self {
        Self { x: 0.0, y: 0.0 }
    }

    pub fn offset(&self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub fn zero() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    pub fn all(v: f32) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    pub fn symmetric(vertical: f32, horizontal: f32) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }

    pub fn zero() -> Self {
        Self::all(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Corners {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Corners {
    pub fn new(top_left: f32, top_right: f32, bottom_right: f32, bottom_left: f32) -> Self {
        Self {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        }
    }

    pub fn all(r: f32) -> Self {
        Self {
            top_left: r,
            top_right: r,
            bottom_right: r,
            bottom_left: r,
        }
    }

    pub fn zero() -> Self {
        Self::all(0.0)
    }
}

/// Maximum finite coordinate magnitude accepted by geometry validation helpers.
pub const MAX_RECT_COORD: f32 = 1.0e15;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub fn new(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    pub fn from_xywh(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            origin: Point::new(x, y),
            size: Size::new(width.max(0.0), height.max(0.0)),
        }
    }

    pub fn zero() -> Self {
        Self {
            origin: Point::zero(),
            size: Size::zero(),
        }
    }

    pub fn min_x(&self) -> f32 {
        self.origin.x
    }

    pub fn max_x(&self) -> f32 {
        self.origin.x + self.size.width
    }

    pub fn min_y(&self) -> f32 {
        self.origin.y
    }

    pub fn max_y(&self) -> f32 {
        self.origin.y + self.size.height
    }

    pub fn width(&self) -> f32 {
        self.size.width
    }

    pub fn height(&self) -> f32 {
        self.size.height
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.min_x() && p.x <= self.max_x() && p.y >= self.min_y() && p.y <= self.max_y()
    }

    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.min_x().max(other.min_x());
        let y0 = self.min_y().max(other.min_y());
        let x1 = self.max_x().min(other.max_x());
        let y1 = self.max_y().min(other.max_y());

        if x1 > x0 && y1 > y0 {
            Some(Rect::from_xywh(x0, y0, x1 - x0, y1 - y0))
        } else {
            None
        }
    }

    pub fn inset(&self, edges: Edges) -> Rect {
        let x = self.origin.x + edges.left;
        let y = self.origin.y + edges.top;
        let width = (self.size.width - edges.left - edges.right).max(0.0);
        let height = (self.size.height - edges.top - edges.bottom).max(0.0);
        Rect::from_xywh(x, y, width, height)
    }

    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.origin.offset(dx, dy), self.size)
    }

    pub fn is_finite(&self) -> bool {
        self.min_x().is_finite()
            && self.min_y().is_finite()
            && self.width().is_finite()
            && self.height().is_finite()
            && self.max_x().is_finite()
            && self.max_y().is_finite()
            && self.origin.x.abs() <= MAX_RECT_COORD
            && self.origin.y.abs() <= MAX_RECT_COORD
            && self.size.width.abs() <= MAX_RECT_COORD
            && self.size.height.abs() <= MAX_RECT_COORD
            && (self.origin.x as f64 + self.size.width as f64).abs() <= f32::MAX as f64
            && (self.origin.y as f64 + self.size.height as f64).abs() <= f32::MAX as f64
    }

    pub fn is_finite_positive(&self) -> bool {
        self.is_finite() && self.size.width > 0.0 && self.size.height > 0.0
    }

    pub fn clamp_to_bounds(&self, bounds: Rect) -> Rect {
        if !bounds.is_finite_positive() {
            return Rect::zero();
        }
        let b_min_x = bounds.min_x();
        let b_max_x = bounds.max_x();
        let b_min_y = bounds.min_y();
        let b_max_y = bounds.max_y();

        let rx = if self.origin.x.is_nan() {
            b_min_x
        } else {
            self.origin.x
        };
        let ry = if self.origin.y.is_nan() {
            b_min_y
        } else {
            self.origin.y
        };
        let rw = if self.size.width.is_nan() || self.size.width <= 0.0 {
            0.0
        } else {
            self.size.width
        };
        let rh = if self.size.height.is_nan() || self.size.height <= 0.0 {
            0.0
        } else {
            self.size.height
        };

        let min_x = rx.clamp(b_min_x, b_max_x);
        let min_y = ry.clamp(b_min_y, b_max_y);

        let max_x = if rw.is_infinite() {
            b_max_x
        } else if rx.is_infinite() {
            min_x
        } else {
            (rx as f64 + rw as f64).clamp(min_x as f64, b_max_x as f64) as f32
        };

        let max_y = if rh.is_infinite() {
            b_max_y
        } else if ry.is_infinite() {
            min_y
        } else {
            (ry as f64 + rh as f64).clamp(min_y as f64, b_max_y as f64) as f32
        };

        Rect::from_xywh(
            min_x,
            min_y,
            (max_x - min_x).max(0.0),
            (max_y - min_y).max(0.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_bounds_and_contains() {
        let rect = Rect::from_xywh(10.0, 20.0, 100.0, 50.0);
        assert_eq!(rect.min_x(), 10.0);
        assert_eq!(rect.max_x(), 110.0);
        assert_eq!(rect.min_y(), 20.0);
        assert_eq!(rect.max_y(), 70.0);

        assert!(rect.contains(Point::new(10.0, 20.0)));
        assert!(rect.contains(Point::new(50.0, 45.0)));
        assert!(rect.contains(Point::new(110.0, 70.0)));
        assert!(!rect.contains(Point::new(9.9, 20.0)));
        assert!(!rect.contains(Point::new(110.1, 70.0)));
    }

    #[test]
    fn test_rect_intersect_and_inset() {
        let r1 = Rect::from_xywh(0.0, 0.0, 100.0, 100.0);
        let r2 = Rect::from_xywh(50.0, 50.0, 100.0, 100.0);
        let inter = r1.intersect(&r2).unwrap();
        assert_eq!(inter, Rect::from_xywh(50.0, 50.0, 50.0, 50.0));

        let inset = r1.inset(Edges::all(5.0));
        assert_eq!(inset, Rect::from_xywh(5.0, 5.0, 90.0, 90.0));
    }

    #[test]
    fn test_rect_is_finite_and_positive() {
        assert!(Rect::from_xywh(10.0, 20.0, 100.0, 50.0).is_finite());
        assert!(Rect::from_xywh(10.0, 20.0, 100.0, 50.0).is_finite_positive());

        let zero_w = Rect::from_xywh(0.0, 0.0, 0.0, 10.0);
        assert!(zero_w.is_finite());
        assert!(!zero_w.is_finite_positive());

        let neg_dim = Rect::new(Point::zero(), Size::new(-5.0, 10.0));
        assert!(!neg_dim.is_finite_positive());

        for bad in [
            Rect::new(Point::new(f32::NAN, 0.0), Size::new(10.0, 10.0)),
            Rect::new(Point::new(0.0, f32::INFINITY), Size::new(10.0, 10.0)),
            Rect::new(Point::new(0.0, f32::NEG_INFINITY), Size::new(10.0, 10.0)),
            Rect::new(Point::new(1.0e16, 0.0), Size::new(10.0, 10.0)),
            Rect::new(Point::zero(), Size::new(1.0e16, 10.0)),
        ] {
            assert!(!bad.is_finite(), "expected !is_finite for {bad:?}");
            assert!(!bad.is_finite_positive(), "expected !is_finite_positive for {bad:?}");
        }
    }

    #[test]
    fn test_rect_clamp_to_bounds_edge_cases() {
        let bounds = Rect::from_xywh(10.0, 20.0, 100.0, 50.0);

        // Child larger than container clamps cleanly without panic or inverted bounds
        let larger = Rect::from_xywh(0.0, 0.0, 500.0, 500.0);
        assert_eq!(larger.clamp_to_bounds(bounds), bounds);

        // Disjoint child beyond max edge clamps to zero-sized edge inside bounds
        let far = Rect::from_xywh(300.0, 300.0, 40.0, 40.0);
        let clamped_far = far.clamp_to_bounds(bounds);
        assert!(clamped_far.is_finite());
        assert_eq!(clamped_far, Rect::from_xywh(110.0, 70.0, 0.0, 0.0));

        // Non-finite child fields safely clamp inside bounds
        let non_finite = Rect::new(Point::new(f32::NAN, f32::INFINITY), Size::new(f32::INFINITY, -5.0));
        let clamped_nf = non_finite.clamp_to_bounds(bounds);
        assert!(clamped_nf.is_finite());
        assert_eq!(clamped_nf, Rect::from_xywh(10.0, 70.0, 100.0, 0.0));

        // -INFINITY origin + +INFINITY size spans full bounds; -INFINITY/NaN size clamps to 0.0
        let span_inf = Rect::new(
            Point::new(f32::NEG_INFINITY, f32::NEG_INFINITY),
            Size::new(f32::INFINITY, f32::INFINITY),
        );
        assert_eq!(span_inf.clamp_to_bounds(bounds), bounds);

        let neg_inf_size = Rect::new(
            Point::new(20.0, 30.0),
            Size::new(f32::NEG_INFINITY, f32::NAN),
        );
        assert_eq!(
            neg_inf_size.clamp_to_bounds(bounds),
            Rect::from_xywh(20.0, 30.0, 0.0, 0.0)
        );

        // Invalid container yields Rect::zero()
        assert_eq!(
            larger.clamp_to_bounds(Rect::from_xywh(0.0, 0.0, 0.0, 50.0)),
            Rect::zero()
        );
        assert_eq!(
            larger.clamp_to_bounds(Rect::new(Point::zero(), Size::new(f32::NAN, 50.0))),
            Rect::zero()
        );
    }
}
