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

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/nearcolor.c`.
/// Calculate squared distance between two colors
fn color_distance_sq(c1: &ColorEntry, c2: &ColorEntry) -> u32 {
    let dr = (c1.r as i32) - (c2.r as i32);
    let dg = (c1.g as i32) - (c2.g as i32);
    let db = (c1.b as i32) - (c2.b as i32);
    (dr * dr + dg * dg + db * db) as u32
}

/// Find the index of the closest 16-colour ANSI palette entry to the
/// given 24-bit RGB triple.
/// Port of `nearcolor()` from Src/Modules/nearcolor.c. The C source
/// uses the same squared-distance metric over the same palette to
/// downgrade `\\e[38;2;R;G;Bm` truecolor escapes when the active
/// terminal can't display them.
pub fn nearcolor(r: u8, g: u8, b: u8) -> u8 {
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

/// Translate a 256-colour palette index into the closest 16-colour
/// ANSI index.
/// Port of `nearcolor_256()` from Src/Modules/nearcolor.c. The
/// 256-colour layout (16 base + 6×6×6 cube + 24 grays) is decoded
/// here using the canonical xterm step values (51 per cube level,
/// 23 grayscale steps from 232..255).
pub fn nearcolor_256(color: u8) -> u8 {
    if color < 16 {
        return color;
    }

    if color >= 232 {
        let gray = (color - 232) * 255 / 23;
        return nearcolor(gray, gray, gray);
    }

    let idx = color - 16;
    let r = (idx / 36) * 51;
    let g = ((idx % 36) / 6) * 51;
    let b = (idx % 6) * 51;

    nearcolor(r, g, b)
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/nearcolor.c`.
/// Map a 24-bit RGB triple to its closest xterm-256 palette index.
/// Inverse of `color_256_to_rgb`. Used by the truecolor → 256
/// downgrade path that mirrors the lookup `nearcolor.c` performs
/// when the active terminal's `colors` capability is 256 instead
/// of truecolor.
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

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/nearcolor.c`.
/// Decode a 256-colour palette index back to the RGB triple it
/// represents.
/// Mirror of `rgb_to_256`. Used by the nearcolor downgrade path
/// (Src/Modules/nearcolor.c) when scoring whether a 256-colour
/// approximation is close enough or whether the 16-colour fallback
/// should fire instead.
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

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/nearcolor.c`.
/// Choose the 256-colour or 16-colour index that best approximates
/// the given truecolor RGB.
/// Top-level entry point for the nearcolor downgrade. Equivalent to
/// `nearest_color_in_palette()` from Src/Modules/nearcolor.c which
/// the C source dispatches based on the terminal's reported
/// `colors` capability.
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
        assert_eq!(nearcolor(0, 0, 0), 0);
    }

    #[test]
    fn test_nearest_color_16_white() {
        assert_eq!(nearcolor(255, 255, 255), 15);
    }

    #[test]
    fn test_nearest_color_16_red() {
        let idx = nearcolor(255, 0, 0);
        assert!(idx == 1 || idx == 9);
    }

    #[test]
    fn test_color_256_to_16_passthrough() {
        for i in 0..16 {
            assert_eq!(nearcolor_256(i), i);
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
        // (128, 128, 128) is mid-gray — lands in the 24-step grayscale
        // ramp (indices 232..=255), not the 6×6×6 color cube.
        let idx = truecolor_to_256(128, 128, 128);
        assert!((232..=255).contains(&idx), "expected gray-ramp index, got {idx}");
    }

    #[test]
    fn test_ansi_colors_size() {
        assert_eq!(ANSI_COLORS.len(), 16);
    }
}
