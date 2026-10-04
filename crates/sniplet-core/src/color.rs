use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Self = Self::new(0, 0, 0, 0);
    pub const BLACK: Self = Self::new(0, 0, 0, 255);
    pub const WHITE: Self = Self::new(255, 255, 255, 255);
    pub const RED: Self = Self::new(255, 55, 66, 255);

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_hex(value: &str) -> Option<Self> {
        let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
        let bytes = value.as_bytes();
        if !bytes.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        match bytes.len() {
            3 => Some(Self::new(
                hex_nibble(bytes[0])? * 17,
                hex_nibble(bytes[1])? * 17,
                hex_nibble(bytes[2])? * 17,
                255,
            )),
            6 | 8 => Some(Self::new(
                hex_pair(bytes[0], bytes[1])?,
                hex_pair(bytes[2], bytes[3])?,
                hex_pair(bytes[4], bytes[5])?,
                if bytes.len() == 8 {
                    hex_pair(bytes[6], bytes[7])?
                } else {
                    255
                },
            )),
            _ => None,
        }
    }

    pub fn format(self, format: ColorFormat) -> String {
        match format {
            ColorFormat::HexRgb => format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b),
            ColorFormat::HexRgba => {
                format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
            }
            ColorFormat::Rgb => format!("rgb({}, {}, {})", self.r, self.g, self.b),
            ColorFormat::Rgba => format!(
                "rgba({}, {}, {}, {:.3})",
                self.r,
                self.g,
                self.b,
                self.a as f32 / 255.0
            ),
            ColorFormat::Hsl => {
                let (h, s, l) = self.hsl();
                format!("hsl({h:.0}, {s:.0}%, {l:.0}%)")
            }
        }
    }

    fn hsl(self) -> (f32, f32, f32) {
        let r = self.r as f32 / 255.0;
        let g = self.g as f32 / 255.0;
        let b = self.b as f32 / 255.0;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let lightness = (max + min) / 2.0;
        if (max - min).abs() < f32::EPSILON {
            return (0.0, 0.0, lightness * 100.0);
        }
        let delta = max - min;
        let saturation = if lightness > 0.5 {
            delta / (2.0 - max - min)
        } else {
            delta / (max + min)
        };
        let mut hue = if max == r {
            (g - b) / delta + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / delta + 2.0
        } else {
            (r - g) / delta + 4.0
        };
        hue *= 60.0;
        (hue, saturation * 100.0, lightness * 100.0)
    }
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    Some(hex_nibble(high)? * 16 + hex_nibble(low)?)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

impl From<Color> for image::Rgba<u8> {
    fn from(value: Color) -> Self {
        image::Rgba([value.r, value.g, value.b, value.a])
    }
}

impl From<image::Rgba<u8>> for Color {
    fn from(value: image::Rgba<u8>) -> Self {
        let [r, g, b, a] = value.0;
        Self::new(r, g, b, a)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorFormat {
    HexRgb,
    HexRgba,
    Rgb,
    Rgba,
    Hsl,
}
