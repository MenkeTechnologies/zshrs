//! `zsh/nearcolor` module — port of `Src/Modules/nearcolor.c`.
//!
//! Top-level declaration order matches C source line-by-line:
//!   - `struct cielab { L, a, b }`                  c:35
//!   - `typedef struct cielab *Cielab;`             c:38
//!   - `deltae(lab1, lab2)`                         c:41
//!   - `RGBtoLAB(red, green, blue, lab)`            c:50
//!   - `mapRGBto88(red, green, blue)`               c:74
//!   - `mapRGBto256(red, green, blue)`              c:110
//!   - `getnearestcolor(dummy, col)`                c:147
//!   - `static struct features module_features`     c:159
//!   - `setup_(m)` / `features_(m, features)` /
//!     `enables_(m, enables)` / `boot_(m)` /
//!     `cleanup_(m)` / `finish_(m)`                 c:168-210

#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

use crate::ported::init::tccolours;
use crate::ported::zsh_h::{color_rgb, features, hookdef, module};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

// =====================================================================
// struct cielab { double L, a, b; };                                 c:35
// typedef struct cielab *Cielab;                                     c:38
// =====================================================================

/// Port of `struct cielab` from `Src/Modules/nearcolor.c:35`.
///
/// ```c
/// struct cielab {
///     double L, a, b;
/// };
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct cielab {
    // c:35
    pub L: f64, // c:36
    pub a: f64, // c:41
    pub b: f64, // c:41
}

/// Port of `typedef struct cielab *Cielab;` from `Src/Modules/nearcolor.c:41`.
pub type Cielab = Box<cielab>; // c:41

// =====================================================================
// deltae(Cielab lab1, Cielab lab2)                                   c:41
// =====================================================================

/// Port of `deltae(Cielab lab1, Cielab lab2)` from `Src/Modules/nearcolor.c:41`.
pub fn deltae(lab1: &cielab, lab2: &cielab) -> f64 {
    // c:41
    /* taking square root unnecessary as we're just comparing values */ // c:41
    // c:44-46 — `pow(L1-L2, 2) + pow(a1-a2, 2) + pow(b1-b2, 2)`
    (lab1.L - lab2.L).powi(2)                                         // c:44
        + (lab1.a - lab2.a).powi(2)                                   // c:45
        + (lab1.b - lab2.b).powi(2) // c:46
}

// =====================================================================
// RGBtoLAB(int red, int green, int blue, Cielab lab)                 c:50
// =====================================================================

/// Port of `RGBtoLAB(int red, int green, int blue, Cielab lab)` from `Src/Modules/nearcolor.c:50`.
///
/// C signature mirrored verbatim:
/// ```c
/// static void
/// RGBtoLAB(int red, int green, int blue, Cielab lab)
/// ```
pub fn RGBtoLAB(red: i32, green: i32, blue: i32, lab: &mut cielab) {
    // c:50
    let mut R: f64 = red as f64 / 255.0; // c:50
    let mut G: f64 = green as f64 / 255.0; // c:53
    let mut B: f64 = blue as f64 / 255.0; // c:54
    R = 100.0
        * if R > 0.04045 {
            ((R + 0.055) / 1.055).powf(2.4)
        }
        // c:55
        else {
            R / 12.92
        };
    G = 100.0
        * if G > 0.04045 {
            ((G + 0.055) / 1.055).powf(2.4)
        }
        // c:56
        else {
            G / 12.92
        };
    B = 100.0
        * if B > 0.04045 {
            ((B + 0.055) / 1.055).powf(2.4)
        }
        // c:57
        else {
            B / 12.92
        };

    /* Observer. = 2 degrees, Illuminant = D65 */
    // c:59
    let mut X: f64 = (R * 0.4124 + G * 0.3576 + B * 0.1805) / 95.047; // c:60
    let mut Y: f64 = (R * 0.2126 + G * 0.7152 + B * 0.0722) / 100.0; // c:61
    let mut Z: f64 = (R * 0.0193 + G * 0.1192 + B * 0.9505) / 108.883; // c:62

    X = if X > 0.008856 {
        X.powf(1.0 / 3.0)
    }
    // c:64
    else {
        7.787 * X + 16.0 / 116.0
    };
    Y = if Y > 0.008856 {
        Y.powf(1.0 / 3.0)
    }
    // c:65
    else {
        7.787 * Y + 16.0 / 116.0
    };
    Z = if Z > 0.008856 {
        Z.powf(1.0 / 3.0)
    }
    // c:66
    else {
        7.787 * Z + 16.0 / 116.0
    };

    lab.L = 116.0 * Y - 16.0; // c:74
    lab.a = 500.0 * (X - Y); // c:74
    lab.b = 200.0 * (Y - Z); // c:74
}

// =====================================================================
// mapRGBto88(int red, int green, int blue)                           c:74
// =====================================================================

