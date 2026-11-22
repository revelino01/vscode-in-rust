//! Hardware GPU render surface (`wgpu` + `cosmic-text`) and pure-Rust scene batching/clip/atlas primitives.

use crate::render::geometry::{Corners, Rect, MAX_RECT_COORD};
#[cfg(feature = "desktop")]
use crate::render::scene::GlyphRun;
use crate::render::scene::{Quad, Scene, SceneNode, Shadow};
use std::collections::HashMap;

#[cfg(feature = "desktop")]
use crate::render::geometry::Size;
#[cfg(feature = "desktop")]
use crate::render::surface::RenderSurface;
#[cfg(feature = "desktop")]
use std::sync::Arc;

/// Default glyph atlas texture dimension (width & height in texels).
pub const DEFAULT_ATLAS_SIZE: u32 = 1024;

/// Sanitize a HiDPI scale factor, falling back to `1.0` when non-finite or non-positive.
pub fn sanitize_scale_factor(scale_factor: f32) -> f32 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

/// Clamp `Corners` radii so each corner radius is finite, non-negative, and `<= min(width, height) / 2.0`.
pub fn clamp_corner_radii(radii: Corners, width: f32, height: f32) -> [f32; 4] {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return [0.0; 4];
    }
    let max_r = (width.min(height) * 0.5).max(0.0);
    let clamp_one = |r: f32| -> f32 {
        if r.is_finite() {
            r.clamp(0.0, max_r)
        } else {
            0.0
        }
    };
    [
        clamp_one(radii.top_left),
        clamp_one(radii.top_right),
        clamp_one(radii.bottom_right),
        clamp_one(radii.bottom_left),
    ]
}

/// Nested intersection clip stack matching `SceneNode::PushClip` / `SceneNode::PopClip`.
#[derive(Debug, Clone)]
pub struct ClipStack {
    viewport: Rect,
    stack: Vec<Rect>,
}

impl ClipStack {
    pub fn new(viewport: Rect) -> Self {
        let safe_viewport = if viewport.is_finite_positive() {
            viewport
        } else {
            Rect::zero()
        };
        Self {
            viewport: safe_viewport,
            stack: Vec::new(),
        }
    }

    pub fn set_viewport(&mut self, viewport: Rect) {
        self.viewport = if viewport.is_finite_positive() {
            viewport
        } else {
            Rect::zero()
        };
        self.stack.clear();
    }

    /// Push a clip rectangle, intersecting with the current active clip (or viewport).
    /// Non-finite or disjoint rectangles clamp to a zero-area clip so matching `pop()` stays balanced.
    pub fn push(&mut self, rect: Rect) {
        let base = self.current();
        let next = if rect.is_finite_positive() && base.is_finite_positive() {
            base.intersect(&rect).unwrap_or_else(Rect::zero)
        } else {
            Rect::zero()
        };
        self.stack.push(next);
    }

    /// Pop the top clip rectangle. Unbalanced pops on an empty stack are a safe no-op.
    pub fn pop(&mut self) {
        let _ = self.stack.pop();
    }

    /// Active clip rectangle in logical coordinates.
    pub fn current(&self) -> Rect {
        self.stack.last().copied().unwrap_or(self.viewport)
    }

    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty_clip(&self) -> bool {
        !self.current().is_finite_positive()
    }

    /// Compute a hardware `wgpu::RenderPass::set_scissor_rect` `[x, y, width, height]`
    /// in physical framebuffer pixels, clamped to `[0..fb_width, 0..fb_height]`.
    /// Returns `None` when the clip area is zero or entirely offscreen (caller should skip draw).
    pub fn scissor_rect(&self, scale_factor: f32, fb_width: u32, fb_height: u32) -> Option<[u32; 4]> {
        if fb_width == 0 || fb_height == 0 {
            return None;
        }
        let cur = self.current();
        if !cur.is_finite_positive() {
            return None;
        }
        let scale = sanitize_scale_factor(scale_factor);
        let fw = fb_width as f32;
        let fh = fb_height as f32;

        let x0 = (cur.min_x() * scale).floor().clamp(0.0, fw) as u32;
        let y0 = (cur.min_y() * scale).floor().clamp(0.0, fh) as u32;
        let x1 = (cur.max_x() * scale).ceil().clamp(0.0, fw) as u32;
        let y1 = (cur.max_y() * scale).ceil().clamp(0.0, fh) as u32;

        if x1 <= x0 || y1 <= y0 {
            None
        } else {
            Some([x0, y0, x1 - x0, y1 - y0])
        }
    }
}

/// Global viewport uniform passed to all WGSL vertex/fragment shaders.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "desktop", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct GlobalsUniform {
    /// `[physical_width, physical_height, scale_factor, 0.0]`
    pub viewport: [f32; 4],
}

/// GPU instance payload for `SceneNode::Quad` in physical pixel coordinates.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "desktop", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct QuadInstance {
    /// `[x, y, width, height]` in physical pixels
    pub bounds: [f32; 4],
    /// `[r, g, b, a]` fill color
    pub bg_color: [f32; 4],
    /// `[r, g, b, a]` border color
    pub border_color: [f32; 4],
    /// `[top, right, bottom, left]` in physical pixels
    pub border_widths: [f32; 4],
    /// `[top_left, top_right, bottom_right, bottom_left]` clamped in physical pixels
    pub corner_radii: [f32; 4],
}

impl QuadInstance {
    /// Convert a logical `SceneNode::Quad` into a physical-pixel `QuadInstance`.
    /// Returns `None` for non-finite or zero/negative-area bounds.
    pub fn from_quad(quad: &Quad, scale_factor: f32) -> Option<Self> {
        if !quad.bounds.is_finite_positive() {
            return None;
        }
        let scale = sanitize_scale_factor(scale_factor);
        let x = quad.bounds.min_x() * scale;
        let y = quad.bounds.min_y() * scale;
        let w = quad.bounds.width() * scale;
        let h = quad.bounds.height() * scale;
        if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
            return None;
        }

        let bg_color = quad
            .background
            .map_or([0.0; 4], |c| [c.r, c.g, c.b, c.a]);
        let border_color = quad
            .border_color
            .map_or([0.0; 4], |c| [c.r, c.g, c.b, c.a]);

        let sanitize_edge = |v: f32| -> f32 {
            if v.is_finite() && v > 0.0 {
                (v * scale).min(w.min(h))
            } else {
                0.0
            }
        };
        let border_widths = [
            sanitize_edge(quad.border_width.top),
            sanitize_edge(quad.border_width.right),
            sanitize_edge(quad.border_width.bottom),
            sanitize_edge(quad.border_width.left),
        ];

        let scaled_corners = Corners::new(
            quad.corner_radii.top_left * scale,
            quad.corner_radii.top_right * scale,
            quad.corner_radii.bottom_right * scale,
            quad.corner_radii.bottom_left * scale,
        );
        let corner_radii = clamp_corner_radii(scaled_corners, w, h);

        Some(Self {
            bounds: [x, y, w, h],
            bg_color,
            border_color,
            border_widths,
            corner_radii,
        })
    }

    pub fn is_visible(&self) -> bool {
        let has_bg = self.bg_color[3] > 0.0;
        let has_border = self.border_color[3] > 0.0
            && self.border_widths.iter().any(|&w| w > 0.0);
        has_bg || has_border
    }
}

/// GPU instance payload for `SceneNode::Shadow` in physical pixel coordinates.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "desktop", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct ShadowInstance {
    /// Expanded outer quad `[x, y, width, height]` in physical pixels
    pub bounds: [f32; 4],
    /// Core shadow box `[x, y, width, height]` after offset & spread in physical pixels
    pub shadow_rect: [f32; 4],
    /// `[r, g, b, a]`
    pub color: [f32; 4],
    /// `[blur_px, spread_px, corner_radius_px, 0.0]`
    pub params: [f32; 4],
}

impl ShadowInstance {
    /// Convert a logical `SceneNode::Shadow` into a physical-pixel `ShadowInstance`.
    /// Safely handles `blur == 0.0`, negative `spread`, and non-finite inputs without panicking.
    pub fn from_shadow(shadow: &Shadow, scale_factor: f32) -> Option<Self> {
        if !shadow.bounds.is_finite_positive()
            || !shadow.offset.x.is_finite()
            || !shadow.offset.y.is_finite()
            || shadow.offset.x.abs() > MAX_RECT_COORD
            || shadow.offset.y.abs() > MAX_RECT_COORD
            || !shadow.blur.is_finite()
            || shadow.blur > MAX_RECT_COORD
            || !shadow.spread.is_finite()
            || shadow.spread.abs() > MAX_RECT_COORD
            || !shadow.color.a.is_finite()
            || shadow.color.a <= 0.0
        {
            return None;
        }
        let scale = sanitize_scale_factor(scale_factor);
        let raw_blur = shadow.blur.max(0.0);
        if !(raw_blur * 2.0 + shadow.spread.abs()).is_finite() {
            return None;
        }
        let blur = raw_blur * scale;
        let spread = shadow.spread * scale;
        if !blur.is_finite()
            || !spread.is_finite()
            || blur > MAX_RECT_COORD
            || spread.abs() > MAX_RECT_COORD
            || !(blur * 2.0 + spread.abs()).is_finite()
        {
            return None;
        }

        let core_w = shadow.bounds.width() * scale + 2.0 * spread;
        let core_h = shadow.bounds.height() * scale + 2.0 * spread;
        if !core_w.is_finite() || !core_h.is_finite() || core_w <= 0.0 || core_h <= 0.0 {
            return None;
        }

        let core_x = (shadow.bounds.min_x() + shadow.offset.x) * scale - spread;
        let core_y = (shadow.bounds.min_y() + shadow.offset.y) * scale - spread;
        if !core_x.is_finite() || !core_y.is_finite() {
            return None;
        }

        let pad = blur.max(1.0);
        let outer_x = core_x - pad;
        let outer_y = core_y - pad;
        let outer_w = core_w + 2.0 * pad;
        let outer_h = core_h + 2.0 * pad;
        let corner_radius = (4.0 * scale).min(core_w.min(core_h) * 0.5);
        if !outer_x.is_finite()
            || !outer_y.is_finite()
            || !outer_w.is_finite()
            || !outer_h.is_finite()
            || outer_w <= 0.0
            || outer_h <= 0.0
            || !corner_radius.is_finite()
        {
            return None;
        }

        Some(Self {
            bounds: [outer_x, outer_y, outer_w, outer_h],
            shadow_rect: [core_x, core_y, core_w, core_h],
            color: [shadow.color.r, shadow.color.g, shadow.color.b, shadow.color.a],
            params: [blur, spread, corner_radius, 0.0],
        })
    }
}

