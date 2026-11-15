//! Mockable render surface and canvas abstraction.

use crate::render::geometry::Size;
use crate::render::scene::Scene;

pub trait RenderSurface: Send + Sync {
    fn render_scene(&mut self, scene: &Scene);
    fn size(&self) -> Size;
}

#[derive(Debug, Clone, Default)]
pub struct MockRenderSurface {
    pub size: Size,
    pub rendered_scenes: Vec<Scene>,
}

impl MockRenderSurface {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            size: Size::new(width, height),
            rendered_scenes: Vec::new(),
        }
    }

    pub fn last_scene(&self) -> Option<&Scene> {
        self.rendered_scenes.last()
    }

    pub fn clear(&mut self) {
        self.rendered_scenes.clear();
    }
}

impl RenderSurface for MockRenderSurface {
    fn render_scene(&mut self, scene: &Scene) {
        self.rendered_scenes.push(scene.clone());
    }

    fn size(&self) -> Size {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::geometry::Rect;
    use crate::render::scene::Quad;

    #[test]
    fn test_mock_render_surface() {
        let mut surface = MockRenderSurface::new(800.0, 600.0);
        let mut scene = Scene::new();
        scene.push_quad(Quad::new(Rect::from_xywh(0.0, 0.0, 800.0, 600.0)));

        surface.render_scene(&scene);
        assert_eq!(surface.rendered_scenes.len(), 1);
        assert_eq!(surface.last_scene().unwrap().quads().len(), 1);
    }
}