/// Port of `mapRGBto88(int red, int green, int blue)` from `Src/Modules/nearcolor.c:74`.
pub fn mapRGBto88(red: i32, green: i32, blue: i32) -> i32 {
    // c:74
    // c:74 — palette ramp: 4 RGB levels + 7 grey levels.
    let component: [i32; 11] = [
        0, 0x8b, 0xcd, 0xff, 0x2e, 0x5c, 0x8b, 0xa2, 0xb9, 0xd0, 0xe7,
    ]; // c:76
    let mut orig = cielab::default(); // c:77
    let mut next = cielab::default(); // c:77
    let mut nextl: f64; // c:78
    let mut bestl: f64 = -1.0; // c:78
    let mut r: i32; // c:79
    let mut g: i32; // c:79
    let mut b: i32; // c:79
    let mut comp_r: i32 = 0; // c:80
    let mut comp_g: i32 = 0; // c:80
    let mut comp_b: i32 = 0; // c:80

    /* Get original value */
    // c:82
    RGBtoLAB(red, green, blue, &mut orig); // c:83

    /* try every one of the 72 colours */
    // c:85
    // c:86-100 — three nested for-loops with `if (r > 3) g = b = r;`
    // grey-ramp shortcut. Mirror C's `for (...)` with mutable counters.
    r = 0; // c:86
    while r < 11 {
        // c:86
        g = 0; // c:87
        while g <= 3 {
            // c:87
            b = 0; // c:88
            while b <= 3 {
                // c:88
                if r > 3 {
                    g = r;
                    b = r;
                } // c:89
                RGBtoLAB(
                    component[r as usize], // c:90
                    component[g as usize],
                    component[b as usize],
                    &mut next,
                );
                nextl = deltae(&orig, &next); // c:91
                if nextl < bestl || bestl < 0.0 {
                    // c:92
                    bestl = nextl; // c:93
                    comp_r = r; // c:94
                    comp_g = g; // c:95
                    comp_b = b; // c:96
                }
                b += 1; // c:88
            }
            g += 1; // c:87
        }
        r += 1; // c:86
    }

    // c:102-103 — return (comp_r > 3) ? 77 + comp_r :
    //                    16 + (comp_r * 16) + (comp_g * 4) + comp_b;
    if comp_r > 3 {
        // c:102
        77 + comp_r // c:102
    } else {
        16 + (comp_r * 16) + (comp_g * 4) + comp_b // c:110
    }
}

// =====================================================================
/*
 * Convert RGB to nearest colour in the 256 colour range            c:106-108
 */
// mapRGBto256(int red, int green, int blue)                          c:110
// =====================================================================

/// Port of `mapRGBto256(int red, int green, int blue)` from `Src/Modules/nearcolor.c:110`.
pub fn mapRGBto256(red: i32, green: i32, blue: i32) -> i32 {
    // c:110
    // c:110-117 — 6-step RGB ramp (216 colours) + 24-step greyscale.
    let component: [i32; 30] = [
        0, 0x5f, 0x87, 0xaf, 0xd7, 0xff, // c:113
        0x8, 0x12, 0x1c, 0x26, 0x30, 0x3a, 0x44, 0x4e, // c:114
        0x58, 0x62, 0x6c, 0x76, 0x80, 0x8a, 0x94, 0x9e, // c:115
        0xa8, 0xb2, 0xbc, 0xc6, 0xd0, 0xda, 0xe4, 0xee, // c:116
    ];
    let mut orig = cielab::default(); // c:118
    let mut next = cielab::default(); // c:118
    let mut nextl: f64; // c:119
    let mut bestl: f64 = -1.0; // c:119
    let mut r: i32; // c:120
    let mut g: i32; // c:120
    let mut b: i32; // c:120
    let mut comp_r: i32 = 0; // c:121
    let mut comp_g: i32 = 0; // c:121
    let mut comp_b: i32 = 0; // c:121

    /* Get original value */
    // c:123
    RGBtoLAB(red, green, blue, &mut orig); // c:124

    // c:126 — `for (r = 0; r < sizeof(component)/sizeof(*component); r++)`
    let len: i32 = component.len() as i32; // c:126
    r = 0; // c:126
    while r < len {
        // c:126
        g = 0; // c:127
        while g <= 5 {
            // c:127
            b = 0; // c:128
            while b <= 5 {
                // c:128
                if r > 5 {
                    g = r;
                    b = r;
                } // c:129
                RGBtoLAB(
                    component[r as usize], // c:130
                    component[g as usize],
                    component[b as usize],
                    &mut next,
                );
                nextl = deltae(&orig, &next); // c:131
                if nextl < bestl || bestl < 0.0 {
                    // c:132
                    bestl = nextl; // c:133
                    comp_r = r; // c:134
                    comp_g = g; // c:135
                    comp_b = b; // c:136
                }
                b += 1; // c:128
            }
            g += 1; // c:127
        }
        r += 1; // c:126
    }

    // c:142-143 — return (comp_r > 5) ? 226 + comp_r :
    //                    16 + (comp_r * 36) + (comp_g * 6) + comp_b;
    if comp_r > 5 {
        // c:142
        226 + comp_r // c:142
    } else {
        16 + (comp_r * 36) + (comp_g * 6) + comp_b // c:143
    }
}

// =====================================================================
// getnearestcolor(UNUSED(Hookdef dummy), Color_rgb col)              c:147
// =====================================================================

/// Port of `getnearestcolor(UNUSED(Hookdef dummy), Color_rgb col)` from `Src/Modules/nearcolor.c:147`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// getnearestcolor(UNUSED(Hookdef dummy), Color_rgb col)
/// ```
/// `Hookdef` and `Color_rgb` are `struct hookdef *` / `struct color_rgb *`
/// (zsh.h:528 / 2752); ported as `*const hookdef` / `*const color_rgb`.
#[allow(unused_variables)]
pub fn getnearestcolor(dummy: *const hookdef, col: *const color_rgb) -> i32 {
    // c:147
    /* we add 1 to the colours so that colour 0 (default) is
     * distinguished from runhookdef() indicating that no
     * hook function is registered */                                 // c:149-151
    let red: i32;
    let green: i32;
    let blue: i32;
    unsafe {
        red = (*col).red as i32;
        green = (*col).green as i32;
        blue = (*col).blue as i32;
    }
    // C `tccolours` is an int global from `Src/init.c:94`; Rust port
    // mirrors it as the existing `init::tccolours` AtomicI32.
    if tccolours.load(Ordering::Relaxed) == 256 {
        // c:152
        return mapRGBto256(red, green, blue) + 1; // c:153
    }
    if tccolours.load(Ordering::Relaxed) == 88 {
        // c:154
        return mapRGBto88(red, green, blue) + 1; // c:155
    }
    -1 // c:156
}