/// GPU instance payload for a single rasterized glyph quad in physical pixels.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "desktop", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct GlyphInstance {
    /// `[x, y, width, height]` in physical pixels
    pub pos_size: [f32; 4],
    /// `[u0, v0, u1, v1]` normalized atlas coordinates
    pub uv_rect: [f32; 4],
    /// `[r, g, b, a]`
    pub color: [f32; 4],
    /// `[is_color (1.0 or 0.0), 0.0, 0.0, 0.0]`
    pub mode: [f32; 4],
}

/// Cache key identifying a rasterized glyph in the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphCacheKey {
    pub font_id: u64,
    pub glyph_id: u16,
    pub font_size_bits: u32,
    pub x_bin: u8,
    pub y_bin: u8,
    pub flags: u8,
}

/// Atlas placement record for a cached glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphAtlasRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub is_color: bool,
    pub last_used_frame: u64,
}

impl GlyphAtlasRect {
    pub fn uv_rect(&self, atlas_width: u32, atlas_height: u32) -> [f32; 4] {
        if atlas_width == 0 || atlas_height == 0 || self.width == 0 || self.height == 0 {
            return [0.0; 4];
        }
        let aw = atlas_width as f32;
        let ah = atlas_height as f32;
        [
            self.x as f32 / aw,
            self.y as f32 / ah,
            (self.x + self.width) as f32 / aw,
            (self.y + self.height) as f32 / ah,
        ]
    }
}

/// Result of attempting to fetch or insert a glyph into `GlyphAtlasAllocator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtlasInsertResult {
    Hit(GlyphAtlasRect),
    Allocated(GlyphAtlasRect),
    AllocatedAfterReset(GlyphAtlasRect),
    TooLarge,
}

/// Pure-Rust shelf-packing glyph atlas allocator with frame-aware hit tracking and overflow reset.
#[derive(Debug, Clone)]
pub struct GlyphAtlasAllocator {
    width: u32,
    height: u32,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    frame_counter: u64,
    hit_count: u64,
    miss_count: u64,
    reset_count: u64,
    entries: HashMap<GlyphCacheKey, GlyphAtlasRect>,
}

impl Default for GlyphAtlasAllocator {
    fn default() -> Self {
        Self::new(DEFAULT_ATLAS_SIZE, DEFAULT_ATLAS_SIZE)
    }
}

impl GlyphAtlasAllocator {
    pub fn new(width: u32, height: u32) -> Self {
        let w = width.max(16);
        let h = height.max(16);
        Self {
            width: w,
            height: h,
            cursor_x: 1,
            cursor_y: 1,
            row_height: 0,
            frame_counter: 0,
            hit_count: 0,
            miss_count: 0,
            reset_count: 0,
            entries: HashMap::new(),
        }
    }

    pub fn begin_frame(&mut self) {
        self.frame_counter = self.frame_counter.saturating_add(1);
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn hit_count(&self) -> u64 {
        self.hit_count
    }

    pub fn miss_count(&self) -> u64 {
        self.miss_count
    }

    pub fn reset_count(&self) -> u64 {
        self.reset_count
    }

    pub fn get(&mut self, key: &GlyphCacheKey) -> Option<GlyphAtlasRect> {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_used_frame = self.frame_counter;
            self.hit_count = self.hit_count.saturating_add(1);
            Some(*entry)
        } else {
            None
        }
    }

    pub fn insert(
        &mut self,
        key: GlyphCacheKey,
        glyph_width: u32,
        glyph_height: u32,
        left: i32,
        top: i32,
        is_color: bool,
    ) -> AtlasInsertResult {
        if let Some(existing) = self.get(&key) {
            return AtlasInsertResult::Hit(existing);
        }
        self.miss_count = self.miss_count.saturating_add(1);

        // Zero-area glyphs (e.g. spaces) occupy no atlas texels.
        if glyph_width == 0 || glyph_height == 0 {
            let rect = GlyphAtlasRect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                left,
                top,
                is_color,
                last_used_frame: self.frame_counter,
            };
            self.entries.insert(key, rect);
            return AtlasInsertResult::Allocated(rect);
        }

        // Leave 1px padding around every glyph to avoid filtering bleed.
        let fits_width = matches!(glyph_width.checked_add(2), Some(w) if w <= self.width);
        let fits_height = matches!(glyph_height.checked_add(2), Some(h) if h <= self.height);
        if !fits_width || !fits_height {
            return AtlasInsertResult::TooLarge;
        }

        if let Some(rect) = self.try_allocate(glyph_width, glyph_height, left, top, is_color) {
            self.entries.insert(key, rect);
            return AtlasInsertResult::Allocated(rect);
        }

        // Atlas is full: clear stale entries and start from the top shelf.
        self.reset();
        if let Some(rect) = self.try_allocate(glyph_width, glyph_height, left, top, is_color) {
            self.entries.insert(key, rect);
            AtlasInsertResult::AllocatedAfterReset(rect)
        } else {
            AtlasInsertResult::TooLarge
        }
    }

    pub fn reset(&mut self) {
        self.entries.clear();
        self.cursor_x = 1;
        self.cursor_y = 1;
        self.row_height = 0;
        self.reset_count = self.reset_count.saturating_add(1);
    }

    fn try_allocate(
        &mut self,
        glyph_width: u32,
        glyph_height: u32,
        left: i32,
        top: i32,
        is_color: bool,
    ) -> Option<GlyphAtlasRect> {
        if self.cursor_x.saturating_add(glyph_width).saturating_add(1) > self.width {
            self.cursor_x = 1;
            self.cursor_y = self
                .cursor_y
                .saturating_add(self.row_height.saturating_add(1));
            self.row_height = 0;
        }
        if self.cursor_y.saturating_add(glyph_height).saturating_add(1) > self.height {
            return None;
        }
        let rect = GlyphAtlasRect {
            x: self.cursor_x,
            y: self.cursor_y,
            width: glyph_width,
            height: glyph_height,
            left,
            top,
            is_color,
            last_used_frame: self.frame_counter,
        };
        self.cursor_x = self.cursor_x.saturating_add(glyph_width.saturating_add(1));
        self.row_height = self.row_height.max(glyph_height);
        Some(rect)
    }
}

/// Ordered GPU draw command produced by scene batching.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawCommand {
    SetScissor(Option<[u32; 4]>),
    DrawQuads(Vec<QuadInstance>),
    DrawShadows(Vec<ShadowInstance>),
    DrawGlyphs(Vec<GlyphInstance>),
}

/// Telemetry counters for a single rendered frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderFrameStats {
    pub quad_count: usize,
    pub shadow_count: usize,
    pub glyph_count: usize,
    pub draw_calls: usize,
    pub scissor_changes: usize,
    pub atlas_resets: u64,
    pub skipped_clipped: usize,
}

/// Pure-Rust scene batcher preserving z-order across `Quad`, `Shadow`, `PushClip`, and `PopClip`.
#[derive(Debug, Default)]
pub struct SceneBatchBuilder {
    pub commands: Vec<DrawCommand>,
    pub stats: RenderFrameStats,
    active_scissor: Option<Option<[u32; 4]>>,
}

