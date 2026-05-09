//! Nearcolor module — port of `Src/Modules/nearcolor.c`.
//!
//! C source has 1 struct (`struct cielab`) + 1 typedef alias
//! (`Cielab` → `struct cielab *`). Rust port mirrors the struct
//! letter-for-letter; the typedef-as-pointer collapses naturally
//! into Rust's `&Cielab` reference style.
//!
//! The module installs a `get_color_attr` hook that downgrades
//! 24-bit RGB color requests to the nearest entry in the 256- or
//! 88-color palette using a CIE Lab perceptual distance metric.

use crate::ported::init::TCCOLOURS;
use std::sync::atomic::Ordering;

/// Port of `struct cielab` from `Src/Modules/nearcolor.c:35-37`.
/// Three doubles representing the CIE 1976 L*a*b* color space.
/// C definition (line 35-37):
/// ```c
/// struct cielab {
///     double L, a, b;
/// };
/// typedef struct cielab *Cielab;
/// ```
#[derive(Debug, Clone, Copy)]
#[allow(non_snake_case)]
pub struct Cielab {                                                      // c:35 struct cielab
    pub L: f64,                                                          // c:36
    pub a: f64,                                                          // c:36
    pub b: f64,                                                          // c:36
}

/// Port of `deltae()` from `Src/Modules/nearcolor.c:41`. CIE Lab
/// distance (squared, since we only compare ordering — the C body
/// notes "taking square root unnecessary as we're just comparing
/// values").
///
/// C signature: `static double deltae(Cielab lab1, Cielab lab2)`.
pub fn deltae(lab1: &Cielab, lab2: &Cielab) -> f64 {                     // c:41
    // C: `return pow(L1-L2, 2) + pow(a1-a2, 2) + pow(b1-b2, 2);`
    let dl = lab1.L - lab2.L;                                            // c:44
    let da = lab1.a - lab2.a;                                            // c:45
    let db = lab1.b - lab2.b;                                            // c:46
    dl * dl + da * da + db * db
}

/// Port of `RGBtoLAB()` from `Src/Modules/nearcolor.c:50`. Converts
/// 8-bit sRGB to CIE L*a*b* via the sRGB → linear → XYZ (D65) →
/// Lab chain.
///
/// C signature: `static void RGBtoLAB(int red, int green, int blue,
///                                     Cielab lab)`.
/// The C body mutates `*lab`; the Rust port returns the populated
/// `Cielab` value (functionally equivalent — same fields written).
#[allow(non_snake_case)]
pub fn RGBtoLAB(red: i32, green: i32, blue: i32) -> Cielab {             // c:50
    // c:52-54 — sRGB normalised to [0,1].
    let mut R = red as f64 / 255.0;                                      // c:52
    let mut G = green as f64 / 255.0;                                    // c:53
    let mut B = blue as f64 / 255.0;                                     // c:54
    // c:55-57 — gamma decode + scale to 100.
    R = 100.0 * if R > 0.04045 { ((R + 0.055) / 1.055).powf(2.4) }       // c:55
                else { R / 12.92 };
    G = 100.0 * if G > 0.04045 { ((G + 0.055) / 1.055).powf(2.4) }       // c:56
                else { G / 12.92 };
    B = 100.0 * if B > 0.04045 { ((B + 0.055) / 1.055).powf(2.4) }       // c:57
                else { B / 12.92 };

    // c:59 — `/* Observer. = 2 degrees, Illuminant = D65 */`
    // c:60-62 — sRGB → XYZ (D65 / 2° observer), normalised by D65 ref.
    let X = (R * 0.4124 + G * 0.3576 + B * 0.1805) / 95.047;             // c:60
    let Y = (R * 0.2126 + G * 0.7152 + B * 0.0722) / 100.0;              // c:61
    let Z = (R * 0.0193 + G * 0.1192 + B * 0.9505) / 108.883;            // c:62

    // c:64-66 — XYZ → Lab via CIE 1976 `f` function.
    let X = if X > 0.008856 { X.powf(1.0 / 3.0) }
            else { 7.787 * X + 16.0 / 116.0 };                           // c:64
    let Y = if Y > 0.008856 { Y.powf(1.0 / 3.0) }
            else { 7.787 * Y + 16.0 / 116.0 };                           // c:65
    let Z = if Z > 0.008856 { Z.powf(1.0 / 3.0) }
            else { 7.787 * Z + 16.0 / 116.0 };                           // c:66

    // c:68-70 — final Lab values written to `*lab`.
    Cielab {
        L: 116.0 * Y - 16.0,                                             // c:68
        a: 500.0 * (X - Y),                                              // c:69
        b: 200.0 * (Y - Z),                                              // c:70
    }
}

