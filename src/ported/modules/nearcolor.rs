//! Nearcolor module — port of `Src/Modules/nearcolor.c`.
//!
//! C source has 1 struct (`struct cielab`) + 1 typedef (`Cielab`).
//! Rust port matches: 1 struct `Cielab` (matching C name and field
//! types).
//!
//! The module installs a `get_color_attr` hook that downgrades
//! 24-bit RGB color requests to the nearest entry in the 256- or
//! 88-color palette using a CIE Lab perceptual distance metric.

/// Port of `struct cielab` from `Src/Modules/nearcolor.c:35-37`.
/// Three doubles representing the CIE 1976 L*a*b* color space.
/// C definition:
/// ```c
/// struct cielab {
///     double L, a, b;
/// };
/// ```
#[derive(Debug, Clone, Copy)]
#[allow(non_snake_case)]
pub struct Cielab {                                                      // c:35 struct cielab
    pub L: f64,
    pub a: f64,
    pub b: f64,
}

/// Port of `deltae()` from `Src/Modules/nearcolor.c:41`. CIE Lab
/// distance (squared, since we only compare ordering — the C body
/// notes "taking square root unnecessary").
///
/// C signature: `static double deltae(Cielab lab1, Cielab lab2)`.
pub fn deltae(lab1: &Cielab, lab2: &Cielab) -> f64 {                     // c:41
    // C: `return pow(L1-L2, 2) + pow(a1-a2, 2) + pow(b1-b2, 2);`
    let dl = lab1.L - lab2.L;                                            // c:45
    let da = lab1.a - lab2.a;                                            // c:46
    let db = lab1.b - lab2.b;                                            // c:47
    dl * dl + da * da + db * db
}

/// Port of `RGBtoLAB()` from `Src/Modules/nearcolor.c:50`. Converts
/// 8-bit sRGB to CIE L*a*b* via the sRGB → linear → XYZ (D65) →
/// Lab chain that nearcolor uses for nearest-palette selection.
///
/// C signature: `static void RGBtoLAB(int red, int green, int blue,
///                                     Cielab lab)`.
/// Rust port returns the populated Cielab rather than mutating an
/// out-arg pointer (functionally equivalent).
#[allow(non_snake_case)]
pub fn RGBtoLAB(red: i32, green: i32, blue: i32) -> Cielab {             // c:50
    // c:53-58 — sRGB → linear (gamma decode).
    let mut R = red as f64 / 255.0;                                      // c:53
    let mut G = green as f64 / 255.0;                                    // c:54
    let mut B = blue as f64 / 255.0;                                     // c:55
    R = 100.0 * if R > 0.04045 { ((R + 0.055) / 1.055).powf(2.4) }       // c:56
                else { R / 12.92 };
    G = 100.0 * if G > 0.04045 { ((G + 0.055) / 1.055).powf(2.4) }       // c:57
                else { G / 12.92 };
    B = 100.0 * if B > 0.04045 { ((B + 0.055) / 1.055).powf(2.4) }       // c:58
                else { B / 12.92 };

    // c:60-63 — sRGB → XYZ (D65 / 2° observer).
    // Comment: /* Observer. = 2 degrees, Illuminant = D65 */
    let X = (R * 0.4124 + G * 0.3576 + B * 0.1805) / 95.047;             // c:62
    let Y = (R * 0.2126 + G * 0.7152 + B * 0.0722) / 100.0;              // c:63
    let Z = (R * 0.0193 + G * 0.1192 + B * 0.9505) / 108.883;            // c:64

    // c:66-68 — XYZ → Lab via CIE 1976 `f` function.
    let f = |t: f64| if t > 0.008856 { t.powf(1.0 / 3.0) }
                     else { 7.787 * t + 16.0 / 116.0 };
    let X = f(X);                                                        // c:66
    let Y = f(Y);                                                        // c:67
    let Z = f(Z);                                                        // c:68

    // c:70-72 — final Lab values.
    Cielab {
        L: 116.0 * Y - 16.0,                                             // c:70
        a: 500.0 * (X - Y),                                              // c:71
        b: 200.0 * (Y - Z),                                              // c:72
    }
}