impl SceneBatchBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build_non_text(
        scene: &Scene,
        logical_viewport: Rect,
        scale_factor: f32,
        fb_width: u32,
        fb_height: u32,
    ) -> Self {
        let mut builder = Self::new();
        let mut clips = ClipStack::new(logical_viewport);
        for node in scene.nodes() {
            match node {
                SceneNode::PushClip(rect) => clips.push(*rect),
                SceneNode::PopClip => clips.pop(),
                SceneNode::Quad(quad) => {
                    let scissor = clips.scissor_rect(scale_factor, fb_width, fb_height);
                    if scissor.is_none() {
                        builder.stats.skipped_clipped += 1;
                        continue;
                    }
                    if let Some(inst) = QuadInstance::from_quad(quad, scale_factor) {
                        if inst.is_visible() {
                            builder.push_quad(inst, scissor);
                        }
                    }
                }
                SceneNode::Shadow(shadow) => {
                    let scissor = clips.scissor_rect(scale_factor, fb_width, fb_height);
                    if scissor.is_none() {
                        builder.stats.skipped_clipped += 1;
                        continue;
                    }
                    if let Some(inst) = ShadowInstance::from_shadow(shadow, scale_factor) {
                        builder.push_shadow(inst, scissor);
                    }
                }
                SceneNode::GlyphRun(_) => {}
            }
        }
        builder
    }

    pub fn ensure_scissor(&mut self, scissor: Option<[u32; 4]>) {
        if self.active_scissor != Some(scissor) {
            self.active_scissor = Some(scissor);
            self.commands.push(DrawCommand::SetScissor(scissor));
            self.stats.scissor_changes += 1;
        }
    }

    pub fn push_quad(&mut self, inst: QuadInstance, scissor: Option<[u32; 4]>) {
        self.ensure_scissor(scissor);
        self.stats.quad_count += 1;
        if let Some(DrawCommand::DrawQuads(ref mut vec)) = self.commands.last_mut() {
            vec.push(inst);
        } else {
            self.commands.push(DrawCommand::DrawQuads(vec![inst]));
            self.stats.draw_calls += 1;
        }
    }

    pub fn push_shadow(&mut self, inst: ShadowInstance, scissor: Option<[u32; 4]>) {
        self.ensure_scissor(scissor);
        self.stats.shadow_count += 1;
        if let Some(DrawCommand::DrawShadows(ref mut vec)) = self.commands.last_mut() {
            vec.push(inst);
        } else {
            self.commands.push(DrawCommand::DrawShadows(vec![inst]));
            self.stats.draw_calls += 1;
        }
    }

    pub fn push_glyph(&mut self, inst: GlyphInstance, scissor: Option<[u32; 4]>) {
        self.ensure_scissor(scissor);
        self.stats.glyph_count += 1;
        if let Some(DrawCommand::DrawGlyphs(ref mut vec)) = self.commands.last_mut() {
            vec.push(inst);
        } else {
            self.commands.push(DrawCommand::DrawGlyphs(vec![inst]));
            self.stats.draw_calls += 1;
        }
    }

    /// Force a break in the current glyph batch (used when the atlas resets mid-frame).
    pub fn split_glyph_batch(&mut self) {
        self.active_scissor = None;
    }
}

/// WGSL shader for instanced SDF rounded-corner quads with asymmetric borders.
pub const QUAD_WGSL: &str = r#"
struct Uniforms {
    viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u_globals: Uniforms;

struct QuadIn {
    @location(0) bounds: vec4<f32>,
    @location(1) bg_color: vec4<f32>,
    @location(2) border_color: vec4<f32>,
    @location(3) border_widths: vec4<f32>,
    @location(4) corner_radii: vec4<f32>,
};

struct QuadOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) bg_color: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) border_widths: vec4<f32>,
    @location(5) corner_radii: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) v_idx: u32, inst: QuadIn) -> QuadOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0)
    );
    let unit = corners[v_idx];
    let px = inst.bounds.xy + unit * inst.bounds.zw;
    let ndc_x = (px.x / max(u_globals.viewport.x, 1.0)) * 2.0 - 1.0;
    let ndc_y = 1.0 - (px.y / max(u_globals.viewport.y, 1.0)) * 2.0;

    var out: QuadOut;
    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.local_pos = unit * inst.bounds.zw;
    out.size = inst.bounds.zw;
    out.bg_color = inst.bg_color;
    out.border_color = inst.border_color;
    out.border_widths = inst.border_widths;
    out.corner_radii = inst.corner_radii;
    return out;
}

fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_main(in: QuadOut) -> @location(0) vec4<f32> {
    let half_size = 0.5 * in.size;
    let p = in.local_pos - half_size;

    var r = in.corner_radii.x;
    if (p.x >= 0.0) {
        r = select(in.corner_radii.y, in.corner_radii.z, p.y >= 0.0);
    } else {
        r = select(in.corner_radii.x, in.corner_radii.w, p.y >= 0.0);
    }

    let dist_outer = sd_rounded_box(p, half_size, r);
    let outer_alpha = 1.0 - smoothstep(-0.5, 0.5, dist_outer);
    if (outer_alpha <= 0.0) {
        discard;
    }

    let bw = in.border_widths; // top, right, bottom, left
    let has_border = (in.border_color.a > 0.0) && (bw.x + bw.y + bw.z + bw.w > 0.0);

    let bg_pm = vec4<f32>(in.bg_color.rgb * in.bg_color.a, in.bg_color.a);
    if (!has_border) {
        return bg_pm * outer_alpha;
    }

    let inner_min = vec2<f32>(bw.w, bw.x);
    let inner_max = in.size - vec2<f32>(bw.y, bw.z);
    let inner_size = max(inner_max - inner_min, vec2<f32>(0.0));
    let border_pm = vec4<f32>(in.border_color.rgb * in.border_color.a, in.border_color.a);

    if (inner_size.x <= 0.0 || inner_size.y <= 0.0) {
        return border_pm * outer_alpha;
    }

    let inner_center = 0.5 * (inner_min + inner_max);
    let inner_half = 0.5 * inner_size;
    let inner_p = in.local_pos - inner_center;
    let max_adj_bw = max(
        select(bw.w, bw.y, p.x >= 0.0),
        select(bw.x, bw.z, p.y >= 0.0)
    );
    let inner_r = max(r - max_adj_bw, 0.0);
    let dist_inner = sd_rounded_box(inner_p, inner_half, inner_r);
    let inner_fill = 1.0 - smoothstep(-0.5, 0.5, dist_inner);

    return mix(border_pm, bg_pm, inner_fill) * outer_alpha;
}
"#;

/// WGSL shader for soft drop shadows (`SceneNode::Shadow`).
pub const SHADOW_WGSL: &str = r#"
struct Uniforms {
    viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u_globals: Uniforms;

struct ShadowIn {
    @location(0) bounds: vec4<f32>,
    @location(1) shadow_rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) params: vec4<f32>,
};

struct ShadowOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) frag_px: vec2<f32>,
    @location(1) shadow_rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) params: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) v_idx: u32, inst: ShadowIn) -> ShadowOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0)
    );
    let unit = corners[v_idx];
    let px = inst.bounds.xy + unit * inst.bounds.zw;
    let ndc_x = (px.x / max(u_globals.viewport.x, 1.0)) * 2.0 - 1.0;
    let ndc_y = 1.0 - (px.y / max(u_globals.viewport.y, 1.0)) * 2.0;

    var out: ShadowOut;
    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.frag_px = px;
    out.shadow_rect = inst.shadow_rect;
    out.color = inst.color;
    out.params = inst.params;
    return out;
}

fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_main(in: ShadowOut) -> @location(0) vec4<f32> {
    let center = in.shadow_rect.xy + 0.5 * in.shadow_rect.zw;
    let half_ext = 0.5 * in.shadow_rect.zw;
    let d = sd_rounded_box(in.frag_px - center, half_ext, in.params.z);
    let blur = in.params.x;

    var coverage: f32 = 0.0;
    if (blur <= 0.001) {
        coverage = 1.0 - smoothstep(-0.5, 0.5, d);
    } else {
        let t = 1.0 - smoothstep(-0.5, 1.0, d / blur);
        coverage = t * t;
    }

    let a = in.color.a * coverage;
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(in.color.rgb * a, a);
}
"#;

/// WGSL shader for glyph atlas quads (alpha-masked text and full-color emoji).
pub const GLYPH_WGSL: &str = r#"
struct Uniforms {
    viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u_globals: Uniforms;

@group(1) @binding(0)
var t_atlas: texture_2d<f32>;

@group(1) @binding(1)
var s_atlas: sampler;

struct GlyphIn {
    @location(0) pos_size: vec4<f32>,
    @location(1) uv_rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mode: vec4<f32>,
};

struct GlyphOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) mode: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) v_idx: u32, inst: GlyphIn) -> GlyphOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0)
    );
    let unit = corners[v_idx];
    let px = inst.pos_size.xy + unit * inst.pos_size.zw;
    let ndc_x = (px.x / max(u_globals.viewport.x, 1.0)) * 2.0 - 1.0;
    let ndc_y = 1.0 - (px.y / max(u_globals.viewport.y, 1.0)) * 2.0;

    var out: GlyphOut;
    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.uv = mix(inst.uv_rect.xy, inst.uv_rect.zw, unit);
    out.color = inst.color;
    out.mode = inst.mode;
    return out;
}

@fragment
fn fs_main(in: GlyphOut) -> @location(0) vec4<f32> {
    let texel = textureSample(t_atlas, s_atlas, in.uv);
    if (in.mode.x > 0.5) {
        let a = texel.a * in.color.a;
        return vec4<f32>(texel.rgb * a, a);
    } else {
        let a = texel.a * in.color.a;
        return vec4<f32>(in.color.rgb * a, a);
    }
}
"#;

// ============================================================================
// Desktop-Gated Hardware GPU + cosmic-text Implementation
// ============================================================================

#[cfg(feature = "desktop")]
mod desktop_impl {
    use super::*;
    use cosmic_text::{
        Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, SwashCache, SwashContent,
        Weight,
    };
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    use wgpu::util::DeviceExt;

    struct ThreadWaker(std::thread::Thread);

