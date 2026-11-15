//! Color representations (RGBA and HSLA) matching GPUI style systems.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            r: r.clamp(0.0, 1.0),
            g: g.clamp(0.0, 1.0),
            b: b.clamp(0.0, 1.0),
            a: a.clamp(0.0, 1.0),
        }
    }

    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::new(r, g, b, 1.0)
    }

    pub fn transparent() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0)
    }

    pub fn black() -> Self {
        Self::new(0.0, 0.0, 0.0, 1.0)
    }

    pub fn white() -> Self {
        Self::new(1.0, 1.0, 1.0, 1.0)
    }

    pub fn with_alpha(&self, alpha: f32) -> Self {
        Self::new(self.r, self.g, self.b, alpha)
    }

    /// Source-over alpha compositing (`self` over `dst`).
    pub fn blend(&self, dst: &Rgba) -> Self {
        let out_a = self.a + dst.a * (1.0 - self.a);
        if out_a <= 0.0 {
            return Self::transparent();
        }
        let out_r = (self.r * self.a + dst.r * dst.a * (1.0 - self.a)) / out_a;
        let out_g = (self.g * self.a + dst.g * dst.a * (1.0 - self.a)) / out_a;
        let out_b = (self.b * self.a + dst.b * dst.a * (1.0 - self.a)) / out_a;
        Self::new(out_r, out_g, out_b, out_a)
    }

    /// Parse hex color string `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let clean = hex.trim().trim_start_matches('#');
        match clean.len() {
            3 => {
                let r = u8::from_str_radix(&clean[0..1].repeat(2), 16).ok()? as f32 / 255.0;
                let g = u8::from_str_radix(&clean[1..2].repeat(2), 16).ok()? as f32 / 255.0;
                let b = u8::from_str_radix(&clean[2..3].repeat(2), 16).ok()? as f32 / 255.0;
                Some(Self::new(r, g, b, 1.0))
            }
            4 => {
                let r = u8::from_str_radix(&clean[0..1].repeat(2), 16).ok()? as f32 / 255.0;
                let g = u8::from_str_radix(&clean[1..2].repeat(2), 16).ok()? as f32 / 255.0;
                let b = u8::from_str_radix(&clean[2..3].repeat(2), 16).ok()? as f32 / 255.0;
                let a = u8::from_str_radix(&clean[3..4].repeat(2), 16).ok()? as f32 / 255.0;
                Some(Self::new(r, g, b, a))
            }
            6 => {
                let r = u8::from_str_radix(&clean[0..2], 16).ok()? as f32 / 255.0;
                let g = u8::from_str_radix(&clean[2..4], 16).ok()? as f32 / 255.0;
                let b = u8::from_str_radix(&clean[4..6], 16).ok()? as f32 / 255.0;
                Some(Self::new(r, g, b, 1.0))
            }
            8 => {
                let r = u8::from_str_radix(&clean[0..2], 16).ok()? as f32 / 255.0;
                let g = u8::from_str_radix(&clean[2..4], 16).ok()? as f32 / 255.0;
                let b = u8::from_str_radix(&clean[4..6], 16).ok()? as f32 / 255.0;
                let a = u8::from_str_radix(&clean[6..8], 16).ok()? as f32 / 255.0;
                Some(Self::new(r, g, b, a))
            }
            _ => None,
        }
    }

    pub fn to_hex(&self) -> String {
        let r = (self.r * 255.0).round() as u8;
        let g = (self.g * 255.0).round() as u8;
        let b = (self.b * 255.0).round() as u8;
        let a = (self.a * 255.0).round() as u8;
        if a == 255 {
            format!("#{:02x}{:02x}{:02x}", r, g, b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", r, g, b, a)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Hsla {
    pub h: f32, // 0.0 .. 360.0
    pub s: f32, // 0.0 .. 1.0
    pub l: f32, // 0.0 .. 1.0
    pub a: f32, // 0.0 .. 1.0
}

impl Hsla {
    pub fn new(h: f32, s: f32, l: f32, a: f32) -> Self {
        Self {
            h: h.rem_euclid(360.0),
            s: s.clamp(0.0, 1.0),
            l: l.clamp(0.0, 1.0),
            a: a.clamp(0.0, 1.0),
        }
    }

    pub fn to_rgba(&self) -> Rgba {
        let c = (1.0 - (2.0 * self.l - 1.0).abs()) * self.s;
        let h_prime = self.h / 60.0;
        let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
        let (r1, g1, b1) = if h_prime < 1.0 {
            (c, x, 0.0)
        } else if h_prime < 2.0 {
            (x, c, 0.0)
        } else if h_prime < 3.0 {
            (0.0, c, x)
        } else if h_prime < 4.0 {
            (0.0, x, c)
        } else if h_prime < 5.0 {
            (x, 0.0, c)
        } else {
            (c, 0.0, x)
        };
        let m = self.l - c / 2.0;
        Rgba::new(r1 + m, g1 + m, b1 + m, self.a)
    }

    pub fn from_rgba(rgba: Rgba) -> Self {
        let max = rgba.r.max(rgba.g.max(rgba.b));
        let min = rgba.r.min(rgba.g.min(rgba.b));
        let delta = max - min;
        let l = (max + min) / 2.0;

        if delta == 0.0 {
            return Self::new(0.0, 0.0, l, rgba.a);
        }

        let s = if l <= 0.5 {
            delta / (max + min)
        } else {
            delta / (2.0 - max - min)
        };

        let mut h = if rgba.r == max {
            ((rgba.g - rgba.b) / delta) % 6.0
        } else if rgba.g == max {
            (rgba.b - rgba.r) / delta + 2.0
        } else {
            (rgba.r - rgba.g) / delta + 4.0
        } * 60.0;

        if h < 0.0 {
            h += 360.0;
        }

        Self::new(h, s, l, rgba.a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgba_from_hex() {
        assert_eq!(Rgba::from_hex("#1e1e1e").unwrap().to_hex(), "#1e1e1e");
        assert_eq!(Rgba::from_hex("#fff").unwrap(), Rgba::white());
        assert_eq!(Rgba::from_hex("#00000080").unwrap().a, 128.0 / 255.0);

        let red = Rgba::from_hex("#ff0000").unwrap();
        assert_eq!(red, Rgba::new(1.0, 0.0, 0.0, 1.0));
    }

    #[test]
    fn test_hsla_roundtrip() {
        let color = Rgba::new(0.5, 0.25, 0.75, 1.0);
        let hsla = Hsla::from_rgba(color);
        let back = hsla.to_rgba();
        assert!((color.r - back.r).abs() < 0.01);
        assert!((color.g - back.g).abs() < 0.01);
        assert!((color.b - back.b).abs() < 0.01);

        // Negative hue wraps to positive [0, 360)
        let hsla_neg = Hsla::new(-60.0, 1.0, 0.5, 1.0);
        assert_eq!(hsla_neg.h, 300.0);
        let hsla_pos = Hsla::new(300.0, 1.0, 0.5, 1.0);
        assert_eq!(hsla_neg.to_rgba(), hsla_pos.to_rgba());
    }
}
