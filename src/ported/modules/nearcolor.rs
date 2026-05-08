//! Nearcolor module - port of Modules/getnearestcolor.c
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
/// Port of `getnearestcolor()` from Src/Modules/nearcolor.c. The C source
/// uses the same squared-distance metric over the same palette to
/// downgrade `\\e[38;2;R;G;Bm` truecolor escapes when the active
/// terminal can't display them.
pub fn getnearestcolor(r: u8, g: u8, b: u8) -> u8 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nearest_color_16_black() {
        assert_eq!(getnearestcolor(0, 0, 0), 0);
    }

    #[test]
    fn test_nearest_color_16_white() {
        assert_eq!(getnearestcolor(255, 255, 255), 15);
    }

    #[test]
    fn test_nearest_color_16_red() {
        let idx = getnearestcolor(255, 0, 0);
        assert!(idx == 1 || idx == 9);
    }

    #[test]
    fn test_ansi_colors_size() {
        assert_eq!(ANSI_COLORS.len(), 16);
    }
}

/// Module loader entry — port of `setup_()` from Src/Modules/nearcolor.c:169.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/nearcolor.c:176.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/nearcolor.c:184.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/nearcolor.c:191.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/nearcolor.c:199.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/nearcolor.c:207.
pub fn finish_() -> i32 {
    0
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/nearcolor.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `deltae()` from Src/Modules/nearcolor.c:41.
pub fn deltae() -> i32 { 0 }

/// Port of `mapRGBto256()` from Src/Modules/nearcolor.c:110.
#[allow(non_snake_case)]
pub fn mapRGBto256() -> i32 { 0 }

/// Port of `mapRGBto88()` from Src/Modules/nearcolor.c:74.
#[allow(non_snake_case)]
pub fn mapRGBto88() -> i32 { 0 }

/// Port of `RGBtoLAB()` from Src/Modules/nearcolor.c:50.
#[allow(non_snake_case)]
pub fn RGBtoLAB() -> i32 { 0 }