    impl std::task::Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        let mut fut = std::pin::pin!(fut);
        let waker = std::task::Waker::from(Arc::new(ThreadWaker(std::thread::current())));
        let mut cx = std::task::Context::from_waker(&waker);
        loop {
            match fut.as_mut().poll(&mut cx) {
                std::task::Poll::Ready(val) => return val,
                std::task::Poll::Pending => std::thread::park(),
            }
        }
    }

    /// Inspectable shaped glyph metadata produced by `TextShaper`.
    #[derive(Debug, Clone)]
    pub struct ShapedGlyphInfo {
        pub glyph_id: u16,
        pub font_id: u64,
        pub x: f32,
        pub y: f32,
        pub advance_width: f32,
        pub has_bitmap: bool,
        pub is_color: bool,
    }

    /// Output of shaping a `GlyphRun` with `cosmic-text`.
    #[derive(Debug, Clone)]
    pub struct ShapedGlyphRun {
        pub text: String,
        pub glyphs: Vec<ShapedGlyphInfo>,
        pub total_advance_width: f32,
        pub baseline_y: f32,
    }

    /// Wrapper around `cosmic_text::FontSystem` and `SwashCache`.
    pub struct TextShaper {
        pub font_system: FontSystem,
        pub swash_cache: SwashCache,
        scratch_buffer: Buffer,
    }

    impl Default for TextShaper {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TextShaper {
        pub fn new() -> Self {
            let mut font_system = FontSystem::new();
            let swash_cache = SwashCache::new();
            let scratch_buffer = Buffer::new(&mut font_system, Metrics::new(14.0, 21.0));
            Self {
                font_system,
                swash_cache,
                scratch_buffer,
            }
        }

        pub fn load_font_bytes(&mut self, data: Vec<u8>) {
            self.font_system.db_mut().load_font_data(data);
        }

        fn map_family(name: &str) -> Family<'_> {
            let trimmed = name.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("monospace") {
                Family::Monospace
            } else if trimmed.eq_ignore_ascii_case("sans-serif") {
                Family::SansSerif
            } else if trimmed.eq_ignore_ascii_case("serif") {
                Family::Serif
            } else {
                Family::Name(trimmed)
            }
        }

        const MAX_SHAPED_RUN_CHARS: usize = 16_384;
        const MAX_SAFE_TEXT_ORIGIN_PX: f32 = 1.0e7;
        const MAX_SAFE_FONT_SIZE: f32 = 4096.0;

        fn is_valid_glyph_run(run: &GlyphRun, scale: f32) -> bool {
            !run.text.is_empty()
                && run.origin.x.is_finite()
                && run.origin.y.is_finite()
                && run.origin.x.abs() <= MAX_RECT_COORD
                && run.origin.y.abs() <= MAX_RECT_COORD
                && (run.origin.x * scale).is_finite()
                && (run.origin.y * scale).is_finite()
                && (run.origin.x * scale).abs() <= Self::MAX_SAFE_TEXT_ORIGIN_PX
                && (run.origin.y * scale).abs() <= Self::MAX_SAFE_TEXT_ORIGIN_PX
                && run.font_size.is_finite()
                && run.font_size > 0.0
                && run.font_size <= Self::MAX_SAFE_FONT_SIZE
                && (run.font_size * scale).is_finite()
                && (run.font_size * scale) <= Self::MAX_SAFE_FONT_SIZE * 16.0
        }

        fn truncate_run_text(text: &str) -> &str {
            if text.len() <= Self::MAX_SHAPED_RUN_CHARS {
                return text;
            }
            match text.char_indices().nth(Self::MAX_SHAPED_RUN_CHARS) {
                Some((byte_idx, _)) => &text[..byte_idx],
                None => text,
            }
        }

        /// Shape and rasterize-probe a `GlyphRun` without requiring a GPU device.
        pub fn shape_run(&mut self, run: &GlyphRun, scale_factor: f32) -> ShapedGlyphRun {
            let scale = sanitize_scale_factor(scale_factor);
            if !Self::is_valid_glyph_run(run, scale) {
                return ShapedGlyphRun {
                    text: String::new(),
                    glyphs: Vec::new(),
                    total_advance_width: 0.0,
                    baseline_y: 0.0,
                };
            }
            let font_size = run.font_size;
            let capped_text = Self::truncate_run_text(&run.text);
            let line_height = (font_size * 1.5).round().max(1.0);
            self.scratch_buffer
                .set_metrics(&mut self.font_system, Metrics::new(font_size, line_height));
            self.scratch_buffer.set_size(&mut self.font_system, None, None);

            let attrs = Attrs::new()
                .family(Self::map_family(&run.font_family))
                .weight(if run.bold { Weight::BOLD } else { Weight::NORMAL })
                .style(if run.italic { Style::Italic } else { Style::Normal });

            self.scratch_buffer.set_text(
                &mut self.font_system,
                capped_text,
                &attrs,
                Shaping::Advanced,
            );
            self.scratch_buffer
                .shape_until_scroll(&mut self.font_system, false);

            let mut glyphs = Vec::new();
            let mut total_advance_width = 0.0_f32;
            let mut baseline_y = 0.0_f32;

            for layout_run in self.scratch_buffer.layout_runs() {
                baseline_y = layout_run.line_y;
                total_advance_width = total_advance_width.max(layout_run.line_w);
                let run_y = run.origin.y + layout_run.line_y;
                if !run_y.is_finite() || (run_y * scale).abs() > Self::MAX_SAFE_TEXT_ORIGIN_PX {
                    continue;
                }
                for g in layout_run.glyphs.iter() {
                    let gx = (run.origin.x + g.x) * scale;
                    let gy = (run_y + g.y) * scale;
                    if !gx.is_finite()
                        || !gy.is_finite()
                        || gx.abs() > Self::MAX_SAFE_TEXT_ORIGIN_PX
                        || gy.abs() > Self::MAX_SAFE_TEXT_ORIGIN_PX
                    {
                        continue;
                    }
                    let phys = g.physical((run.origin.x, run_y), scale);
                    let img_opt = self
                        .swash_cache
                        .get_image(&mut self.font_system, phys.cache_key)
                        .as_ref();
                    let (has_bitmap, is_color) = match img_opt {
                        Some(img) => (
                            img.placement.width > 0 && img.placement.height > 0,
                            matches!(img.content, SwashContent::Color),
                        ),
                        None => (false, false),
                    };
                    glyphs.push(ShapedGlyphInfo {
                        glyph_id: g.glyph_id,
                        font_id: hash_font_id(g.font_id),
                        x: g.x,
                        y: g.y,
                        advance_width: g.w,
                        has_bitmap,
                        is_color,
                    });
                }
            }

            ShapedGlyphRun {
                text: capped_text.to_string(),
                glyphs,
                total_advance_width,
                baseline_y,
            }
        }
    }

    fn hash_font_id(id: cosmic_text::fontdb::ID) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        id.hash(&mut hasher);
        hasher.finish()
    }

    fn glyph_cache_key_from_cosmic(key: cosmic_text::CacheKey) -> GlyphCacheKey {
        GlyphCacheKey {
            font_id: hash_font_id(key.font_id),
            glyph_id: key.glyph_id,
            font_size_bits: key.font_size_bits,
            x_bin: key.x_bin as u8,
            y_bin: key.y_bin as u8,
            flags: key.flags.bits() as u8,
        }
    }

    fn convert_swash_image_to_rgba8(content: SwashContent, data: &[u8]) -> (Vec<u8>, bool) {
        match content {
            SwashContent::Color => (data.to_vec(), true),
            SwashContent::Mask => {
                let mut out = Vec::with_capacity(data.len() * 4);
                for &alpha in data {
                    out.extend_from_slice(&[255, 255, 255, alpha]);
                }
                (out, false)
            }
            SwashContent::SubpixelMask => {
                let (chunks, _) = data.as_chunks::<3>();
                let mut out = Vec::with_capacity(chunks.len() * 4);
                for rgb in chunks {
                    let avg = ((rgb[0] as u16 + rgb[1] as u16 + rgb[2] as u16) / 3) as u8;
                    out.extend_from_slice(&[255, 255, 255, avg]);
                }
                (out, false)
            }
        }
    }

    enum RenderTarget {
        WindowSurface {
            surface: wgpu::Surface<'static>,
            config: wgpu::SurfaceConfiguration,
        },
        Offscreen {
            texture: wgpu::Texture,
            view: wgpu::TextureView,
            format: wgpu::TextureFormat,
        },
    }

    /// Hardware GPU `RenderSurface` backed by `wgpu` and `cosmic-text`.
    pub struct WgpuRenderSurface {
        _instance: wgpu::Instance,
        device: wgpu::Device,
        queue: wgpu::Queue,
        target: RenderTarget,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
        is_minimized: bool,
        globals_buffer: wgpu::Buffer,
        globals_bind_group: wgpu::BindGroup,
        quad_pipeline: wgpu::RenderPipeline,
        shadow_pipeline: wgpu::RenderPipeline,
        glyph_pipeline: wgpu::RenderPipeline,
        atlas_texture: wgpu::Texture,
        atlas_bind_group: wgpu::BindGroup,
        atlas_allocator: GlyphAtlasAllocator,
        shaper: TextShaper,
        last_stats: RenderFrameStats,
    }

    impl WgpuRenderSurface {
        /// Create a `WgpuRenderSurface` attached to an owned `raw-window-handle` window (`Arc<W>`).
        pub fn new_for_window<W>(
            window: Arc<W>,
            width: u32,
            height: u32,
            scale_factor: f32,
        ) -> Result<Self, String>
        where
            W: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static,
        {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
            let surface = instance
                .create_surface(window)
                .map_err(|e| format!("Failed to create wgpu surface: {e}"))?;
            Self::init_with_surface(instance, surface, width, height, scale_factor)
        }

        /// Create a `WgpuRenderSurface` from a borrowed window reference.
        ///
        /// # Safety
        /// The caller must ensure `window` outlives the returned `WgpuRenderSurface`.
        pub unsafe fn new_for_raw_window<W>(
            window: &W,
            width: u32,
            height: u32,
            scale_factor: f32,
        ) -> Result<Self, String>
        where
            W: HasWindowHandle + HasDisplayHandle,
        {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
            let target = wgpu::SurfaceTargetUnsafe::from_window(window)
                .map_err(|e| format!("Invalid raw window handle: {e}"))?;
            let surface = instance
                .create_surface_unsafe(target)
                .map_err(|e| format!("Failed to create unsafe wgpu surface: {e}"))?;
            Self::init_with_surface(instance, surface, width, height, scale_factor)
        }

        /// Create a headless offscreen `WgpuRenderSurface` (no window handle required).
        /// Returns `Err(...)` cleanly if no compatible GPU adapter is available.
        pub fn new_headless(width: u32, height: u32, scale_factor: f32) -> Result<Self, String> {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
            let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: None,
                force_fallback_adapter: false,
            }))
            .map_err(|e| format!("No GPU adapter available for headless wgpu surface: {e}"))?;

            let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(|e| format!("Failed to request wgpu device: {e}"))?;

            let safe_w = width.max(1);
            let safe_h = height.max(1);
            let is_minimized = width == 0 || height == 0;
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let texture = Self::create_offscreen_texture(&device, safe_w, safe_h, format);
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            let target = RenderTarget::Offscreen {
                texture,
                view,
                format,
            };
            Self::finish_init(
                instance,
                device,
                queue,
                target,
                format,
                safe_w,
                safe_h,
                scale_factor,
                is_minimized,
            )
        }

        fn init_with_surface(
            instance: wgpu::Instance,
            surface: wgpu::Surface<'static>,
            width: u32,
            height: u32,
            scale_factor: f32,
        ) -> Result<Self, String> {
            let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            }))
            .map_err(|e| format!("No compatible GPU adapter found for window surface: {e}"))?;

            let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(|e| format!("Failed to request wgpu device: {e}"))?;

            let caps = surface.get_capabilities(&adapter);
            let format = caps
                .formats
                .iter()
                .copied()
                .find(|f| f.is_srgb())
                .or_else(|| caps.formats.first().copied())
                .ok_or_else(|| "Surface reported no supported texture formats".to_string())?;
            let alpha_mode = caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Opaque);

            let safe_w = width.max(1);
            let safe_h = height.max(1);
            let is_minimized = width == 0 || height == 0;
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: safe_w,
                height: safe_h,
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 2,
                alpha_mode,
                view_formats: vec![],
            };
            if !is_minimized {
                surface.configure(&device, &config);
            }

            let target = RenderTarget::WindowSurface { surface, config };
            Self::finish_init(
                instance,
                device,
                queue,
                target,
                format,
                safe_w,
                safe_h,
                scale_factor,
                is_minimized,
            )
        }

        #[allow(clippy::too_many_arguments)]
        fn finish_init(
            instance: wgpu::Instance,
            device: wgpu::Device,
            queue: wgpu::Queue,
            target: RenderTarget,
            format: wgpu::TextureFormat,
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            is_minimized: bool,
        ) -> Result<Self, String> {
            let scale = sanitize_scale_factor(scale_factor);
            let globals = GlobalsUniform {
                viewport: [physical_width as f32, physical_height as f32, scale, 0.0],
            };
            let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("rustcode_globals_uniform"),
                contents: bytemuck::bytes_of(&globals),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rustcode_globals_bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
            let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rustcode_globals_bg"),
                layout: &globals_bgl,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals_buffer.as_entire_binding(),
                }],
            });

            let atlas_size = DEFAULT_ATLAS_SIZE;
            let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("rustcode_glyph_atlas"),
                size: wgpu::Extent3d {
                    width: atlas_size,
                    height: atlas_size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("rustcode_glyph_sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            });

            let atlas_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rustcode_atlas_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
            let atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rustcode_atlas_bg"),
                layout: &atlas_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&atlas_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                    },
                ],
            });

            let quad_pipeline = Self::create_quad_pipeline(&device, format, &globals_bgl);
            let shadow_pipeline = Self::create_shadow_pipeline(&device, format, &globals_bgl);
            let glyph_pipeline =
                Self::create_glyph_pipeline(&device, format, &globals_bgl, &atlas_bgl);

            Ok(Self {
                _instance: instance,
                device,
                queue,
                target,
                physical_width,
                physical_height,
                scale_factor: scale,
                is_minimized,
                globals_buffer,
                globals_bind_group,
                quad_pipeline,
                shadow_pipeline,
                glyph_pipeline,
                atlas_texture,
                atlas_bind_group,
                atlas_allocator: GlyphAtlasAllocator::new(atlas_size, atlas_size),
                shaper: TextShaper::new(),
                last_stats: RenderFrameStats::default(),
            })
        }

        fn create_offscreen_texture(
            device: &wgpu::Device,
            width: u32,
            height: u32,
            format: wgpu::TextureFormat,
        ) -> wgpu::Texture {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("rustcode_offscreen_target"),
                size: wgpu::Extent3d {
                    width: width.max(1),
                    height: height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        }

        fn create_quad_pipeline(
            device: &wgpu::Device,
            format: wgpu::TextureFormat,
            globals_bgl: &wgpu::BindGroupLayout,
        ) -> wgpu::RenderPipeline {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("rustcode_quad_shader"),
                source: wgpu::ShaderSource::Wgsl(QUAD_WGSL.into()),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("rustcode_quad_layout"),
                bind_group_layouts: &[globals_bgl],
                push_constant_ranges: &[],
            });
            let attrs = wgpu::vertex_attr_array![
                0 => Float32x4,
                1 => Float32x4,
                2 => Float32x4,
                3 => Float32x4,
                4 => Float32x4
            ];
            Self::build_instanced_pipeline(
                device,
                "rustcode_quad_pipeline",
                &layout,
                &shader,
                format,
                std::mem::size_of::<QuadInstance>() as u64,
                &attrs,
            )
        }

        fn create_shadow_pipeline(
            device: &wgpu::Device,
            format: wgpu::TextureFormat,
            globals_bgl: &wgpu::BindGroupLayout,
        ) -> wgpu::RenderPipeline {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("rustcode_shadow_shader"),
                source: wgpu::ShaderSource::Wgsl(SHADOW_WGSL.into()),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("rustcode_shadow_layout"),
                bind_group_layouts: &[globals_bgl],
                push_constant_ranges: &[],
            });
            let attrs = wgpu::vertex_attr_array![
                0 => Float32x4,
                1 => Float32x4,
                2 => Float32x4,
                3 => Float32x4
            ];
            Self::build_instanced_pipeline(
                device,
                "rustcode_shadow_pipeline",
                &layout,
                &shader,
                format,
                std::mem::size_of::<ShadowInstance>() as u64,
                &attrs,
            )
        }

        fn create_glyph_pipeline(
            device: &wgpu::Device,
            format: wgpu::TextureFormat,
            globals_bgl: &wgpu::BindGroupLayout,
            atlas_bgl: &wgpu::BindGroupLayout,
        ) -> wgpu::RenderPipeline {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("rustcode_glyph_shader"),
                source: wgpu::ShaderSource::Wgsl(GLYPH_WGSL.into()),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("rustcode_glyph_layout"),
                bind_group_layouts: &[globals_bgl, atlas_bgl],
                push_constant_ranges: &[],
            });
            let attrs = wgpu::vertex_attr_array![
                0 => Float32x4,
                1 => Float32x4,
                2 => Float32x4,
                3 => Float32x4
            ];
            Self::build_instanced_pipeline(
                device,
                "rustcode_glyph_pipeline",
                &layout,
                &shader,
                format,
                std::mem::size_of::<GlyphInstance>() as u64,
                &attrs,
            )
        }

        fn build_instanced_pipeline(
            device: &wgpu::Device,
            label: &str,
            layout: &wgpu::PipelineLayout,
            shader: &wgpu::ShaderModule,
            format: wgpu::TextureFormat,
            stride: u64,
            attributes: &[wgpu::VertexAttribute],
        ) -> wgpu::RenderPipeline {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes,
                    }],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            })
        }

        /// Load custom font bytes (e.g., Codicons `.ttf` or bundled monospace font) into `FontSystem`.
        pub fn load_font_bytes(&mut self, data: Vec<u8>) {
            self.shaper.load_font_bytes(data);
        }

        /// Shape a `GlyphRun` using `cosmic-text` at the current surface scale factor.
        pub fn shape_glyph_run(&mut self, run: &GlyphRun) -> ShapedGlyphRun {
            self.shaper.shape_run(run, self.scale_factor)
        }

        pub fn physical_size(&self) -> (u32, u32) {
            (self.physical_width, self.physical_height)
        }

        pub fn scale_factor(&self) -> f32 {
            self.scale_factor
        }

        pub fn is_minimized(&self) -> bool {
            self.is_minimized
        }

        pub fn last_frame_stats(&self) -> &RenderFrameStats {
            &self.last_stats
        }

        pub fn atlas_stats(&self) -> (usize, u64, u64, u64) {
            (
                self.atlas_allocator.len(),
                self.atlas_allocator.hit_count(),
                self.atlas_allocator.miss_count(),
                self.atlas_allocator.reset_count(),
            )
        }

        /// Resize the surface safely, guarding `0x0` minimized dimensions so `wgpu::SurfaceConfiguration` never panics.
        pub fn resize_surface(&mut self, width: u32, height: u32, scale_factor: f32) {
            self.scale_factor = sanitize_scale_factor(scale_factor);
            if width == 0 || height == 0 {
                self.is_minimized = true;
                return;
            }
            self.is_minimized = false;
            self.physical_width = width;
            self.physical_height = height;

            let globals = GlobalsUniform {
                viewport: [
                    self.physical_width as f32,
                    self.physical_height as f32,
                    self.scale_factor,
                    0.0,
                ],
            };
            self.queue
                .write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

            match &mut self.target {
                RenderTarget::WindowSurface { surface, config } => {
                    config.width = width;
                    config.height = height;
                    surface.configure(&self.device, config);
                }
                RenderTarget::Offscreen {
                    texture,
                    view,
                    format,
                } => {
                    *texture = Self::create_offscreen_texture(&self.device, width, height, *format);
                    *view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                }
            }
        }

        fn upload_glyph_to_atlas(&self, rect: GlyphAtlasRect, rgba_bytes: &[u8]) {
            if rect.width == 0 || rect.height == 0 {
                return;
            }
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: rect.x,
                        y: rect.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                rgba_bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(rect.width * 4),
                    rows_per_image: Some(rect.height),
                },
                wgpu::Extent3d {
                    width: rect.width,
                    height: rect.height,
                    depth_or_array_layers: 1,
                },
            );
        }

        fn clear_atlas_texture(&self) {
            let (aw, ah) = self.atlas_allocator.dimensions();
            let zeros = vec![0u8; (aw * ah * 4) as usize];
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &zeros,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(aw * 4),
                    rows_per_image: Some(ah),
                },
                wgpu::Extent3d {
                    width: aw,
                    height: ah,
                    depth_or_array_layers: 1,
                },
            );
        }

        fn build_scene_batches(&mut self, scene: &Scene) -> SceneBatchBuilder {
            let logical_size = self.size();
            let viewport = Rect::from_xywh(0.0, 0.0, logical_size.width, logical_size.height);
            let mut builder = SceneBatchBuilder::new();
            let mut clips = ClipStack::new(viewport);
            self.atlas_allocator.begin_frame();
            let scale = self.scale_factor;
            let fb_w = self.physical_width;
            let fb_h = self.physical_height;
            let (atlas_w, atlas_h) = self.atlas_allocator.dimensions();

            for node in scene.nodes() {
                match node {
                    SceneNode::PushClip(rect) => clips.push(*rect),
                    SceneNode::PopClip => clips.pop(),
                    SceneNode::Quad(quad) => {
                        let scissor = clips.scissor_rect(scale, fb_w, fb_h);
                        if scissor.is_none() {
                            builder.stats.skipped_clipped += 1;
                            continue;
                        }
                        if let Some(inst) = QuadInstance::from_quad(quad, scale) {
                            if inst.is_visible() {
                                builder.push_quad(inst, scissor);
                            }
                        }
                    }
                    SceneNode::Shadow(shadow) => {
                        let scissor = clips.scissor_rect(scale, fb_w, fb_h);
                        if scissor.is_none() {
                            builder.stats.skipped_clipped += 1;
                            continue;
                        }
                        if let Some(inst) = ShadowInstance::from_shadow(shadow, scale) {
                            builder.push_shadow(inst, scissor);
                        }
                    }
                    SceneNode::GlyphRun(run) => {
                        let scissor = clips.scissor_rect(scale, fb_w, fb_h);
                        if scissor.is_none() {
                            builder.stats.skipped_clipped += 1;
                            continue;
                        }
                        if !TextShaper::is_valid_glyph_run(run, scale) {
                            continue;
                        }

                        let font_size = run.font_size;
                        let capped_text = TextShaper::truncate_run_text(&run.text);
                        let line_height = (font_size * 1.5).round().max(1.0);
                        self.shaper
                            .scratch_buffer
                            .set_metrics(&mut self.shaper.font_system, Metrics::new(font_size, line_height));
                        self.shaper
                            .scratch_buffer
                            .set_size(&mut self.shaper.font_system, None, None);
                        let attrs = Attrs::new()
                            .family(TextShaper::map_family(&run.font_family))
                            .weight(if run.bold { Weight::BOLD } else { Weight::NORMAL })
                            .style(if run.italic { Style::Italic } else { Style::Normal });
                        self.shaper.scratch_buffer.set_text(
                            &mut self.shaper.font_system,
                            capped_text,
                            &attrs,
                            Shaping::Advanced,
                        );
                        self.shaper
                            .scratch_buffer
                            .shape_until_scroll(&mut self.shaper.font_system, false);

                        let mut max_line_w = 0.0_f32;
                        let mut last_line_y = font_size * 0.8;
                        let mut pending_glyphs: Vec<GlyphInstance> = Vec::new();
                        let mut reset_occurred = false;

                        for layout_run in self.shaper.scratch_buffer.layout_runs() {
                            max_line_w = max_line_w.max(layout_run.line_w);
                            last_line_y = layout_run.line_y;
                            let run_y = run.origin.y + layout_run.line_y;
                            if !run_y.is_finite()
                                || (run_y * scale).abs() > TextShaper::MAX_SAFE_TEXT_ORIGIN_PX
                            {
                                continue;
                            }
                            for g in layout_run.glyphs.iter() {
                                let g_px = (run.origin.x + g.x) * scale;
                                let g_py = (run_y + g.y) * scale;
                                if !g_px.is_finite()
                                    || !g_py.is_finite()
                                    || g_px.abs() > TextShaper::MAX_SAFE_TEXT_ORIGIN_PX
                                    || g_py.abs() > TextShaper::MAX_SAFE_TEXT_ORIGIN_PX
                                {
                                    continue;
                                }
                                let phys = g.physical((run.origin.x, run_y), scale);
                                let key = glyph_cache_key_from_cosmic(phys.cache_key);
                                let rect_opt = if let Some(hit) = self.atlas_allocator.get(&key) {
                                    Some(hit)
                                } else {
                                    let img_opt = self
                                        .shaper
                                        .swash_cache
                                        .get_image(&mut self.shaper.font_system, phys.cache_key)
                                        .as_ref()
                                        .cloned();
                                    let Some(img) = img_opt else {
                                        continue;
                                    };
                                    let gw = img.placement.width;
                                    let gh = img.placement.height;
                                    let (rgba, is_color) =
                                        convert_swash_image_to_rgba8(img.content, &img.data);
                                    match self.atlas_allocator.insert(
                                        key,
                                        gw,
                                        gh,
                                        img.placement.left,
                                        img.placement.top,
                                        is_color,
                                    ) {
                                        AtlasInsertResult::Hit(r)
                                        | AtlasInsertResult::Allocated(r) => {
                                            self.upload_glyph_to_atlas(r, &rgba);
                                            Some(r)
                                        }
                                        AtlasInsertResult::AllocatedAfterReset(r) => {
                                            reset_occurred = true;
                                            self.clear_atlas_texture();
                                            self.upload_glyph_to_atlas(r, &rgba);
                                            Some(r)
                                        }
                                        AtlasInsertResult::TooLarge => None,
                                    }
                                };

                                if let Some(rect) = rect_opt {
                                    if rect.width > 0 && rect.height > 0 {
                                        let gx = phys.x as f32 + rect.left as f32;
                                        let gy = phys.y as f32 - rect.top as f32;
                                        pending_glyphs.push(GlyphInstance {
                                            pos_size: [
                                                gx,
                                                gy,
                                                rect.width as f32,
                                                rect.height as f32,
                                            ],
                                            uv_rect: rect.uv_rect(atlas_w, atlas_h),
                                            color: [run.color.r, run.color.g, run.color.b, run.color.a],
                                            mode: [if rect.is_color { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
                                        });
                                    }
                                }
                            }
                        }

                        if reset_occurred {
                            builder.split_glyph_batch();
                        }
                        for inst in pending_glyphs {
                            builder.push_glyph(inst, scissor);
                        }

                        if run.underline && max_line_w > 0.0 {
                            let uw = max_line_w * scale;
                            let uh = (1.0 * scale).max(1.0);
                            let ux = run.origin.x * scale;
                            let uy = (run.origin.y + last_line_y + 2.0) * scale;
                            if ux.is_finite()
                                && uy.is_finite()
                                && uw.is_finite()
                                && uh.is_finite()
                                && uw > 0.0
                                && uh > 0.0
                            {
                                builder.push_quad(
                                    QuadInstance {
                                        bounds: [ux, uy, uw, uh],
                                        bg_color: [run.color.r, run.color.g, run.color.b, run.color.a],
                                        border_color: [0.0; 4],
                                        border_widths: [0.0; 4],
                                        corner_radii: [0.0; 4],
                                    },
                                    scissor,
                                );
                            }
                        }
                    }
                }
            }

            builder.stats.atlas_resets = self.atlas_allocator.reset_count();
            builder
        }
    }

    impl RenderSurface for WgpuRenderSurface {
        fn render_scene(&mut self, scene: &Scene) {
            if self.is_minimized || self.physical_width == 0 || self.physical_height == 0 {
                return;
            }

            let globals = GlobalsUniform {
                viewport: [
                    self.physical_width as f32,
                    self.physical_height as f32,
                    self.scale_factor,
                    0.0,
                ],
            };
            self.queue
                .write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

            let mut batches = self.build_scene_batches(scene);

            // Upload all instance buffers up front so their lifetimes outlive the RenderPass.
            enum UploadedBatch {
                Scissor([u32; 4]),
                Quads { buf: wgpu::Buffer, count: u32 },
                Shadows { buf: wgpu::Buffer, count: u32 },
                Glyphs { buf: wgpu::Buffer, count: u32 },
            }

            let fb_w = self.physical_width;
            let fb_h = self.physical_height;
            let mut uploaded = Vec::with_capacity(batches.commands.len());
            for cmd in batches.commands.drain(..) {
                match cmd {
                    DrawCommand::SetScissor(Some(s)) => {
                        uploaded.push(UploadedBatch::Scissor(s));
                    }
                    DrawCommand::SetScissor(None) => {
                        uploaded.push(UploadedBatch::Scissor([0, 0, fb_w, fb_h]));
                    }
                    DrawCommand::DrawQuads(instances) if !instances.is_empty() => {
                        let buf =
                            self.device
                                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("rustcode_quad_instances"),
                                    contents: bytemuck::cast_slice(&instances),
                                    usage: wgpu::BufferUsages::VERTEX,
                                });
                        uploaded.push(UploadedBatch::Quads {
                            buf,
                            count: instances.len() as u32,
                        });
                    }
                    DrawCommand::DrawShadows(instances) if !instances.is_empty() => {
                        let buf =
                            self.device
                                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("rustcode_shadow_instances"),
                                    contents: bytemuck::cast_slice(&instances),
                                    usage: wgpu::BufferUsages::VERTEX,
                                });
                        uploaded.push(UploadedBatch::Shadows {
                            buf,
                            count: instances.len() as u32,
                        });
                    }
                    DrawCommand::DrawGlyphs(instances) if !instances.is_empty() => {
                        let buf =
                            self.device
                                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("rustcode_glyph_instances"),
                                    contents: bytemuck::cast_slice(&instances),
                                    usage: wgpu::BufferUsages::VERTEX,
                                });
                        uploaded.push(UploadedBatch::Glyphs {
                            buf,
                            count: instances.len() as u32,
                        });
                    }
                    _ => {}
                }
            }

            let surface_frame = match &self.target {
                RenderTarget::WindowSurface { surface, config } => {
                    match surface.get_current_texture() {
                        Ok(frame) => Some(frame),
                        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                            surface.configure(&self.device, config);
                            return;
                        }
                        Err(_) => return,
                    }
                }
                RenderTarget::Offscreen { .. } => None,
            };

            let target_view_owned;
            let target_view: &wgpu::TextureView = match (&self.target, &surface_frame) {
                (RenderTarget::WindowSurface { .. }, Some(frame)) => {
                    target_view_owned = frame
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    &target_view_owned
                }
                (RenderTarget::Offscreen { view, .. }, _) => view,
                _ => return,
            };

            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("rustcode_render_encoder"),
                });

            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("rustcode_main_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.117,
                                g: 0.117,
                                b: 0.117,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

                pass.set_scissor_rect(0, 0, fb_w, fb_h);
                for item in &uploaded {
                    match item {
                        UploadedBatch::Scissor([x, y, w, h]) => {
                            if *w > 0 && *h > 0 && x + w <= fb_w && y + h <= fb_h {
                                pass.set_scissor_rect(*x, *y, *w, *h);
                            }
                        }
                        UploadedBatch::Quads { buf, count } => {
                            pass.set_pipeline(&self.quad_pipeline);
                            pass.set_bind_group(0, &self.globals_bind_group, &[]);
                            pass.set_vertex_buffer(0, buf.slice(..));
                            pass.draw(0..6, 0..*count);
                        }
                        UploadedBatch::Shadows { buf, count } => {
                            pass.set_pipeline(&self.shadow_pipeline);
                            pass.set_bind_group(0, &self.globals_bind_group, &[]);
                            pass.set_vertex_buffer(0, buf.slice(..));
                            pass.draw(0..6, 0..*count);
                        }
                        UploadedBatch::Glyphs { buf, count } => {
                            pass.set_pipeline(&self.glyph_pipeline);
                            pass.set_bind_group(0, &self.globals_bind_group, &[]);
                            pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                            pass.set_vertex_buffer(0, buf.slice(..));
                            pass.draw(0..6, 0..*count);
                        }
                    }
                }
            }

            self.queue.submit(std::iter::once(encoder.finish()));
            if let Some(frame) = surface_frame {
                frame.present();
            }
            self.last_stats = batches.stats;
        }

        fn size(&self) -> Size {
            Size::new(
                self.physical_width as f32 / self.scale_factor,
                self.physical_height as f32 / self.scale_factor,
            )
        }

        fn resize(&mut self, width: u32, height: u32, scale_factor: f32) {
            self.resize_surface(width, height, scale_factor);
        }
    }
}

