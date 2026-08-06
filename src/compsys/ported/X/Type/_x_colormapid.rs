//! Port of `_x_colormapid` from `Completion/X/Type/_x_colormapid`.
//!
//! Full upstream body (17 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl list desc
//! sh: 5  _tags colormapids || return 1
//! sh: 7  list=(${(f)"$(xprop -root -f RGB_COLOR_MAP 32xcccccccxx ': $0\n'|awk -F'[ ():]' '/^[a-zA-Z_]+\(RGB_COLOR_MAP\)/ {print $5, "--", $1}')"})
//! sh: 9  if zstyle -T ":completion:${curcontext}:colormap-id" verbose; then
//! sh:10    desc=(-ld list)
//! sh:11  else
//! sh:12    desc=()
//! sh:13  fi
//! sh:15  _wanted colormapids expl 'colormap id' \
//! sh:16      compadd "$@" "$desc[@]" - "${(@)list%% *}"
//! ```
//!
//! sh:7 — the `xprop | awk` pipeline is preserved verbatim (spawned via
//! `sh -c`, backtick/`$(...)`-equivalent) rather than reimplemented in
//! Rust; each output line is already formatted `"<id> -- <name>"` by the
//! awk program. `${(f)...}` then splits that captured text into `list`
//! (one element per line).
//! sh:16 — `${(@)list%% *}` strips everything from the first space
//! onward, leaving just the `<id>` half of each `list` element as the
//! actual completion candidate; the full `"<id> -- <name>"` strings stay
//! in `list` and are handed to compadd as the `-ld` display array (sh:10)
//! when the `colormap-id` style's `verbose` boolean is on (sh:9, default
//! true — `zstyle -T` is false only when explicitly set false).

use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getsparam, setaparam};

#[cfg(test)]
use crate::ported::modules::zutil::bin_zstyle;
#[cfg(test)]
use crate::ported::zsh_h::{options, MAX_OPS};

#[cfg(test)]
fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:7 — the exact upstream pipeline, run through `sh -c` so its
/// quoting/escaping (single-quoted awk program with an embedded `\(`)
/// is interpreted identically to the shell source.
const XPROP_COLORMAP_PIPELINE: &str = r#"xprop -root -f RGB_COLOR_MAP 32xcccccccxx ': $0\n'|awk -F'[ ():]' '/^[a-zA-Z_]+\(RGB_COLOR_MAP\)/ {print $5, "--", $1}'"#;

/// sh:7 `$(...)` capture — spawn the pipeline and return its stdout
/// (empty string on spawn failure, matching zsh command-substitution
/// degradation rather than aborting the script).
fn run_xprop_colormap() -> String {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(XPROP_COLORMAP_PIPELINE)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// zsh `zstyle -T ctx style` — true when unset OR set true; false only
/// when explicitly set to a false value.
fn zstyle_t_default_true(ctx: &str, style: &str) -> bool {
    !matches!(
        lookupstyle(ctx, style).first().map(|s| s.as_str()),
        Some("no") | Some("false") | Some("off") | Some("0")
    )
}

/// sh:16 `${(@)list%% *}` — strip everything from the first space
/// onward.
fn strip_after_first_space(s: &str) -> String {
    match s.find(' ') {
        Some(idx) => s[..idx].to_string(),
        None => s.to_string(),
    }
}

/// `_x_colormapid` — complete X11 colormap IDs discovered via the
/// `RGB_COLOR_MAP`-typed root-window properties.
pub fn _x_colormapid(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_x_colormapid");
    // sh:5
    if _tags(&["colormapids".to_string()]) != 0 {
        return 1;
    }

    // sh:7 — capture pipeline output, ${(f)...} split into lines.
    let raw = run_xprop_colormap();
    let list: Vec<String> = raw.lines().map(String::from).collect();

    // sh:9-13
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:colormap-id", curcontext);
    let mut desc: Vec<String> = Vec::new();
    if zstyle_t_default_true(&ctx, "verbose") {
        setaparam("list", list.clone());
        desc = vec!["-ld".to_string(), "list".to_string()];
    }

    // sh:16 — candidates are the id-only half of each "<id> -- <name>" line.
    let candidates: Vec<String> = list.iter().map(|s| strip_after_first_space(s)).collect();

    // sh:15-16  _wanted colormapids expl 'colormap id' compadd "$@" "$desc[@]" - <candidates>
    let mut w: Vec<String> = vec![
        "colormapids".to_string(),
        "expl".to_string(),
        "colormap id".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.extend(desc);
    w.push("-".to_string());
    w.extend(candidates);
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_after_first_space_keeps_id_only() {
        // sh:16 — "0x210007e -- RGB_COLOR_MAP" -> "0x210007e".
        assert_eq!(
            strip_after_first_space("0x210007e -- RGB_COLOR_MAP"),
            "0x210007e"
        );
    }

    #[test]
    fn strip_after_first_space_no_space_is_noop() {
        assert_eq!(strip_after_first_space("0x210007e"), "0x210007e");
    }

    #[test]
    fn zstyle_t_default_true_is_true_when_unset() {
        let _g = crate::test_util::global_state_lock();
        assert!(zstyle_t_default_true(":completion::colormap-id", "verbose"));
    }

    #[test]
    fn zstyle_t_default_true_is_false_when_explicitly_false() {
        let _g = crate::test_util::global_state_lock();
        let _ = bin_zstyle(
            "zstyle",
            &[
                ":completion:colormap-id-test:colormap-id".to_string(),
                "verbose".to_string(),
                "false".to_string(),
            ],
            &make_ops(),
            0,
        );
        assert!(!zstyle_t_default_true(
            ":completion:colormap-id-test:colormap-id",
            "verbose"
        ));
    }

    #[test]
    fn returns_one_without_registered_tags() {
        // sh:5 — `_tags colormapids || return 1` with no completion
        // context / registered tags configured.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _x_colormapid(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