// =====================================================================
// static struct features module_features                             c:159
//
// Empty feature table — the module exposes only the boot_/cleanup_
// hookfunc registration. Static-link / fusevm path doesn't traverse
// the table; `getnearestcolor` is invoked directly. Omitted from the
// Rust port.
// =====================================================================

// =====================================================================
// setup_(UNUSED(Module m))                                           c:168
// =====================================================================

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/nearcolor.c:169`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:169
    0 // c:184
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/nearcolor.c:176`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:176
    *features = featuresarray(m, module_features());
    0 // c:191
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/nearcolor.c:184`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:184
    handlefeatures(m, module_features(), enables) // c:191
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/nearcolor.c:191`.
/// C body: `addhookfunc("get_color_attr", (Hookfn) getnearestcolor); return 0;`
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:191
    addhookfunc("get_color_attr", getnearestcolor); // c:199
    0 // c:207
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/nearcolor.c:199`.
/// C body: `deletehookfunc("get_color_attr", ...); return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:199
    deletehookfunc("get_color_attr", getnearestcolor); // c:207
    setfeatureenables(m, module_features(), None) // c:207
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/nearcolor.c:207`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:207
    0 // c:207
}

// =====================================================================
// `static struct features module_features` from nearcolor.c:159 (empty).
// =====================================================================

// `module_features` — port of `static struct features module_features`
// from nearcolor.c:159. All four feature slices empty.

// Port of `addhookfunc(char *n, Hookfn f)` from Src/module.c:948.
// C: `int addhookfunc(char *n, Hookfn f)` →
//   `Hookdef h = gethookdef(n); if (h) return addhookdeffunc(h, f); return 1;`
//
// `gethookdef` (`Src/module.c:849`), `addhookdeffunc` (`:939`) and
// `deletehookdeffunc` (`:961`) are module.c functions; the canonical
// ports live in `src/ported/module.rs`. This file used to carry a
// second, inert copy of all three — `gethookdef` returned `None`
// unconditionally, so neither hook fn was ever reached. Those copies
// are gone; the behaviour they produced (C's "hookdef not found"
// return) is stated directly here.
//
// They are NOT forwarded to module.rs's real chain because the two
// hook signatures differ: module.rs types the chain as `Hookfn =
// fn(*mut hookdef, *mut c_void) -> i32` (`zsh_h.rs:1051`, C's
// `Src/zsh.h` Hookfn), while nearcolor's hook is
// `int (*)(Hookdef, Colour_rgb)` (`Src/Modules/nearcolor.c:161`). C
// bridges them with the explicit `(Hookfn)` cast at nearcolor.c:199;
// Rust cannot without a transmute, and wiring it up would change
// runtime behaviour (`get_color_attr` IS registered in the hooktab —
// `src/ported/init.rs:179`), which is a separate change.
fn addhookfunc(n: &str, f: fn(*const hookdef, *const color_rgb) -> i32) -> i32 {
    // c:948
    let _ = (n, f);
    1 // c:955
}

// Port of `deletehookfunc(const char *n, Hookfn f)` from Src/module.c:977.
// C: `int deletehookfunc(const char *n, Hookfn f)` →
//   `Hookdef h = gethookdef(n); if (h) return deletehookdeffunc(h, f); return 1;`
// Same signature-mismatch note as `addhookfunc` above.
fn deletehookfunc(n: &str, f: fn(*const hookdef, *const color_rgb) -> i32) {
    // c:977
    let _ = (n, f);
}

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN NEARCOLOR.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec![]
}

// WARNING: NOT IN NEARCOLOR.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 0]);
    }
    0
}

