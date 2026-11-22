//! Monospace font metrics and layout helpers.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontMetrics {
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub char_width: f32,
    pub ascent: f32,
    pub descent: f32,
}

impl Default for FontMetrics {
    fn default() -> Self {
        Self::default_monospace()
    }
}

impl FontMetrics {
    pub fn new(font_family: &str, font_size: f32) -> Self {
        let char_width = (font_size * 0.6).round();
        let line_height = (font_size * 1.5).round();
        let ascent = (font_size * 0.8).round();
        let descent = line_height - ascent;

        Self {
            font_family: font_family.to_string(),
            font_size,
            line_height,
            char_width,
            ascent,
            descent,
        }
    }

    pub fn default_monospace() -> Self {
        Self::new("monospace", 14.0)
    }

    pub fn with_line_height(mut self, line_height: f32) -> Self {
        self.line_height = line_height;
        self.descent = self.line_height - self.ascent;
        self
    }

    /// Measure text width in monospace pixels based on character count.
    pub fn text_width(&self, text: &str) -> f32 {
        text.chars().count() as f32 * self.char_width
    }

    pub fn is_finite(&self) -> bool {
        const MAX_FONT_METRIC: f32 = 10_000.0;
        self.line_height.is_finite()
            && self.line_height > 0.0
            && self.line_height <= MAX_FONT_METRIC
            && self.char_width.is_finite()
            && self.char_width >= 0.0
            && self.char_width <= MAX_FONT_METRIC
            && self.font_size.is_finite()
            && self.font_size >= 0.0
            && self.font_size <= MAX_FONT_METRIC
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_font_metrics_calculations() {
        let metrics = FontMetrics::new("JetBrains Mono", 14.0);
        assert_eq!(metrics.font_size, 14.0);
        assert_eq!(metrics.char_width, 8.0);
        assert_eq!(metrics.line_height, 21.0);
        assert_eq!(metrics.ascent, 11.0);
        assert_eq!(metrics.descent, 10.0);
        assert!(metrics.is_finite());
    }

    #[test]
    fn test_font_metrics_is_finite_validation() {
        let base = FontMetrics::default_monospace();
        assert!(base.is_finite());
        assert!(!(FontMetrics { line_height: 0.0, ..base.clone() }).is_finite());
        assert!(!(FontMetrics { line_height: f32::NAN, ..base.clone() }).is_finite());
        assert!(!(FontMetrics { char_width: -1.0, ..base.clone() }).is_finite());
        assert!(!(FontMetrics { font_size: f32::INFINITY, ..base.clone() }).is_finite());
        assert!(!(FontMetrics { font_size: 20_000.0, ..base }).is_finite());
    }
}
