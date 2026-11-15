//! Scene graph and display list primitives mirroring GPUI element rendering pipeline.

use crate::render::color::Rgba;
use crate::render::geometry::{Corners, Edges, Point, Rect};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quad {
    pub bounds: Rect,
    pub background: Option<Rgba>,
    pub border_color: Option<Rgba>,
    pub border_width: Edges,
    pub corner_radii: Corners,
}

impl Quad {
    pub fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            background: None,
            border_color: None,
            border_width: Edges::zero(),
            corner_radii: Corners::zero(),
        }
    }

    pub fn with_background(mut self, bg: Rgba) -> Self {
        self.background = Some(bg);
        self
    }

    pub fn with_border(mut self, color: Rgba, width: f32) -> Self {
        self.border_color = Some(color);
        self.border_width = Edges::all(width);
        self
    }

    pub fn with_border_edges(mut self, color: Rgba, edges: Edges) -> Self {
        self.border_color = Some(color);
        self.border_width = edges;
        self
    }

    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radii = Corners::all(radius);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlyphRun {
    pub text: String,
    pub origin: Point,
    pub bounds: Rect,
    pub color: Rgba,
    pub font_size: f32,
    pub font_family: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl GlyphRun {
    pub fn new(text: String, origin: Point, bounds: Rect, color: Rgba, font_size: f32) -> Self {
        Self {
            text,
            origin,
            bounds,
            color,
            font_size,
            font_family: "monospace".to_string(),
            bold: false,
            italic: false,
            underline: false,
        }
    }

    pub fn with_font_family(mut self, family: &str) -> Self {
        self.font_family = family.to_string();
        self
    }

    pub fn with_style(mut self, bold: bool, italic: bool, underline: bool) -> Self {
        self.bold = bold;
        self.italic = italic;
        self.underline = underline;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub bounds: Rect,
    pub offset: Point,
    pub blur: f32,
    pub spread: f32,
    pub color: Rgba,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SceneNode {
    Quad(Quad),
    GlyphRun(GlyphRun),
    Shadow(Shadow),
    PushClip(Rect),
    PopClip,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    nodes: Vec<SceneNode>,
}

impl Scene {
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(cap),
        }
    }

    pub fn push_quad(&mut self, quad: Quad) {
        self.nodes.push(SceneNode::Quad(quad));
    }

    pub fn push_text(&mut self, glyph_run: GlyphRun) {
        self.nodes.push(SceneNode::GlyphRun(glyph_run));
    }

    pub fn push_shadow(&mut self, shadow: Shadow) {
        self.nodes.push(SceneNode::Shadow(shadow));
    }

    pub fn push_clip(&mut self, rect: Rect) {
        self.nodes.push(SceneNode::PushClip(rect));
    }

    pub fn pop_clip(&mut self) {
        self.nodes.push(SceneNode::PopClip);
    }

    pub fn append(&mut self, other: &mut Scene) {
        self.nodes.append(&mut other.nodes);
    }

    pub fn nodes(&self) -> &[SceneNode] {
        &self.nodes
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    // Helper queries for testing and inspection:

    pub fn quads(&self) -> Vec<&Quad> {
        self.nodes
            .iter()
            .filter_map(|node| match node {
                SceneNode::Quad(q) => Some(q),
                _ => None,
            })
            .collect()
    }

    pub fn glyph_runs(&self) -> Vec<&GlyphRun> {
        self.nodes
            .iter()
            .filter_map(|node| match node {
                SceneNode::GlyphRun(g) => Some(g),
                _ => None,
            })
            .collect()
    }

    pub fn all_text(&self) -> String {
        self.glyph_runs()
            .into_iter()
            .map(|g| g.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn contains_text(&self, needle: &str) -> bool {
        if self.glyph_runs().iter().any(|g| g.text.contains(needle)) {
            return true;
        }

        // Check if concatenated glyph runs on the same visual line contain the needle
        let mut current_y: Option<f32> = None;
        let mut line_buf = String::new();

        for run in self.glyph_runs() {
            if let Some(y) = current_y {
                if (run.origin.y - y).abs() > 1.0 {
                    if line_buf.contains(needle) {
                        return true;
                    }
                    line_buf.clear();
                    current_y = Some(run.origin.y);
                }
            } else {
                current_y = Some(run.origin.y);
            }
            line_buf.push_str(&run.text);
        }

        line_buf.contains(needle)
    }


    pub fn hit_test(&self, point: Point) -> Vec<&SceneNode> {
        self.nodes
            .iter()
            .filter(|node| match node {
                SceneNode::Quad(q) => q.bounds.contains(point),
                SceneNode::GlyphRun(g) => g.bounds.contains(point),
                _ => false,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scene_construction_and_query() {
        let mut scene = Scene::new();
        scene.push_quad(Quad::new(Rect::from_xywh(0.0, 0.0, 100.0, 100.0)).with_background(Rgba::white()));
        scene.push_text(GlyphRun::new(
            "Hello GPUI".to_string(),
            Point::new(10.0, 10.0),
            Rect::from_xywh(10.0, 10.0, 80.0, 20.0),
            Rgba::black(),
            14.0,
        ));

        assert_eq!(scene.quads().len(), 1);
        assert_eq!(scene.glyph_runs().len(), 1);
        assert!(scene.contains_text("Hello"));

        let hits = scene.hit_test(Point::new(15.0, 15.0));
        assert_eq!(hits.len(), 2); // Both quad and glyph run
    }
}