/// Port of `mapRGBto88()` from `Src/Modules/nearcolor.c:74`. Maps
/// 24-bit RGB to the nearest entry in the 88-color terminal
/// palette via CIE Lab distance.
///
/// C signature: `static int mapRGBto88(int red, int green, int blue)`.
#[allow(non_snake_case)]
pub fn mapRGBto88(red: i32, green: i32, blue: i32) -> i32 {              // c:74
    // c:76 — palette ramp: 4 RGB levels + 7 grey levels.
    let component: [i32; 11] = [
        0, 0x8b, 0xcd, 0xff, 0x2e, 0x5c, 0x8b, 0xa2, 0xb9, 0xd0, 0xe7
    ];
    // c:77 — `struct cielab orig, next;`
    // c:78 — `double nextl, bestl = -1;`
    let mut bestl: f64 = -1.0;                                           // c:78
    // c:79 — `int r, g, b;`
    // c:80 — `int comp_r = 0, comp_g = 0, comp_b = 0;`
    let mut comp_r: i32 = 0;                                             // c:80
    let mut comp_g: i32 = 0;                                             // c:80
    let mut comp_b: i32 = 0;                                             // c:80

    // c:82-83 — `/* Get original value */` then `RGBtoLAB(...,&orig);`
    let orig = RGBtoLAB(red, green, blue);                               // c:83

    // c:85 — `/* try every one of the 72 colours */`
    // c:86-100 — three nested for-loops with the `if (r > 3) g = b = r;`
    // grey-ramp shortcut. Mirror the C control flow exactly with
    // mutable counters so the shortcut's effect on the loop conditions
    // matches C bit-for-bit.
    let mut r: i32 = 0;                                                  // c:86
    while r < 11 {                                                       // c:86
        let mut g: i32 = 0;                                              // c:87
        while g <= 3 {                                                   // c:87
            let mut b: i32 = 0;                                          // c:88
            while b <= 3 {                                               // c:88
                if r > 3 { g = r; b = r; }                               // c:89
                let next = RGBtoLAB(component[r as usize],               // c:90
                                    component[g as usize],
                                    component[b as usize]);
                let nextl = deltae(&orig, &next);                        // c:91
                if nextl < bestl || bestl < 0.0 {                        // c:92
                    bestl = nextl;                                       // c:93
                    comp_r = r;                                          // c:94
                    comp_g = g;                                          // c:95
                    comp_b = b;                                          // c:96
                }
                b += 1;                                                  // c:88
            }
            g += 1;                                                      // c:87
        }
        r += 1;                                                          // c:86
    }

    // c:102-103 — `return (comp_r > 3) ? 77 + comp_r :
    //                     16 + (comp_r * 16) + (comp_g * 4) + comp_b;`
    if comp_r > 3 {                                                      // c:102
        77 + comp_r                                                      // c:102
    } else {
        16 + (comp_r * 16) + (comp_g * 4) + comp_b                       // c:103
    }
}

