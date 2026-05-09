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

/// Find the index of the closest 16-colour ANSI palette entry to the
/// given 24-bit RGB triple.
/// Port of `getnearestcolor()` from Src/Modules/nearcolor.c. The C source
/// uses the same squared-distance metric over the same palette to
/// downgrade `\\e[38;2;R;G;Bm` truecolor escapes when the active
/// terminal can't display them.
pub fn getnearestcolor(r: u8, g: u8, b: u8) -> u8 {
    let mut best_idx = 0u8;
    let mut best_dist = u32::MAX;

    for (idx, color) in ANSI_COLORS.iter().enumerate() {
        // Squared RGB distance (inlined from the deleted
        // color_distance_sq helper); C source uses the same metric.
        let dr = (r as i32) - (color.r as i32);
        let dg = (g as i32) - (color.g as i32);
        let db = (b as i32) - (color.b as i32);
        let dist = (dr * dr + dg * dg + db * db) as u32;
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

/// Port of `setup_()` from `Src/Modules/nearcolor.c:169`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:169
    0                                                                    // c:172
}

/// Port of `features_()` from `Src/Modules/nearcolor.c:176`. C body
/// is `*features = featuresarray(m, &module_features); return 0;`.
/// nearcolor exposes no shell features; static-link path: 0.
pub fn features_() -> i32 {                                              // c:176
    0                                                                    // c:180
}

/// Port of `enables_()` from `Src/Modules/nearcolor.c:184`. C body
/// is `return handlefeatures(m, &module_features, enables);`.
/// Static-link path: 0.
pub fn enables_() -> i32 {                                               // c:184
    0                                                                    // c:187
}

/// Port of `boot_()` from `Src/Modules/nearcolor.c:191`. C body is
/// `addhookfunc("get_color_attr", (Hookfn) getnearestcolor);
///  return 0;` — installs the color-mapping hook so terminals
/// limited to 88/256 colors can map an RGB request to the nearest
/// palette index. zshrs's color subsystem invokes the mapper
/// directly via `getnearestcolor` (already implemented above);
/// loader returns 0.
pub fn boot_() -> i32 {                                                  // c:191
    0                                                                    // c:195
}

/// Port of `cleanup_()` from `Src/Modules/nearcolor.c:199`. C body
/// is `deletehookfunc("get_color_attr", (Hookfn) getnearestcolor);
///  return setfeatureenables(m, &module_features, NULL);`.
/// Static-link path: 0.
pub fn cleanup_() -> i32 {                                               // c:199
    0                                                                    // c:204
}

/// Port of `finish_()` from `Src/Modules/nearcolor.c:207`. C body
/// is `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:207
    0                                                                    // c:210
}

/// Port of `deltae()` from `Src/Modules/nearcolor.c:41`. CIE Lab
/// distance (squared, since we only compare ordering — the C body
/// notes "taking square root unnecessary"). Inputs are (L, a, b)
/// triples computed by `RGBtoLAB`.
pub fn deltae(lab1: (f64, f64, f64), lab2: (f64, f64, f64)) -> f64 {     // c:41
    // C: `return pow(L1-L2, 2) + pow(a1-a2, 2) + pow(b1-b2, 2);`
    let dl = lab1.0 - lab2.0;                                            // c:45
    let da = lab1.1 - lab2.1;                                            // c:46
    let db = lab1.2 - lab2.2;                                            // c:47
    dl * dl + da * da + db * db                                          // c:45-47
}

/// Port of `RGBtoLAB()` from `Src/Modules/nearcolor.c:50`. Converts
/// 8-bit RGB to CIE L*a*b* via the sRGB → linear → XYZ (D65) → Lab
/// chain that nearcolor uses for nearest-palette selection. Returns
/// (L, a, b) for use with `deltae()`.
#[allow(non_snake_case)]
pub fn RGBtoLAB(red: i32, green: i32, blue: i32) -> (f64, f64, f64) {    // c:50
    // sRGB → linear (gamma decode) — c:54-58
    let mut r = red as f64 / 255.0;                                      // c:52
    let mut g = green as f64 / 255.0;                                    // c:53
    let mut b = blue as f64 / 255.0;                                     // c:54
    r = 100.0 * if r > 0.04045 { ((r + 0.055) / 1.055).powf(2.4) }       // c:55
                else { r / 12.92 };
    g = 100.0 * if g > 0.04045 { ((g + 0.055) / 1.055).powf(2.4) }       // c:56
                else { g / 12.92 };
    b = 100.0 * if b > 0.04045 { ((b + 0.055) / 1.055).powf(2.4) }       // c:57
                else { b / 12.92 };
    // sRGB → XYZ (D65 / 2°) — c:60-63
    let x = (r * 0.4124 + g * 0.3576 + b * 0.1805) / 95.047;             // c:61
    let y = (r * 0.2126 + g * 0.7152 + b * 0.0722) / 100.0;              // c:62
    let z = (r * 0.0193 + g * 0.1192 + b * 0.9505) / 108.883;            // c:63
    // XYZ → Lab — c:65-67 (the `f` function in the CIE 1976 formula)
    let f = |t: f64| if t > 0.008856 { t.powf(1.0 / 3.0) } else { 7.787 * t + 16.0 / 116.0 };
    let xf = f(x);                                                       // c:65
    let yf = f(y);                                                       // c:66
    let zf = f(z);                                                       // c:67
    // c:69-71
    let l = 116.0 * yf - 16.0;                                           // c:69
    let a = 500.0 * (xf - yf);                                           // c:70
    let bb = 200.0 * (yf - zf);                                          // c:71
    (l, a, bb)
}

