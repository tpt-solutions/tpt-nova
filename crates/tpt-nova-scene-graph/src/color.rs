//! Linear color values.

use serde::{Deserialize, Serialize};

/// A linear RGB color (no alpha), used for lights and environment settings.
///
/// Colors are stored in **linear** space because that is what the renderer's
/// shading math expects. [`Color::hex`] accepts the sRGB hex strings designers
/// type (`"#404050"`) and converts them to linear.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Color {
    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
    };
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
    };

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Color { r, g, b }
    }

    /// Decode an sRGB hex string (`#RRGGBB`, `RRGGBB`, or `#RGB`) into linear
    /// RGB.
    ///
    /// # Panics
    /// Panics on a malformed string; use [`Color::try_hex`] for untrusted input.
    pub fn hex(hex: &str) -> Self {
        Self::try_hex(hex).unwrap_or_else(|| panic!("invalid color hex string: {hex:?}"))
    }

    /// Non-panicking [`Color::hex`].
    pub fn try_hex(hex: &str) -> Option<Self> {
        let s = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
        let expand = |c: char| -> Option<u8> {
            let v = c.to_digit(16)? as u8;
            Some(v * 17)
        };
        let (r, g, b) = match s.len() {
            3 => {
                let mut chars = s.chars();
                (
                    expand(chars.next()?)?,
                    expand(chars.next()?)?,
                    expand(chars.next()?)?,
                )
            }
            6 => {
                let v = u32::from_str_radix(s, 16).ok()?;
                (
                    ((v >> 16) & 0xFF) as u8,
                    ((v >> 8) & 0xFF) as u8,
                    (v & 0xFF) as u8,
                )
            }
            _ => return None,
        };
        Some(Color {
            r: srgb_to_linear(r as f32 / 255.0),
            g: srgb_to_linear(g as f32 / 255.0),
            b: srgb_to_linear(b as f32 / 255.0),
        })
    }

    pub fn to_array(self) -> [f32; 3] {
        [self.r, self.g, self.b]
    }

    pub fn to_vec3(self) -> glam::Vec3 {
        glam::Vec3::new(self.r, self.g, self.b)
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::WHITE
    }
}

/// Standard sRGB transfer function (IEC 61966-2-1).
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_decodes_srgb_to_linear() {
        assert_eq!(Color::hex("#FFFFFF"), Color::WHITE);
        assert_eq!(Color::hex("000000"), Color::BLACK);
        let mid = Color::hex("#808080");
        assert!((mid.r - 0.2159).abs() < 1e-3, "{mid:?}");
        assert_eq!(Color::hex("#F00"), Color::hex("#FF0000"));
    }

    #[test]
    fn malformed_hex_is_rejected() {
        assert!(Color::try_hex("#12").is_none());
        assert!(Color::try_hex("#GGGGGG").is_none());
        assert!(Color::try_hex("").is_none());
    }
}