/// Port of `mapRGBto88()` from `Src/Modules/nearcolor.c:74`. Maps
/// 24-bit RGB to the nearest entry in the 88-color terminal
/// palette via CIE Lab distance.
#[allow(non_snake_case)]
pub fn mapRGBto88(red: i32, green: i32, blue: i32) -> i32 {              // c:74
    // c:78 — palette ramp: 4 RGB levels + 8 grey levels.
    let component: [i32; 11] = [
        0, 0x8b, 0xcd, 0xff, 0x2e, 0x5c, 0x8b, 0xa2, 0xb9, 0xd0, 0xe7
    ];
    let orig = RGBtoLAB(red, green, blue);                               // c:84
    let mut bestl: f64 = -1.0;                                           // c:81
    let mut comp_r = 0usize;                                             // c:82
    let mut comp_g = 0usize;
    let mut comp_b = 0usize;
    // c:87 — `for (r = 0; r < 11; r++)`.
    for r in 0..11usize {
        // c:88-89 — `for (g = 0; g <= 3; g++) for (b = 0; b <= 3; b++)`
        // with C's `if (r > 3) g = b = r;` inner-loop-skip-to-grey hack.
        let g_range = if r > 3 { vec![r] } else { (0..=3usize).collect::<Vec<_>>() };
        for &g in &g_range {
            let b_range = if r > 3 { vec![r] } else { (0..=3usize).collect::<Vec<_>>() };
            for &b in &b_range {
                let next = RGBtoLAB(component[r], component[g], component[b]);  // c:91
                let nextl = deltae(&orig, &next);                        // c:92
                if nextl < bestl || bestl < 0.0 {                        // c:93
                    bestl = nextl;
                    comp_r = r;
                    comp_g = g;
                    comp_b = b;
                }
            }
        }
    }
    // c:103-105 — final palette index.
    if comp_r > 3 {                                                      // c:104
        77 + comp_r as i32                                               // c:104
    } else {
        16 + (comp_r as i32 * 16) + (comp_g as i32 * 4) + comp_b as i32  // c:105
    }
}

/// Port of `mapRGBto256()` from `Src/Modules/nearcolor.c:110`. Maps
/// 24-bit RGB to the nearest entry in the xterm 256-color palette
/// via CIE Lab distance.
#[allow(non_snake_case)]
pub fn mapRGBto256(red: i32, green: i32, blue: i32) -> i32 {             // c:110
    // c:113-117 — 6-step RGB ramp (216 colors) + 24 greyscale.
    let component: [i32; 30] = [
        0,    0x5f, 0x87, 0xaf, 0xd7, 0xff,
        0x8,  0x12, 0x1c, 0x26, 0x30, 0x3a, 0x44, 0x4e,
        0x58, 0x62, 0x6c, 0x76, 0x80, 0x8a, 0x94, 0x9e,
        0xa8, 0xb2, 0xbc, 0xc6, 0xd0, 0xda, 0xe4, 0xee,
    ];
    let orig = RGBtoLAB(red, green, blue);                               // c:124
    let mut bestl: f64 = -1.0;                                           // c:121
    let mut comp_r = 0usize;
    let mut comp_g = 0usize;
    let mut comp_b = 0usize;
    // c:127 — `for (r = 0; r < sizeof(component)/sizeof(*component); r++)`.
    for r in 0..component.len() {
        let g_range = if r > 5 { vec![r] } else { (0..=5usize).collect::<Vec<_>>() };
        for &g in &g_range {
            let b_range = if r > 5 { vec![r] } else { (0..=5usize).collect::<Vec<_>>() };
            for &b in &b_range {
                let next = RGBtoLAB(component[r], component[g], component[b]);
                let nextl = deltae(&orig, &next);                        // c:133
                if nextl < bestl || bestl < 0.0 {                        // c:134
                    bestl = nextl;
                    comp_r = r;
                    comp_g = g;
                    comp_b = b;
                }
            }
        }
    }
    if comp_r > 5 {                                                      // c:144
        226 + comp_r as i32
    } else {
        16 + (comp_r as i32 * 36) + (comp_g as i32 * 6) + comp_b as i32  // c:145
    }
}