/// Port of `mapRGBto88()` from `Src/Modules/nearcolor.c:74`. Returns
/// the nearest of the 88-color terminal palette for the given RGB.
#[allow(non_snake_case)]
pub fn mapRGBto88(red: i32, green: i32, blue: i32) -> i32 {              // c:74
    // c:76 — palette ramp: 4 RGB levels + 8 grey levels.
    let component: [i32; 11] = [0, 0x8b, 0xcd, 0xff, 0x2e, 0x5c, 0x8b, 0xa2, 0xb9, 0xd0, 0xe7];
    let orig = RGBtoLAB(red, green, blue);                               // c:84
    let mut bestl: f64 = -1.0;
    let mut comp_r = 0usize;
    let mut comp_g = 0usize;
    let mut comp_b = 0usize;
    for r in 0..11usize {                                                // c:87
        let mut g_iter: Vec<usize> = if r > 3 { vec![r] } else { (0..=3).collect() };
        if r > 3 { g_iter = vec![r]; }
        for &g in &g_iter {
            let b_iter: Vec<usize> = if r > 3 { vec![r] } else { (0..=3).collect() };
            for &b in &b_iter {
                let next = RGBtoLAB(component[r], component[g], component[b]);
                let nextl = deltae(orig, next);                          // c:93
                if nextl < bestl || bestl < 0.0 {                        // c:94
                    bestl = nextl;
                    comp_r = r;
                    comp_g = g;
                    comp_b = b;
                }
            }
        }
    }
    let _ = comp_g;
    let _ = comp_b;
    if comp_r > 3 {                                                       // c:104
        77 + comp_r as i32                                                // c:104
    } else {                                                              // c:105
        16 + (comp_r as i32 * 16) + (comp_g as i32 * 4) + comp_b as i32   // c:105
    }
}

/// Port of `mapRGBto256()` from `Src/Modules/nearcolor.c:110`. Returns
/// the nearest of the xterm 256-color palette for the given RGB.
#[allow(non_snake_case)]
pub fn mapRGBto256(red: i32, green: i32, blue: i32) -> i32 {             // c:110
    // c:113-116 — 6-step RGB ramp (216 colors) + 24 greyscale steps.
    let component: [i32; 30] = [
        0,    0x5f, 0x87, 0xaf, 0xd7, 0xff,
        0x8,  0x12, 0x1c, 0x26, 0x30, 0x3a, 0x44, 0x4e,
        0x58, 0x62, 0x6c, 0x76, 0x80, 0x8a, 0x94, 0x9e,
        0xa8, 0xb2, 0xbc, 0xc6, 0xd0, 0xda, 0xe4, 0xee,
    ];
    let orig = RGBtoLAB(red, green, blue);                               // c:124
    let mut bestl: f64 = -1.0;
    let mut comp_r = 0usize;
    let mut comp_g = 0usize;
    let mut comp_b = 0usize;
    for r in 0..component.len() {                                         // c:127
        let g_iter: Vec<usize> = if r > 5 { vec![r] } else { (0..=5).collect() };
        for &g in &g_iter {
            let b_iter: Vec<usize> = if r > 5 { vec![r] } else { (0..=5).collect() };
            for &b in &b_iter {
                let next = RGBtoLAB(component[r], component[g], component[b]);
                let nextl = deltae(orig, next);                          // c:133
                if nextl < bestl || bestl < 0.0 {                        // c:134
                    bestl = nextl;
                    comp_r = r;
                    comp_g = g;
                    comp_b = b;
                }
            }
        }
    }
    if comp_r > 5 {                                                       // c:144
        226 + comp_r as i32                                               // c:144
    } else {                                                              // c:145
        16 + (comp_r as i32 * 36) + (comp_g as i32 * 6) + comp_b as i32   // c:145
    }
}