/// Port of `mapRGBto256()` from `Src/Modules/nearcolor.c:110`.
/// Maps 24-bit RGB to the nearest entry in the xterm 256-color
/// palette via CIE Lab distance.
///
/// C comment (c:106-108):
/// ```text
/// Convert RGB to nearest colour in the 256 colour range
/// ```
///
/// C signature: `static int mapRGBto256(int red, int green, int blue)`.
#[allow(non_snake_case)]
pub fn mapRGBto256(red: i32, green: i32, blue: i32) -> i32 {             // c:110
    // c:112-117 — 6-step RGB ramp (216 colours) + 24-step greyscale.
    let component: [i32; 30] = [
        0,    0x5f, 0x87, 0xaf, 0xd7, 0xff,
        0x8,  0x12, 0x1c, 0x26, 0x30, 0x3a, 0x44, 0x4e,
        0x58, 0x62, 0x6c, 0x76, 0x80, 0x8a, 0x94, 0x9e,
        0xa8, 0xb2, 0xbc, 0xc6, 0xd0, 0xda, 0xe4, 0xee,
    ];
    // c:118 — `struct cielab orig, next;`
    // c:119 — `double nextl, bestl = -1;`
    let mut bestl: f64 = -1.0;                                           // c:119
    // c:120 — `int r, g, b;`
    // c:121 — `int comp_r = 0, comp_g = 0, comp_b = 0;`
    let mut comp_r: i32 = 0;                                             // c:121
    let mut comp_g: i32 = 0;                                             // c:121
    let mut comp_b: i32 = 0;                                             // c:121

    // c:123-124 — `/* Get original value */` then `RGBtoLAB(...,&orig);`
    let orig = RGBtoLAB(red, green, blue);                               // c:124

    // c:126-140 — three nested for-loops with the `if (r > 5) g = b = r;`
    // grey-ramp shortcut. C uses `r < sizeof(component)/sizeof(*component)`
    // which equals 30; Rust uses `component.len()` for the same value.
    let len = component.len() as i32;                                    // c:126
    let mut r: i32 = 0;                                                  // c:126
    while r < len {                                                      // c:126
        let mut g: i32 = 0;                                              // c:127
        while g <= 5 {                                                   // c:127
            let mut b: i32 = 0;                                          // c:128
            while b <= 5 {                                               // c:128
                if r > 5 { g = r; b = r; }                               // c:129
                let next = RGBtoLAB(component[r as usize],               // c:130
                                    component[g as usize],
                                    component[b as usize]);
                let nextl = deltae(&orig, &next);                        // c:131
                if nextl < bestl || bestl < 0.0 {                        // c:132
                    bestl = nextl;                                       // c:133
                    comp_r = r;                                          // c:134
                    comp_g = g;                                          // c:135
                    comp_b = b;                                          // c:136
                }
                b += 1;                                                  // c:128
            }
            g += 1;                                                      // c:127
        }
        r += 1;                                                          // c:126
    }

    // c:142-143 — `return (comp_r > 5) ? 226 + comp_r :
    //                     16 + (comp_r * 36) + (comp_g * 6) + comp_b;`
    if comp_r > 5 {                                                      // c:142
        226 + comp_r                                                     // c:142
    } else {
        16 + (comp_r * 36) + (comp_g * 6) + comp_b                       // c:143
    }
}

/// Port of `getnearestcolor()` from `Src/Modules/nearcolor.c:147`.
/// The hook installed via `addhookfunc("get_color_attr", ...)`.
///
/// C signature: `static int getnearestcolor(UNUSED(Hookdef dummy),
///                                            Color_rgb col)`.
/// `Color_rgb` is a 3-int RGB struct in zsh.h; the Rust port flattens
/// it to three `i32`s — same observable effect, no abstraction
/// invented.
///
/// C body (c:148-156) reads the global `tccolours` (init.c:94) to
/// dispatch between the 256-, 88-, or no-match path. Rust port loads
/// the matching `init::TCCOLOURS` static.
///
/// The `+ 1` on success is the c:149-151 comment trick: distinguish a
/// returned colour 0 from `runhookdef`'s "no hook registered"
/// sentinel of 0.
pub fn getnearestcolor(red: i32, green: i32, blue: i32) -> i32 {         // c:147
    let tccolours = TCCOLOURS.load(Ordering::Relaxed);                   // init.c:94 global
    if tccolours == 256 {                                                // c:152
        return mapRGBto256(red, green, blue) + 1;                        // c:153
    }
    if tccolours == 88 {                                                 // c:154
        return mapRGBto88(red, green, blue) + 1;                         // c:155
    }
    -1                                                                   // c:156
}

/// Port of `setup_()` from `Src/Modules/nearcolor.c:169`.
/// C body is `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:169
    0                                                                    // c:171
}

/// Port of `features_()` from `Src/Modules/nearcolor.c:176`.
/// C body is:
/// ```c
/// *features = featuresarray(m, &module_features);
/// return 0;
/// ```
/// `module_features` is the empty-table `struct features` at c:159
/// (zero builtins, params, conds, math fns) so the array is also
/// empty. zshrs's static-link path loads modules by direct `pub fn`
/// dispatch and does not maintain a runtime feature table — body is
/// a no-op `return 0` matching the C return value while skipping the
/// out-arg side effect (the caller in zshrs never reads it).
pub fn features_() -> i32 {                                              // c:176
    0                                                                    // c:179
}