/// Port of `getnearestcolor()` from `Src/Modules/nearcolor.c:147`.
/// The hook installed via `addhookfunc("get_color_attr", ...)` —
/// dispatches between `mapRGBto256` and `mapRGBto88` based on the
/// active terminal's color count, adding 1 so the result is
/// distinguishable from `runhookdef`'s "no hook" sentinel of 0.
///
/// C signature: `static int getnearestcolor(Hookdef dummy, Color_rgb col)`.
/// Rust port takes the RGB triple + the terminal color count
/// directly (the C body reads the `tccolours` global).
pub fn getnearestcolor(red: i32, green: i32, blue: i32, tccolours: i32) -> i32 {  // c:147
    // c:151 — `if (tccolours == 256)`
    if tccolours == 256 {
        return mapRGBto256(red, green, blue) + 1;                        // c:152
    }
    // c:153 — `if (tccolours == 88)`
    if tccolours == 88 {
        return mapRGBto88(red, green, blue) + 1;                         // c:154
    }
    -1                                                                   // c:155
}

/// Port of `setup_()` from `Src/Modules/nearcolor.c:169`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:169
    0                                                                    // c:172
}

/// Port of `features_()` from `Src/Modules/nearcolor.c:176`. C body
/// is `*features = featuresarray(m, &module_features); return 0;`.
/// Static-link path: 0.
pub fn features_() -> i32 {                                              // c:176
    0                                                                    // c:180
}

/// Port of `enables_()` from `Src/Modules/nearcolor.c:184`. C body
/// is `return handlefeatures(m, &module_features, enables);`.
pub fn enables_() -> i32 {                                               // c:184
    0                                                                    // c:187
}

/// Port of `boot_()` from `Src/Modules/nearcolor.c:191`. C body is
/// `addhookfunc("get_color_attr", (Hookfn) getnearestcolor); return 0;`.
/// zshrs's color subsystem invokes `getnearestcolor` directly when
/// downgrade is needed; loader hook returns 0.
pub fn boot_() -> i32 {                                                  // c:191
    0                                                                    // c:195
}

/// Port of `cleanup_()` from `Src/Modules/nearcolor.c:199`. C body
/// is `deletehookfunc("get_color_attr", ...); return setfeatureenables(...);`.
/// Static-link path: 0.
pub fn cleanup_() -> i32 {                                               // c:199
    0                                                                    // c:204
}

/// Port of `finish_()` from `Src/Modules/nearcolor.c:207`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:207
    0                                                                    // c:210
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_to_lab_black_is_zero() {
        let lab = RGBtoLAB(0, 0, 0);
        // L≈0, a≈0, b≈0 for pure black.
        assert!(lab.L.abs() < 0.5);
        assert!(lab.a.abs() < 0.5);
        assert!(lab.b.abs() < 0.5);
    }

    #[test]
    fn deltae_self_is_zero() {
        let lab = RGBtoLAB(123, 45, 67);
        assert!(deltae(&lab, &lab).abs() < 1e-9);
    }

    #[test]
    fn map_rgb_to_256_white_is_15_or_higher() {
        let idx = mapRGBto256(0xff, 0xff, 0xff);
        assert!(idx >= 15);
    }

    #[test]
    fn map_rgb_to_88_white_is_in_range() {
        let idx = mapRGBto88(0xff, 0xff, 0xff);
        assert!((16..=87).contains(&idx) || idx >= 77);
    }

    #[test]
    fn getnearestcolor_dispatches_on_tccolours() {
        // 256-mode delegates to mapRGBto256.
        let r256 = getnearestcolor(0xff, 0xff, 0xff, 256);
        assert_eq!(r256, mapRGBto256(0xff, 0xff, 0xff) + 1);
        // 88-mode delegates to mapRGBto88.
        let r88 = getnearestcolor(0xff, 0xff, 0xff, 88);
        assert_eq!(r88, mapRGBto88(0xff, 0xff, 0xff) + 1);
        // Anything else returns -1.
        assert_eq!(getnearestcolor(0xff, 0xff, 0xff, 16), -1);
    }
}