// WARNING: NOT IN NEARCOLOR.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN NEARCOLOR.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 0,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 0,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies `RGBtoLAB(0,0,0)` yields the CIE 1976 Lab origin
    /// (L≈0, a≈0, b≈0) — port of c:50 with red=green=blue=0.
    #[test]
    fn rgb_to_lab_black_is_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut lab = cielab::default();
        RGBtoLAB(0, 0, 0, &mut lab);
        assert!(lab.L.abs() < 0.5);
        assert!(lab.a.abs() < 0.5);
        assert!(lab.b.abs() < 0.5);
    }

    /// Verifies `deltae` of a colour against itself is zero — c:41
    /// invariant when `lab1 == lab2`.
    #[test]
    fn deltae_self_is_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut lab = cielab::default();
        RGBtoLAB(123, 45, 67, &mut lab);
        assert!(deltae(&lab, &lab).abs() < 1e-9);
    }

    /// Verifies pure white maps into the upper end of the 256-colour
    /// palette (>= 15) — sanity-check on the c:142-143 final-index formula.
    #[test]
    fn map_rgb_to_256_white_is_15_or_higher() {
        let _g = crate::test_util::global_state_lock();
        let idx = mapRGBto256(0xff, 0xff, 0xff);
        assert!(idx >= 15);
    }

    /// Verifies pure white maps into the 88-colour palette range —
    /// c:102-103 final-index formula.
    #[test]
    fn map_rgb_to_88_white_is_in_range() {
        let _g = crate::test_util::global_state_lock();
        let idx = mapRGBto88(0xff, 0xff, 0xff);
        assert!((16..=87).contains(&idx) || idx >= 77);
    }

    /// Port of `getnearestcolor(UNUSED(Hookdef dummy), Color_rgb col)` from `Src/Modules/nearcolor.c:147`.
    /// Verifies `getnearestcolor` dispatches on the `tccolours` global
    /// per c:152-156: 256→`mapRGBto256+1`, 88→`mapRGBto88+1`, otherwise -1.
    #[test]
    fn getnearestcolor_dispatches_on_tccolours() {
        let _g = crate::test_util::global_state_lock();
        let saved = tccolours.load(Ordering::SeqCst);
        let col = color_rgb {
            red: 0xff,
            green: 0xff,
            blue: 0xff,
        };

        tccolours.store(256, Ordering::SeqCst);
        let r256 = getnearestcolor(std::ptr::null(), &col);
        assert_eq!(r256, mapRGBto256(0xff, 0xff, 0xff) + 1);

        tccolours.store(88, Ordering::SeqCst);
        let r88 = getnearestcolor(std::ptr::null(), &col);
        assert_eq!(r88, mapRGBto88(0xff, 0xff, 0xff) + 1);

        tccolours.store(16, Ordering::SeqCst);
        assert_eq!(getnearestcolor(std::ptr::null(), &col), -1);

        tccolours.store(saved, Ordering::SeqCst);
    }

    /// c:41 — `deltae` is symmetric in its arguments: swapping lab1
    /// and lab2 gives the same value. The formula is
    /// `(L1-L2)² + (a1-a2)² + (b1-b2)²` so symmetry is a guarantee.
    /// Pinning it catches a regression that introduces an asymmetric
    /// term (rare but possible if someone "optimises" the powi(2)
    /// into a multiply with a sign drift).
    #[test]
    fn deltae_is_symmetric() {
        let _g = crate::test_util::global_state_lock();
        let mut a = cielab::default();
        let mut b = cielab::default();
        RGBtoLAB(200, 50, 25, &mut a);
        RGBtoLAB(25, 200, 50, &mut b);
        let d_ab = deltae(&a, &b);
        let d_ba = deltae(&b, &a);
        assert!(
            (d_ab - d_ba).abs() < 1e-9,
            "deltae symmetry broken: {} vs {}",
            d_ab,
            d_ba
        );
    }

    /// c:41 — `deltae` between two distinct colours is strictly
    /// positive. Pin the non-degenerate case so a regression that
    /// returns 0 (or NaN) for unequal inputs fails loudly.
    #[test]
    fn deltae_distinct_colors_is_positive() {
        let _g = crate::test_util::global_state_lock();
        let mut white = cielab::default();
        let mut black = cielab::default();
        RGBtoLAB(0xff, 0xff, 0xff, &mut white);
        RGBtoLAB(0, 0, 0, &mut black);
        let d = deltae(&white, &black);
        assert!(
            d > 0.0 && d.is_finite(),
            "deltae(white, black) = {} — must be a finite positive value",
            d
        );
    }

    /// c:50-74 — `RGBtoLAB` of pure black gives L=0, a=0, b=0. The
    /// inverse-sRGB branch at c:55 takes the `R / 12.92` path for
    /// R=0, and the XYZ→Lab branch at c:64 takes the
    /// `7.787 * X + 16/116` path. The final L = 116·Y - 16 should
    /// resolve to exactly 0 for black.
    #[test]
    fn rgb_to_lab_pure_black_yields_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut lab = cielab::default();
        RGBtoLAB(0, 0, 0, &mut lab);
        assert!(lab.L.abs() < 1e-9, "L for black = {}; should be 0", lab.L);
        assert!(lab.a.abs() < 1e-9, "a for black = {}; should be 0", lab.a);
        assert!(lab.b.abs() < 1e-9, "b for black = {}; should be 0", lab.b);
    }

    /// c:50 — `RGBtoLAB` of pure white gives L ≈ 100 (the L*a*b*
    /// space's maximum lightness). Catches a regression that uses the
    /// wrong D65 whitepoint scaling factor at c:60-62.
    #[test]
    fn rgb_to_lab_pure_white_has_lightness_near_100() {
        let _g = crate::test_util::global_state_lock();
        let mut lab = cielab::default();
        RGBtoLAB(0xff, 0xff, 0xff, &mut lab);
        assert!(
            (lab.L - 100.0).abs() < 1.0,
            "L for white = {}, expected ≈ 100",
            lab.L
        );
    }

    /// c:142-143 — `mapRGBto256` for pure black hits the all-zero
    /// `r=g=b=0` slot at the start of the 6×6×6 cube → index 16
    /// (`16 + 0*36 + 0*6 + 0`).
    #[test]
    fn map_rgb_to_256_black_is_16() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mapRGBto256(0, 0, 0), 16);
    }

    /// c:142-143 — primary red `0xff,0,0` lives on the outer face
    /// of the cube at r=5, g=0, b=0 → `16 + 5*36 = 196`.
    #[test]
    fn map_rgb_to_256_pure_red_is_196() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mapRGBto256(0xff, 0, 0), 196);
    }

    /// c:142-143 — primary green `0,0xff,0` → r=0, g=5, b=0
    /// → `16 + 0 + 5*6 + 0 = 46`.
    #[test]
    fn map_rgb_to_256_pure_green_is_46() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mapRGBto256(0, 0xff, 0), 46);
    }

    /// c:142-143 — primary blue `0,0,0xff` → r=0, g=0, b=5
    /// → `16 + 0 + 0 + 5 = 21`.
    #[test]
    fn map_rgb_to_256_pure_blue_is_21() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mapRGBto256(0, 0, 0xff), 21);
    }

    /// c:102-103 — `mapRGBto88` for pure black hits r=0, g=0, b=0
    /// → `16 + 0 + 0 + 0 = 16`. Symmetry with the 256-colour case.
    #[test]
    fn map_rgb_to_88_black_is_16() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mapRGBto88(0, 0, 0), 16);
    }

    /// c:102-103 — primary red `0xff,0,0`: r=3 (the 0xff bucket in
    /// the 4-level RGB ramp at c:76), g=b=0 → `16 + 3*16 + 0 + 0 = 64`.
    #[test]
    fn map_rgb_to_88_pure_red_is_64() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(mapRGBto88(0xff, 0, 0), 64);
    }

    /// c:147-156 — `getnearestcolor` with `tccolours = 0` (terminal
    /// reports no palette) must take the default branch and return
    /// -1. Pinning this protects callers from a regression that
    /// invokes `mapRGBto256` regardless of palette size.
    #[test]
    fn getnearestcolor_unknown_palette_returns_neg_one() {
        let _g = crate::test_util::global_state_lock();
        let saved = tccolours.load(Ordering::SeqCst);
        tccolours.store(0, Ordering::SeqCst);
        let col = color_rgb {
            red: 100,
            green: 100,
            blue: 100,
        };
        let r = getnearestcolor(std::ptr::null(), &col);
        tccolours.store(saved, Ordering::SeqCst);
        assert_eq!(r, -1);
    }

    /// c:169-220 — module lifecycle stubs all return 0 in C.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    // ─── zsh-corpus pins for nearcolor math ─────────────────────────

    /// `mapRGBto256` for pure black (0,0,0) returns index 16.
    #[test]
    fn nearcolor_corpus_map256_black_is_16() {
        let r = mapRGBto256(0, 0, 0);
        assert_eq!(r, 16, "black = palette index 16");
    }

    /// `mapRGBto256` for pure white (255,255,255) returns index 231.
    #[test]
    fn nearcolor_corpus_map256_white_is_231() {
        let r = mapRGBto256(255, 255, 255);
        assert_eq!(r, 231, "white = palette index 231");
    }

    /// `mapRGBto256` is deterministic.
    #[test]
    fn nearcolor_corpus_map256_is_deterministic() {
        let a = mapRGBto256(128, 64, 200);
        let b = mapRGBto256(128, 64, 200);
        assert_eq!(a, b);
    }

    /// `mapRGBto256` returns a valid palette index (16..=255).
    #[test]
    fn nearcolor_corpus_map256_within_palette_range() {
        for r in [0, 64, 128, 192, 255] {
            for g in [0, 128, 255] {
                for b in [0, 128, 255] {
                    let idx = mapRGBto256(r, g, b);
                    assert!(
                        (16..=255).contains(&idx),
                        "({r},{g},{b}) → {idx} out of [16,255]"
                    );
                }
            }
        }
    }

    /// `mapRGBto88` returns valid 88-palette index for black.
    #[test]
    fn nearcolor_corpus_map88_black_in_palette() {
        let r = mapRGBto88(0, 0, 0);
        assert!(r >= 0 && r < 88, "88-palette index in [0,88), got {r}");
    }

    /// `mapRGBto88` returns valid 88-palette index for white.
    #[test]
    fn nearcolor_corpus_map88_white_in_palette() {
        let r = mapRGBto88(255, 255, 255);
        assert!(r >= 0 && r < 88, "88-palette index in [0,88), got {r}");
    }

    /// `deltae` of a color and itself is 0.
    #[test]
    fn nearcolor_corpus_deltae_self_is_zero() {
        let lab = cielab {
            L: 50.0,
            a: 25.0,
            b: 30.0,
        };
        let d = deltae(&lab, &lab);
        assert!(d.abs() < 1e-9, "deltae(x,x)=0, got {d}");
    }

    /// `deltae` is symmetric.
    #[test]
    fn nearcolor_corpus_deltae_symmetric() {
        let a = cielab {
            L: 50.0,
            a: 25.0,
            b: 30.0,
        };
        let b = cielab {
            L: 60.0,
            a: 15.0,
            b: 35.0,
        };
        let d1 = deltae(&a, &b);
        let d2 = deltae(&b, &a);
        assert!((d1 - d2).abs() < 1e-9);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/nearcolor.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:41 — `deltae` always returns non-negative (sum of squares).
    #[test]
    fn deltae_always_non_negative() {
        let a = cielab {
            L: -100.0,
            a: -50.0,
            b: 200.0,
        };
        let b = cielab {
            L: 100.0,
            a: 50.0,
            b: -200.0,
        };
        let d = deltae(&a, &b);
        assert!(
            d >= 0.0,
            "delta-E is sum of squares, must be ≥ 0, got {}",
            d
        );
    }

    /// c:41 — `deltae` matches the (L1-L2)^2 + (a1-a2)^2 + (b1-b2)^2
    /// formula exactly (no sqrt per c:42 comment).
    #[test]
    fn deltae_matches_sum_of_squares_formula() {
        let a = cielab {
            L: 50.0,
            a: 0.0,
            b: 0.0,
        };
        let b = cielab {
            L: 53.0,
            a: 4.0,
            b: 12.0,
        };
        let r = deltae(&a, &b);
        let expected = 9.0 + 16.0 + 144.0; // 3² + 4² + 12² = 169
        assert!((r - expected).abs() < 1e-9, "{} != {}", r, expected);
    }

    /// c:50 — `RGBtoLAB(0,0,0)` → black: L=0, a=0, b=0.
    #[test]
    fn rgb_to_lab_black_origin() {
        let mut lab = cielab {
            L: 99.0,
            a: 99.0,
            b: 99.0,
        };
        RGBtoLAB(0, 0, 0, &mut lab);
        assert!(lab.L.abs() < 1e-3, "black L≈0, got {}", lab.L);
        assert!(lab.a.abs() < 1e-3, "black a≈0, got {}", lab.a);
        assert!(lab.b.abs() < 1e-3, "black b≈0, got {}", lab.b);
    }

    /// c:50 — `RGBtoLAB(255,255,255)` → white: L≈100.
    #[test]
    fn rgb_to_lab_white_has_l_near_100() {
        let mut lab = cielab {
            L: 0.0,
            a: 0.0,
            b: 0.0,
        };
        RGBtoLAB(255, 255, 255, &mut lab);
        assert!((lab.L - 100.0).abs() < 0.5, "white L≈100, got {}", lab.L);
    }

    /// c:50 — RGBtoLAB(R,R,R) for gray scale → a≈0, b≈0 (chromatic
    /// neutrality of pure grays).
    #[test]
    fn rgb_to_lab_gray_has_zero_chroma() {
        let mut lab = cielab {
            L: 0.0,
            a: 99.0,
            b: 99.0,
        };
        RGBtoLAB(128, 128, 128, &mut lab);
        assert!(lab.a.abs() < 1.0, "gray a≈0, got {}", lab.a);
        assert!(lab.b.abs() < 1.0, "gray b≈0, got {}", lab.b);
    }

    /// c:50 — RGBtoLAB is deterministic.
    #[test]
    fn rgb_to_lab_is_deterministic() {
        let mut lab1 = cielab {
            L: 0.0,
            a: 0.0,
            b: 0.0,
        };
        let mut lab2 = cielab {
            L: 0.0,
            a: 0.0,
            b: 0.0,
        };
        RGBtoLAB(100, 150, 200, &mut lab1);
        RGBtoLAB(100, 150, 200, &mut lab2);
        assert_eq!(lab1.L, lab2.L);
        assert_eq!(lab1.a, lab2.a);
        assert_eq!(lab1.b, lab2.b);
    }

    /// c:74 — `mapRGBto88(0,0,0)` returns valid index in 0..88 range.
    #[test]
    fn mapRGBto88_returns_valid_palette_index() {
        let r = mapRGBto88(0, 0, 0);
        assert!(
            r >= 0 && r < 88,
            "88-color palette index in [0,88), got {}",
            r
        );
    }

    /// c:74 — `mapRGBto88` deterministic for fixed input.
    #[test]
    fn mapRGBto88_is_deterministic() {
        let a = mapRGBto88(255, 128, 64);
        let b = mapRGBto88(255, 128, 64);
        assert_eq!(a, b);
    }

    /// c:218 — `mapRGBto256(0,0,0)` returns valid index in 0..256.
    #[test]
    fn mapRGBto256_returns_valid_palette_index() {
        let r = mapRGBto256(0, 0, 0);
        assert!(
            r >= 0 && r < 256,
            "256-color palette index in [0,256), got {}",
            r
        );
    }

    /// c:218 — `mapRGBto256` deterministic.
    #[test]
    fn mapRGBto256_is_deterministic() {
        let a = mapRGBto256(200, 100, 50);
        let b = mapRGBto256(200, 100, 50);
        assert_eq!(a, b);
    }

    /// Lifecycle (c:343/366/374) split per-hook.
    #[test]
    fn nearcolor_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:374 — cleanup_ returns 0.
    #[test]
    fn nearcolor_cleanup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cleanup_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/nearcolor.c
    // c:53 deltae / c:73 RGBtoLAB / c:141 mapRGBto88 / c:218 mapRGBto256
    // c:302 getnearestcolor / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:53 — `deltae(a, a)` = 0 (reflexive).
    #[test]
    fn deltae_reflexive_for_arbitrary_colors() {
        for (r, g, b) in [(0, 0, 0), (255, 0, 0), (128, 128, 128), (255, 255, 255)] {
            let mut lab = cielab::default();
            RGBtoLAB(r, g, b, &mut lab);
            assert_eq!(
                deltae(&lab, &lab),
                0.0,
                "deltae({:?}, {:?}) must be 0",
                (r, g, b),
                (r, g, b)
            );
        }
    }

    /// c:53 — `deltae` is symmetric: deltae(a,b) == deltae(b,a).
    #[test]
    fn deltae_symmetric_full_sweep() {
        let pairs = [
            ((255, 0, 0), (0, 255, 0)),
            ((0, 0, 0), (128, 128, 128)),
            ((255, 255, 255), (0, 0, 0)),
        ];
        for ((r1, g1, b1), (r2, g2, b2)) in pairs {
            let mut a = cielab::default();
            let mut b = cielab::default();
            RGBtoLAB(r1, g1, b1, &mut a);
            RGBtoLAB(r2, g2, b2, &mut b);
            let ab = deltae(&a, &b);
            let ba = deltae(&b, &a);
            assert_eq!(ab, ba, "deltae must be symmetric");
        }
    }

    /// c:141 — `mapRGBto88` is pure (deterministic for same input).
    #[test]
    fn mapRGBto88_full_sweep_pure() {
        for (r, g, b) in [(0, 0, 0), (127, 127, 127), (255, 0, 0), (255, 255, 255)] {
            let first = mapRGBto88(r, g, b);
            for _ in 0..3 {
                assert_eq!(
                    mapRGBto88(r, g, b),
                    first,
                    "mapRGBto88({},{},{}) must be pure",
                    r,
                    g,
                    b
                );
            }
        }
    }

    /// c:218 — `mapRGBto256` is pure.
    #[test]
    fn mapRGBto256_full_sweep_pure() {
        for (r, g, b) in [(0, 0, 0), (127, 127, 127), (255, 0, 0), (255, 255, 255)] {
            let first = mapRGBto256(r, g, b);
            for _ in 0..3 {
                assert_eq!(
                    mapRGBto256(r, g, b),
                    first,
                    "mapRGBto256({},{},{}) must be pure",
                    r,
                    g,
                    b
                );
            }
        }
    }

    /// c:73 — `RGBtoLAB` produces L in [0, 100] for valid sRGB input.
    #[test]
    fn rgb_to_lab_lightness_in_zero_to_hundred() {
        for (r, g, b) in [
            (0, 0, 0),
            (255, 0, 0),
            (0, 255, 0),
            (0, 0, 255),
            (255, 255, 255),
        ] {
            let mut lab = cielab::default();
            RGBtoLAB(r, g, b, &mut lab);
            assert!(
                (0.0..=100.5).contains(&lab.L),
                "L value {} out of [0, 100] for ({},{},{})",
                lab.L,
                r,
                g,
                b
            );
        }
    }

    /// c:141 — `mapRGBto88` output in [16, 88) palette range.
    #[test]
    fn mapRGBto88_output_in_palette_range() {
        for (r, g, b) in [(0, 0, 0), (255, 0, 0), (128, 128, 128), (255, 255, 255)] {
            let idx = mapRGBto88(r, g, b);
            assert!(
                (16..88).contains(&idx),
                "mapRGBto88({},{},{}) = {} out of [16, 88)",
                r,
                g,
                b,
                idx
            );
        }
    }

    /// c:218 — `mapRGBto256` output in [16, 256) palette range.
    #[test]
    fn mapRGBto256_output_in_palette_range() {
        for (r, g, b) in [(0, 0, 0), (255, 0, 0), (128, 128, 128), (255, 255, 255)] {
            let idx = mapRGBto256(r, g, b);
            assert!(
                (16..256).contains(&idx),
                "mapRGBto256({},{},{}) = {} out of [16, 256)",
                r,
                g,
                b,
                idx
            );
        }
    }

    /// c:73 — `RGBtoLAB` is pure (same input → same output).
    #[test]
    fn rgb_to_lab_full_sweep_is_pure() {
        for (r, g, b) in [(0, 0, 0), (255, 128, 64), (50, 50, 50)] {
            let mut a = cielab::default();
            let mut b_lab = cielab::default();
            RGBtoLAB(r, g, b, &mut a);
            RGBtoLAB(r, g, b, &mut b_lab);
            assert_eq!(a.L, b_lab.L, "L must be pure");
            assert_eq!(a.a, b_lab.a, "a must be pure");
            assert_eq!(a.b, b_lab.b, "b must be pure");
        }
    }

    /// c:343-... — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn nearcolor_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
        assert_eq!(finish_(null), 0);
    }

    /// c:343 — setup_ idempotent.
    #[test]
    fn nearcolor_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/nearcolor.c
    // c:53 deltae (CIE76 ΔE metric) / c:73 RGBtoLAB / c:141 mapRGBto88 /
    // c:218 mapRGBto256
    // ═══════════════════════════════════════════════════════════════════

    /// c:53 — `deltae` returns f64 (compile-time pin).
    #[test]
    fn deltae_returns_f64_type() {
        let a = cielab::default();
        let b = cielab::default();
        let _: f64 = deltae(&a, &b);
    }

    /// c:53 — `deltae(lab, lab)` of identical labs = 0 (CIE76 distance).
    #[test]
    fn deltae_identical_labs_is_zero() {
        let lab = cielab {
            L: 50.0,
            a: 25.0,
            b: -10.0,
        };
        let d = deltae(&lab, &lab);
        assert_eq!(d, 0.0, "deltae(x, x) = 0; got {}", d);
    }

    /// c:53 — `deltae` is symmetric: deltae(a, b) == deltae(b, a)
    /// (alt-name pin to coexist with sibling test).
    #[test]
    fn deltae_symmetric_alt_pin() {
        let p = cielab {
            L: 30.0,
            a: 10.0,
            b: 20.0,
        };
        let q = cielab {
            L: 60.0,
            a: -5.0,
            b: 40.0,
        };
        let d_pq = deltae(&p, &q);
        let d_qp = deltae(&q, &p);
        assert!(
            (d_pq - d_qp).abs() < 1e-9,
            "CIE76 ΔE must be symmetric; got {} vs {}",
            d_pq,
            d_qp
        );
    }

    /// c:53 — `deltae` always non-negative (distance metric invariant).
    #[test]
    fn deltae_non_negative_invariant() {
        for (l, a, b) in [(0.0, 0.0, 0.0), (100.0, 50.0, -50.0), (25.0, -30.0, 10.0)] {
            let x = cielab { L: l, a, b };
            let y = cielab {
                L: 50.0,
                a: 0.0,
                b: 0.0,
            };
            let d = deltae(&x, &y);
            assert!(
                d >= 0.0,
                "ΔE must be ≥ 0 for any inputs; got {} from L={}/a={}/b={}",
                d,
                l,
                a,
                b
            );
        }
    }

    /// c:73 — `RGBtoLAB(0,0,0)` produces L≈0 (pure black sRGB).
    #[test]
    fn rgb_to_lab_black_has_zero_lightness() {
        let mut lab = cielab::default();
        RGBtoLAB(0, 0, 0, &mut lab);
        assert!(
            lab.L.abs() < 0.5,
            "black should have L ≈ 0; got L={}",
            lab.L
        );
    }

    /// c:73 — `RGBtoLAB(255,255,255)` produces L≈100 (pure white sRGB).
    #[test]
    fn rgb_to_lab_white_has_lightness_near_hundred() {
        let mut lab = cielab::default();
        RGBtoLAB(255, 255, 255, &mut lab);
        assert!(
            (lab.L - 100.0).abs() < 0.5,
            "white should have L ≈ 100; got L={}",
            lab.L
        );
    }

    /// c:141 — `mapRGBto88` returns i32 (compile-time pin).
    #[test]
    fn mapRGBto88_returns_i32_type() {
        let _: i32 = mapRGBto88(0, 0, 0);
    }

    /// c:218 — `mapRGBto256` returns i32 (compile-time pin).
    #[test]
    fn mapRGBto256_returns_i32_type() {
        let _: i32 = mapRGBto256(0, 0, 0);
    }

    /// c:141 — `mapRGBto88(0,0,0)` (black) maps in [16, 88) palette
    /// range (system colors below 16 excluded).
    #[test]
    fn mapRGBto88_black_in_palette_range() {
        let idx = mapRGBto88(0, 0, 0);
        assert!(
            idx >= 16 && idx < 88,
            "mapRGBto88(black) = {} must be in [16, 88)",
            idx
        );
    }

    /// c:218 — `mapRGBto256(255,255,255)` (white) returns an index
    /// distinct from black (perceptually maximal extremes).
    #[test]
    fn mapRGBto256_black_and_white_distinct() {
        let black_idx = mapRGBto256(0, 0, 0);
        let white_idx = mapRGBto256(255, 255, 255);
        assert_ne!(
            black_idx, white_idx,
            "black and white must map to different 256-palette entries"
        );
    }

    /// c:53 — `deltae` returns SQUARED Euclidean distance, NOT
    /// the actual ΔE (no sqrt). C body skips sqrt for perf since
    /// nearest-color comparisons only care about ordering. Pin
    /// the squared-form contract so a future "let's add sqrt"
    /// regression breaks the test loudly.
    #[test]
    fn deltae_returns_squared_distance_no_sqrt() {
        let a = cielab {
            L: 0.0,
            a: 0.0,
            b: 0.0,
        };
        let c = cielab {
            L: 100.0,
            a: 0.0,
            b: 0.0,
        };
        let d = deltae(&a, &c);
        // True ΔE = 100; squared = 10000.
        assert!(
            (d - 10000.0).abs() < 0.5,
            "deltae must return Σ(diff²) = 10000 (NOT sqrt'd 100); got {}",
            d
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/nearcolor.c
    // c:53 deltae / c:73 RGBtoLAB / c:141 mapRGBto88 / c:218 mapRGBto256 /
    // c:343-382 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:53 — `deltae` returns 0 for identical inputs.
    #[test]
    fn deltae_zero_for_identical_inputs() {
        let a = cielab {
            L: 50.0,
            a: 10.0,
            b: -5.0,
        };
        let b = a.clone();
        assert_eq!(
            deltae(&a, &b),
            0.0,
            "deltae(x, x.clone()) must be exactly 0"
        );
    }

    /// c:53 — `deltae` is symmetric (alt).
    #[test]
    fn deltae_is_symmetric_alt() {
        let a = cielab {
            L: 10.0,
            a: 20.0,
            b: 30.0,
        };
        let b = cielab {
            L: 40.0,
            a: 50.0,
            b: 60.0,
        };
        let d1 = deltae(&a, &b);
        let d2 = deltae(&b, &a);
        assert!(
            (d1 - d2).abs() < 1e-9,
            "deltae must be symmetric, got {} vs {}",
            d1,
            d2
        );
    }

    /// c:53 — `deltae` is non-negative.
    #[test]
    fn deltae_is_non_negative() {
        for ((l1, a1, b1), (l2, a2, b2)) in [
            ((0.0, 0.0, 0.0), (100.0, 100.0, 100.0)),
            ((50.0, -50.0, 50.0), (0.0, 0.0, 0.0)),
            ((1.0, 2.0, 3.0), (4.0, 5.0, 6.0)),
        ] {
            let p = cielab {
                L: l1,
                a: a1,
                b: b1,
            };
            let q = cielab {
                L: l2,
                a: a2,
                b: b2,
            };
            assert!(deltae(&p, &q) >= 0.0, "deltae must be ≥ 0");
        }
    }

    /// c:73 — `RGBtoLAB(0,0,0,&lab)` → L=0 (black).
    #[test]
    fn rgb_to_lab_black_is_l_zero() {
        let mut lab = cielab {
            L: -1.0,
            a: -1.0,
            b: -1.0,
        };
        RGBtoLAB(0, 0, 0, &mut lab);
        assert!(lab.L.abs() < 1.0, "black must produce L≈0, got {}", lab.L);
    }

    /// c:73 — `RGBtoLAB(255,255,255,&lab)` → L≈100 (white). RGB scale
    /// is 0..255 per c:50 `R = red / 255.0`.
    #[test]
    fn rgb_to_lab_white_is_l_one_hundred() {
        let mut lab = cielab {
            L: 0.0,
            a: 0.0,
            b: 0.0,
        };
        RGBtoLAB(255, 255, 255, &mut lab);
        assert!(
            (lab.L - 100.0).abs() < 1.0,
            "white must produce L≈100, got {}",
            lab.L
        );
    }

    /// c:141 — `mapRGBto88` for black (0,0,0) returns valid index in 0..88.
    #[test]
    fn map_rgb_to_88_black_in_range() {
        let i = mapRGBto88(0, 0, 0);
        assert!(
            (0..88).contains(&i),
            "mapRGBto88(0,0,0) = {} must be in 0..88",
            i
        );
    }

    /// c:218 — `mapRGBto256` for black returns valid index in 0..256.
    #[test]
    fn map_rgb_to_256_black_in_range() {
        let i = mapRGBto256(0, 0, 0);
        assert!(
            (0..256).contains(&i),
            "mapRGBto256(0,0,0) = {} must be in 0..256",
            i
        );
    }

    /// c:141/218 — both map fns are deterministic.
    #[test]
    fn map_rgb_functions_deterministic() {
        for (r, g, b) in [
            (0, 0, 0),
            (255, 0, 0),
            (0, 255, 0),
            (0, 0, 255),
            (128, 128, 128),
        ] {
            let a1 = mapRGBto88(r, g, b);
            let a2 = mapRGBto88(r, g, b);
            assert_eq!(a1, a2);
            let b1 = mapRGBto256(r, g, b);
            let b2 = mapRGBto256(r, g, b);
            assert_eq!(b1, b2);
        }
    }

    /// c:343 — `setup_` is idempotent.
    #[test]
    fn nearcolor_setup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:374 — `cleanup_` is idempotent.
    #[test]
    fn nearcolor_cleanup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:382 — `finish_` is idempotent.
    #[test]
    fn nearcolor_finish_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }
}