#[cfg(feature = "desktop")]
pub use desktop_impl::{ShapedGlyphInfo, ShapedGlyphRun, TextShaper, WgpuRenderSurface};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::color::Rgba;
    use crate::render::geometry::{Edges, Point};

    #[test]
    fn test_corner_radius_clamping_to_half_min_dim() {
        let clamped = clamp_corner_radii(Corners::new(100.0, 20.0, -5.0, f32::NAN), 40.0, 24.0);
        // min(40, 24) / 2 = 12.0
        assert_eq!(clamped, [12.0, 12.0, 0.0, 0.0]);
        assert_eq!(clamp_corner_radii(Corners::all(10.0), 0.0, 20.0), [0.0; 4]);
    }

    #[test]
    fn test_quad_instance_asymmetric_borders_and_clamping() {
        let quad = Quad::new(Rect::from_xywh(10.0, 20.0, 100.0, 30.0))
            .with_background(Rgba::new(0.2, 0.4, 0.6, 0.8))
            .with_border_edges(Rgba::white(), Edges::new(1.0, 2.0, 3.0, 4.0))
            .with_corner_radius(50.0);

        let inst = QuadInstance::from_quad(&quad, 2.0).expect("valid quad");
        assert_eq!(inst.bounds, [20.0, 40.0, 200.0, 60.0]);
        assert_eq!(inst.border_widths, [2.0, 4.0, 6.0, 8.0]);
        // min(200, 60) / 2 = 30.0
        assert_eq!(inst.corner_radii, [30.0, 30.0, 30.0, 30.0]);
        assert!(inst.is_visible());

        // Zero-area or NaN quads return None
        assert!(QuadInstance::from_quad(&Quad::new(Rect::from_xywh(0.0, 0.0, 0.0, 10.0)), 1.0).is_none());
        assert!(QuadInstance::from_quad(&Quad::new(Rect::from_xywh(f32::NAN, 0.0, 10.0, 10.0)), 1.0).is_none());
    }

    #[test]
    fn test_shadow_instance_zero_blur_and_negative_spread() {
        let base = Shadow {
            bounds: Rect::from_xywh(10.0, 20.0, 100.0, 40.0),
            offset: Point::new(2.0, 4.0),
            blur: 0.0,
            spread: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 0.5),
        };
        let zero_blur = ShadowInstance::from_shadow(&base, 1.0).expect("zero blur valid");
        assert_eq!(zero_blur.params[0], 0.0);
        assert_eq!(zero_blur.shadow_rect, [12.0, 24.0, 100.0, 40.0]);

        // Small negative spread shrinks core box safely
        let neg_spread = Shadow {
            spread: -5.0,
            ..base.clone()
        };
        let shrunk = ShadowInstance::from_shadow(&neg_spread, 1.0).expect("partial negative spread");
        assert_eq!(shrunk.shadow_rect, [17.0, 29.0, 90.0, 30.0]);

        // Excessive negative spread collapses to zero area -> None (never panics)
        let collapsed = Shadow {
            spread: -25.0,
            ..base.clone()
        };
        assert!(ShadowInstance::from_shadow(&collapsed, 1.0).is_none());

        // Overflowing blur/spread/offset/scale rejected cleanly instead of emitting +/-inf bounds
        let huge_blur = Shadow {
            blur: f32::MAX,
            ..base.clone()
        };
        assert!(ShadowInstance::from_shadow(&huge_blur, 2.0).is_none());

        let huge_spread = Shadow {
            spread: f32::MAX,
            ..base.clone()
        };
        assert!(ShadowInstance::from_shadow(&huge_spread, 1.0).is_none());

        let huge_scale = ShadowInstance::from_shadow(&base, f32::MAX);
        assert!(huge_scale.is_none());
    }

    #[test]
    fn test_clip_stack_nested_intersection_and_unbalanced_pop() {
        let viewport = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let mut stack = ClipStack::new(viewport);

        // Unbalanced pop on empty stack is a safe no-op
        stack.pop();
        assert_eq!(stack.depth(), 0);
        assert_eq!(stack.current(), viewport);

        // First clip
        stack.push(Rect::from_xywh(100.0, 50.0, 400.0, 300.0));
        assert_eq!(stack.current(), Rect::from_xywh(100.0, 50.0, 400.0, 300.0));

        // Nested clip intersects, never expands beyond parent
        stack.push(Rect::from_xywh(50.0, 100.0, 200.0, 400.0));
        assert_eq!(stack.current(), Rect::from_xywh(100.0, 100.0, 150.0, 250.0));
        assert_eq!(stack.scissor_rect(2.0, 1600, 1200), Some([200, 200, 300, 500]));

        // Disjoint nested clip produces empty scissor
        stack.push(Rect::from_xywh(700.0, 500.0, 50.0, 50.0));
        assert!(stack.is_empty_clip());
        assert_eq!(stack.scissor_rect(1.0, 800, 600), None);

        stack.pop();
        assert_eq!(stack.current(), Rect::from_xywh(100.0, 100.0, 150.0, 250.0));
        stack.pop();
        assert_eq!(stack.current(), Rect::from_xywh(100.0, 50.0, 400.0, 300.0));
        stack.pop();
        assert_eq!(stack.current(), viewport);
    }

    #[test]
    fn test_scene_batch_builder_clips_and_unclosed_push_clip() {
        let mut scene = Scene::new();
        scene.push_clip(Rect::from_xywh(10.0, 10.0, 100.0, 100.0));
        scene.push_quad(Quad::new(Rect::from_xywh(20.0, 20.0, 50.0, 50.0)).with_background(Rgba::white()));
        // Disjoint nested clip skips the quad
        scene.push_clip(Rect::from_xywh(500.0, 500.0, 50.0, 50.0));
        scene.push_quad(Quad::new(Rect::from_xywh(510.0, 510.0, 10.0, 10.0)).with_background(Rgba::black()));
        // Intentionally unclosed PushClip + extra PopClip safety
        scene.pop_clip();
        scene.pop_clip();
        scene.pop_clip();

        let batch = SceneBatchBuilder::build_non_text(
            &scene,
            Rect::from_xywh(0.0, 0.0, 800.0, 600.0),
            1.5,
            1200,
            900,
        );
        assert_eq!(batch.stats.quad_count, 1);
        assert_eq!(batch.stats.skipped_clipped, 1);
    }

    #[test]
    fn test_glyph_atlas_allocator_hit_and_overflow_reset() {
        let mut atlas = GlyphAtlasAllocator::new(64, 64);
        let key1 = GlyphCacheKey {
            font_id: 1,
            glyph_id: 65,
            font_size_bits: 14.0_f32.to_bits(),
            x_bin: 0,
            y_bin: 0,
            flags: 0,
        };

        atlas.begin_frame();
        match atlas.insert(key1, 16, 16, 1, 12, false) {
            AtlasInsertResult::Allocated(r) => {
                assert_eq!((r.x, r.y, r.width, r.height), (1, 1, 16, 16));
            }
            other => panic!("expected Allocated, got {other:?}"),
        }

        // Frame 2: cache hit reuse
        atlas.begin_frame();
        match atlas.insert(key1, 16, 16, 1, 12, false) {
            AtlasInsertResult::Hit(r) => assert_eq!(r.last_used_frame, 2),
            other => panic!("expected Hit, got {other:?}"),
        }
        assert_eq!(atlas.hit_count(), 1);

        // Overflow small 64x64 atlas with many unique 20x20 glyphs
        let mut saw_reset = false;
        for gid in 100..120u16 {
            let k = GlyphCacheKey {
                glyph_id: gid,
                ..key1
            };
            if matches!(
                atlas.insert(k, 20, 20, 0, 15, false),
                AtlasInsertResult::AllocatedAfterReset(_)
            ) {
                saw_reset = true;
            }
        }
        assert!(saw_reset, "expected atlas overflow reset");
        assert!(atlas.reset_count() >= 1);

        // Oversized glyph returns TooLarge without panicking
        let huge_key = GlyphCacheKey {
            glyph_id: 999,
            ..key1
        };
        assert_eq!(
            atlas.insert(huge_key, 128, 128, 0, 0, false),
            AtlasInsertResult::TooLarge
        );

        // u32::MAX / u32::MAX - 1 dimensions return TooLarge without integer overflow panic
        let max_key = GlyphCacheKey {
            glyph_id: 1000,
            ..key1
        };
        assert_eq!(
            atlas.insert(max_key, u32::MAX, 10, 0, 0, false),
            AtlasInsertResult::TooLarge
        );
        let max_m1_key = GlyphCacheKey {
            glyph_id: 1001,
            ..key1
        };
        assert_eq!(
            atlas.insert(max_m1_key, 10, u32::MAX - 1, 0, 0, false),
            AtlasInsertResult::TooLarge
        );
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn test_cosmic_text_shaping_ligatures_cjk_emoji_and_rtl() {
        let mut shaper = TextShaper::new();

        // 1. Monospace & programming ligatures
        let mono_run = GlyphRun::new(
            "fn main() -> bool { a !== b && c === d => e }".to_string(),
            Point::new(0.0, 0.0),
            Rect::from_xywh(0.0, 0.0, 500.0, 24.0),
            Rgba::white(),
            14.0,
        )
        .with_font_family("monospace");
        let shaped_mono = shaper.shape_run(&mono_run, 1.0);
        assert!(!shaped_mono.glyphs.is_empty());
        assert!(shaped_mono.total_advance_width > 0.0);

        // 2. CJK + combining mark + RTL + emoji fallback
        let intl_run = GlyphRun::new(
            "你好世界 日本語 e\u{0301} שלום 🚀 👨‍👩‍👧‍👦".to_string(),
            Point::new(0.0, 0.0),
            Rect::from_xywh(0.0, 0.0, 600.0, 24.0),
            Rgba::white(),
            16.0,
        );
        let shaped_intl = shaper.shape_run(&intl_run, 1.5);
        assert!(!shaped_intl.glyphs.is_empty());
        assert!(shaped_intl.total_advance_width > 0.0);

        // 3. Adversarial origins, invalid font sizes, and huge single-line runs must not panic or hang
        for bad_origin in [
            Point::new(f32::NAN, 0.0),
            Point::new(0.0, f32::INFINITY),
            Point::new(1.0e20, 0.0),
            Point::new(1.0e12, 0.0),
        ] {
            let bad_run = GlyphRun::new(
                "hello".to_string(),
                bad_origin,
                Rect::from_xywh(0.0, 0.0, 200.0, 24.0),
                Rgba::white(),
                14.0,
            );
            let shaped = shaper.shape_run(&bad_run, 1.5);
            assert!(shaped.glyphs.is_empty());
        }

        for bad_font_size in [0.0, -4.0, f32::NAN, f32::INFINITY, 1.0e9] {
            let bad_run = GlyphRun::new(
                "hello".to_string(),
                Point::new(0.0, 0.0),
                Rect::from_xywh(0.0, 0.0, 200.0, 24.0),
                Rgba::white(),
                bad_font_size,
            );
            let shaped = shaper.shape_run(&bad_run, 1.0);
            assert!(shaped.glyphs.is_empty());
        }

        let long_run = GlyphRun::new(
            "a".repeat(100_000),
            Point::new(0.0, 0.0),
            Rect::from_xywh(0.0, 0.0, 1000.0, 24.0),
            Rgba::white(),
            14.0,
        );
        let shaped_long = shaper.shape_run(&long_run, 1.0);
        assert!(shaped_long.glyphs.len() <= 16_384);
        assert!(shaped_long.text.chars().count() <= 16_384);
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn test_wgpu_headless_surface_optional_adapter_smoke() {
        match WgpuRenderSurface::new_headless(800, 600, 2.0) {
            Ok(mut surface) => {
                // Logical size = physical / scale_factor
                assert_eq!(surface.size(), Size::new(400.0, 300.0));

                let mut scene = Scene::new();
                scene.push_shadow(Shadow {
                    bounds: Rect::from_xywh(20.0, 20.0, 120.0, 60.0),
                    offset: Point::new(0.0, 4.0),
                    blur: 8.0,
                    spread: 0.0,
                    color: Rgba::new(0.0, 0.0, 0.0, 0.4),
                });
                scene.push_clip(Rect::from_xywh(10.0, 10.0, 300.0, 200.0));
                scene.push_quad(
                    Quad::new(Rect::from_xywh(20.0, 20.0, 120.0, 60.0))
                        .with_background(Rgba::new(0.15, 0.15, 0.18, 1.0))
                        .with_border_edges(Rgba::white(), Edges::new(1.0, 1.0, 2.0, 1.0))
                        .with_corner_radius(8.0),
                );
                scene.push_text(
                    GlyphRun::new(
                        "RustCode GPU -> OK 🚀".to_string(),
                        Point::new(28.0, 32.0),
                        Rect::from_xywh(28.0, 32.0, 200.0, 24.0),
                        Rgba::white(),
                        14.0,
                    )
                    .with_style(false, false, true),
                );
                scene.pop_clip();

                surface.render_scene(&scene);
                assert!(surface.last_frame_stats().quad_count >= 1);
                assert_eq!(surface.last_frame_stats().shadow_count, 1);

                // Guard 0x0 minimized resize without panicking
                surface.resize(0, 0, 2.0);
                assert!(surface.is_minimized());
                surface.render_scene(&scene);

                // Restore non-zero resize
                surface.resize(1280, 720, 1.25);
                assert!(!surface.is_minimized());
                assert_eq!(surface.size(), Size::new(1024.0, 576.0));
            }
            Err(_) => {
                // Gracefully skip when running in a headless CI environment without any GPU/WARP adapter.
            }
        }
    }
}