/// Port of `enables_()` from `Src/Modules/nearcolor.c:184`.
/// C body is `return handlefeatures(m, &module_features, enables);`.
/// With the empty `module_features` table at c:159, `handlefeatures`
/// is a no-op that returns 0. zshrs static-link path: 0.
pub fn enables_() -> i32 {                                               // c:184
    0                                                                    // c:186
}

/// Port of `boot_()` from `Src/Modules/nearcolor.c:191`.
/// C body is:
/// ```c
/// addhookfunc("get_color_attr", (Hookfn) getnearestcolor);
/// return 0;
/// ```
/// zshrs's colour subsystem invokes [`getnearestcolor`] directly when
/// downgrade is needed (no separate hook registry); loader hook
/// returns 0.
pub fn boot_() -> i32 {                                                  // c:191
    0                                                                    // c:194
}

/// Port of `cleanup_()` from `Src/Modules/nearcolor.c:199`.
/// C body is:
/// ```c
/// deletehookfunc("get_color_attr", (Hookfn) getnearestcolor);
/// return setfeatureenables(m, &module_features, NULL);
/// ```
/// `setfeatureenables` on the empty table is a no-op returning 0,
/// and the hook is direct-dispatch (see `boot_`). Body returns 0.
pub fn cleanup_() -> i32 {                                               // c:199
    0                                                                    // c:202
}

/// Port of `finish_()` from `Src/Modules/nearcolor.c:207`.
/// C body is `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:207
    0                                                                    // c:209
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies `RGBtoLAB(0,0,0)` yields the C 1976 Lab origin
    /// (L≈0, a≈0, b≈0) — port of c:50 with red=green=blue=0.
    #[test]
    fn rgb_to_lab_black_is_zero() {
        let lab = RGBtoLAB(0, 0, 0);
        assert!(lab.L.abs() < 0.5);
        assert!(lab.a.abs() < 0.5);
        assert!(lab.b.abs() < 0.5);
    }

    /// Verifies `deltae` of a colour against itself is zero — c:41
    /// invariant when `lab1 == lab2`.
    #[test]
    fn deltae_self_is_zero() {
        let lab = RGBtoLAB(123, 45, 67);
        assert!(deltae(&lab, &lab).abs() < 1e-9);
    }

    /// Verifies pure white maps into the upper end of the 256-colour
    /// palette (>= 15) — sanity-check on the c:142-143 final-index
    /// formula.
    #[test]
    fn map_rgb_to_256_white_is_15_or_higher() {
        let idx = mapRGBto256(0xff, 0xff, 0xff);
        assert!(idx >= 15);
    }

    /// Verifies pure white maps into the 88-colour palette range —
    /// c:102-103 final-index formula.
    #[test]
    fn map_rgb_to_88_white_is_in_range() {
        let idx = mapRGBto88(0xff, 0xff, 0xff);
        assert!((16..=87).contains(&idx) || idx >= 77);
    }

    /// Verifies `getnearestcolor` dispatches on the `TCCOLOURS`
    /// global per c:152-156: 256→`mapRGBto256+1`, 88→`mapRGBto88+1`,
    /// otherwise -1.
    #[test]
    fn getnearestcolor_dispatches_on_tccolours() {
        let saved = TCCOLOURS.load(Ordering::SeqCst);
        TCCOLOURS.store(256, Ordering::SeqCst);
        let r256 = getnearestcolor(0xff, 0xff, 0xff);
        assert_eq!(r256, mapRGBto256(0xff, 0xff, 0xff) + 1);
        TCCOLOURS.store(88, Ordering::SeqCst);
        let r88 = getnearestcolor(0xff, 0xff, 0xff);
        assert_eq!(r88, mapRGBto88(0xff, 0xff, 0xff) + 1);
        TCCOLOURS.store(16, Ordering::SeqCst);
        assert_eq!(getnearestcolor(0xff, 0xff, 0xff), -1);
        TCCOLOURS.store(saved, Ordering::SeqCst);
    }

    /// Verifies the 8-color palette case (tccolours=8) returns the
    /// "no match" sentinel -1 — c:156 default branch.
    #[test]
    fn getnearestcolor_unsupported_returns_minus_one() {
        let saved = TCCOLOURS.load(Ordering::SeqCst);
        TCCOLOURS.store(8, Ordering::SeqCst);
        assert_eq!(getnearestcolor(128, 128, 128), -1);
        TCCOLOURS.store(saved, Ordering::SeqCst);
    }
}
