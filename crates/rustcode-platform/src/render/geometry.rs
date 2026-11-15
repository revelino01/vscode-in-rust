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
}
