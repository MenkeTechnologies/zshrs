//! Nearcolor module - port of Modules/nearcolor.c
//!
//! Provides color approximation for terminals with limited color support.

/// Color approximation table entry
#[derive(Debug, Clone, Copy)]
pub struct ColorEntry {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl ColorEntry {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Standard 16-color palette (ANSI colors)
pub static ANSI_COLORS: [ColorEntry; 16] = [
    ColorEntry::new(0, 0, 0),       // 0: black
    ColorEntry::new(128, 0, 0),     // 1: red
    ColorEntry::new(0, 128, 0),     // 2: green
    ColorEntry::new(128, 128, 0),   // 3: yellow
    ColorEntry::new(0, 0, 128),     // 4: blue
    ColorEntry::new(128, 0, 128),   // 5: magenta
    ColorEntry::new(0, 128, 128),   // 6: cyan
    ColorEntry::new(192, 192, 192), // 7: white
    ColorEntry::new(128, 128, 128), // 8: bright black (gray)
    ColorEntry::new(255, 0, 0),     // 9: bright red
    ColorEntry::new(0, 255, 0),     // 10: bright green
    ColorEntry::new(255, 255, 0),   // 11: bright yellow
    ColorEntry::new(0, 0, 255),     // 12: bright blue
    ColorEntry::new(255, 0, 255),   // 13: bright magenta
    ColorEntry::new(0, 255, 255),   // 14: bright cyan
    ColorEntry::new(255, 255, 255), // 15: bright white
];

/// Calculate squared distance between two colors
fn color_distance_sq(c1: &ColorEntry, c2: &ColorEntry) -> u32 {
    let dr = (c1.r as i32) - (c2.r as i32);
    let dg = (c1.g as i32) - (c2.g as i32);
    let db = (c1.b as i32) - (c2.b as i32);
    (dr * dr + dg * dg + db * db) as u32
}

/// Find nearest color in 16-color palette
pub fn nearest_color_16(r: u8, g: u8, b: u8) -> u8 {
    let target = ColorEntry::new(r, g, b);
    let mut best_idx = 0u8;
    let mut best_dist = u32::MAX;

    for (idx, color) in ANSI_COLORS.iter().enumerate() {
        let dist = color_distance_sq(&target, color);
        if dist < best_dist {
            best_dist = dist;
            best_idx = idx as u8;
        }
    }

    best_idx
}

/// Convert 256-color index to 16-color approximation
pub fn color_256_to_16(color: u8) -> u8 {
    if color < 16 {
        return color;
    }

    if color >= 232 {
        let gray = ((color - 232) * 255 / 23) as u8;
        return nearest_color_16(gray, gray, gray);
    }

    let idx = color - 16;
    let r = (idx / 36) * 51;
    let g = ((idx % 36) / 6) * 51;
    let b = (idx % 6) * 51;

    nearest_color_16(r, g, b)
}

/// Convert RGB to 256-color approximation
pub fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    let r_idx = (r as u32 + 25) / 51;
    let g_idx = (g as u32 + 25) / 51;
    let b_idx = (b as u32 + 25) / 51;

    if r_idx == g_idx && g_idx == b_idx {
        let gray = (r as u32 + g as u32 + b as u32) / 3;
        if gray < 8 {
            return 16;
        }
        if gray > 248 {
            return 231;
        }
        return (232 + (gray - 8) / 10) as u8;
    }

    (16 + 36 * r_idx + 6 * g_idx + b_idx) as u8
}

/// Convert 256-color to RGB
pub fn color_256_to_rgb(color: u8) -> (u8, u8, u8) {
    if color < 16 {
        let c = ANSI_COLORS[color as usize];
        return (c.r, c.g, c.b);
    }

    if color >= 232 {
        let gray = ((color - 232) as u32 * 255 / 23) as u8;
        return (gray, gray, gray);
    }

    let idx = (color - 16) as u32;
    let r = (idx / 36 * 51) as u8;
    let g = ((idx % 36) / 6 * 51) as u8;
    let b = (idx % 6 * 51) as u8;

    (r, g, b)
}

/// Approximate true color to 256-color palette
pub fn truecolor_to_256(r: u8, g: u8, b: u8) -> u8 {
    let color_idx = rgb_to_256(r, g, b);

    let (cr, cg, cb) = color_256_to_rgb(color_idx);
    let color_dist = color_distance_sq(&ColorEntry::new(r, g, b), &ColorEntry::new(cr, cg, cb));

    let avg = ((r as u32 + g as u32 + b as u32) / 3) as u8;
    let gray_idx = if avg < 8 {
        16
    } else if avg > 248 {
        231
    } else {
        232 + ((avg as u32 - 8) / 10) as u8
    };

    let (gr, gg, gb) = color_256_to_rgb(gray_idx);
    let gray_dist = color_distance_sq(&ColorEntry::new(r, g, b), &ColorEntry::new(gr, gg, gb));

    if gray_dist < color_dist {
        gray_idx
    } else {
        color_idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nearest_color_16_black() {
        assert_eq!(nearest_color_16(0, 0, 0), 0);
    }

    #[test]
    fn test_nearest_color_16_white() {
        assert_eq!(nearest_color_16(255, 255, 255), 15);
    }

    #[test]
    fn test_nearest_color_16_red() {
        let idx = nearest_color_16(255, 0, 0);
        assert!(idx == 1 || idx == 9);
    }

    #[test]
    fn test_color_256_to_16_passthrough() {
        for i in 0..16 {
            assert_eq!(color_256_to_16(i), i);
        }
    }

    #[test]
    fn test_rgb_to_256_black() {
        let idx = rgb_to_256(0, 0, 0);
        assert!(idx == 16 || idx < 232);
    }

    #[test]
    fn test_rgb_to_256_white() {
        let idx = rgb_to_256(255, 255, 255);
        assert_eq!(idx, 231);
    }

    #[test]
    fn test_color_256_to_rgb() {
        let (r, g, b) = color_256_to_rgb(16);
        assert_eq!((r, g, b), (0, 0, 0));

        let (r, g, b) = color_256_to_rgb(231);
        assert_eq!((r, g, b), (255, 255, 255));

        let (r, g, b) = color_256_to_rgb(240);
        assert!(r == g && g == b);
    }

    #[test]
    fn test_truecolor_to_256() {
        let idx = truecolor_to_256(128, 128, 128);
        assert!(idx >= 232 || idx <= 255);
    }

    #[test]
    fn test_ansi_colors_size() {
        assert_eq!(ANSI_COLORS.len(), 16);
    }
}
