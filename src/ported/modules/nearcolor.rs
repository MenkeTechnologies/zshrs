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

use std::sync::atomic::Ordering;

use crate::ported::init::tccolours;
use crate::ported::zsh_h::{color_rgb, hookdef, module};

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
fn addhookfunc(n: &str, f: fn(*const hookdef, *const color_rgb) -> i32) -> i32 {
    // c:948
    // c:948 — `Hookdef h = gethookdef(n);`
    let h = gethookdef(n);
    if let Some(h) = h {
        // c:953
        return addhookdeffunc(h, f); // c:954
    }
    1 // c:955
}

// Port of `deletehookfunc(const char *n, Hookfn f)` from Src/module.c:977.
// C: `int deletehookfunc(const char *n, Hookfn f)` →
//   `Hookdef h = gethookdef(n); if (h) return deletehookdeffunc(h, f); return 1;`
fn deletehookfunc(n: &str, f: fn(*const hookdef, *const color_rgb) -> i32) {
    // c:977
    let h = gethookdef(n); // c:977
    if let Some(h) = h {
        // c:982
        let _ = deletehookdeffunc(h, f); // c:983
    }
}

// Port of `gethookdef(const char *n)` from Src/module.c:849 — looks up a Hookdef by
// name in the static-link `HOOKDEFS` registry.
/// WARNING: param names don't match C — Rust=(_n) vs C=(funcs, NULL)
fn gethookdef(_n: &str) -> Option<*const hookdef> {
    // c:849
    // Static-link path: hookdefs registry lives in src/ported/module.rs;
    // until that exposes a typed lookup, return None.
    None
}

// Port of `addhookdeffunc(Hookdef h, Hookfn f)` from Src/module.c:939.
// C: `int addhookdeffunc(Hookdef h, Hookfn f)` →
//   `addlinknode(h->funcs, (void *)f); return 0;`
#[allow(unused_variables)]
fn addhookdeffunc(h: *const hookdef, f: fn(*const hookdef, *const color_rgb) -> i32) -> i32 {
    // c:939
    // c:961 — addlinknode(h->funcs, f). Static-link path: registry is static.
    0 // c:961
}

// Port of `deletehookdeffunc(Hookdef h, Hookfn f)` from Src/module.c:961.
// C: `int deletehookdeffunc(Hookdef h, Hookfn f)` — walk h->funcs,
// remove the matching entry; returns 0 on success, 1 if not found.
/// WARNING: param names don't match C — Rust=(_h, _f) vs C=()
fn deletehookdeffunc(_h: *const hookdef, _f: fn(*const hookdef, *const color_rgb) -> i32) -> i32 {
    // c:961
    // c:966-971 — walks h->funcs list; static-link path: nothing to remove.
    1 // c:972
}

use std::sync::{Mutex, OnceLock};

static MODULE_FEATURES: OnceLock<Mutex<crate::ported::zsh_h::features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN NEARCOLOR.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<crate::ported::zsh_h::features>) -> Vec<String> {
    vec![]
}

// WARNING: NOT IN NEARCOLOR.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 0]);
    }
    0
}

// WARNING: NOT IN NEARCOLOR.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<crate::ported::zsh_h::features>, _e: Option<&[i32]>) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN NEARCOLOR.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<crate::ported::zsh_h::features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(crate::ported::zsh_h::features {
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
}
