//! Port of `_signals` from `Completion/Unix/Type/_signals`.
//!
//! Full upstream body (48 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # -a all signals;  -p needs `-' prefix;  -s SIG prefix allowed
//! sh:11  local expl minus pre sigs
//! sh:12  local first last
//! sh:14  zparseopts -D -K -E 'p=minus' 'a=last' 's=pre'
//! sh:15  if [[ -z "$last" ]]; then first=2; last=-3; else first=1; last=-1; fi
//! sh:22  [[ -n "$minus" ]] && minus='-'
//! sh:24  [[ "$1" = -(|-) ]] && shift
//! sh:26  if [[ -z "$minus" ]] ||
//! sh:27     ! zstyle -T ":completion:${curcontext}:signals" prefix-needed ||
//! sh:28     [[ -prefix -* ]]; then
//! sh:29    if zstyle -t …:signals prefix-hidden; then
//! sh:30      tmp=( "${(@)signals[first,last]}" );  disp=(-d tmp)
//! sh:32    else disp=(); fi
//! sh:38    if [[ -n "$pre" && $PREFIX = ${minus}S* ]]; then
//! sh:39      sigs=( "${minus}SIG${(@)^${(@)signals[first,last]:#<->}}" )
//! sh:40      (( $#disp )) && tmp=( "$tmp[@]" "${(@)signals[first,last]}" )
//! sh:38    else sigs=(); fi
//! sh:45    _wanted signals expl signal \
//! sh:46      compadd "$@" "$disp[@]" -M 'm:{a-z}={A-Z}' - \
//! sh:47              "${minus}${(@)^signals[first,last]}" "$sigs[@]"
//! sh:48  fi
//! ```
//!
//! sh:47 `"${minus}${(@)^signals[first,last]}"` is the plan9 cross-product
//! that prefixes EVERY signal name with `minus` — ported by iterating the
//! signal slice and prepending `minus` per element (not shell expansion).

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};

/// zsh `zstyle -t` — true only when explicitly set to a true value.
fn zstyle_t(ctx: &str, style: &str) -> bool {
    matches!(
        lookupstyle(ctx, style).first().map(|s| s.as_str()),
        Some("yes") | Some("true") | Some("on") | Some("1")
    )
}
/// zsh `zstyle -T` — true when unset OR set true; false only when set false.
fn zstyle_t_default_true(ctx: &str, style: &str) -> bool {
    !matches!(
        lookupstyle(ctx, style).first().map(|s| s.as_str()),
        Some("no") | Some("false") | Some("off") | Some("0")
    )
}

/// sh:15 — resolve a 1-based (possibly negative) zsh slice `[first,last]`
/// of `v` into the concrete elements.
fn slice_1based(v: &[String], first: i32, last: i32) -> Vec<String> {
    let len = v.len() as i32;
    let resolve = |i: i32| -> i32 {
        if i > 0 {
            i
        } else {
            len + i + 1
        }
    };
    let s = resolve(first).clamp(1, len.max(1));
    let e = resolve(last).clamp(0, len);
    if s > e || len == 0 {
        return Vec::new();
    }
    v[(s - 1) as usize..e as usize].to_vec()
}

/// `_signals` — complete signal names (optionally `-`/`SIG` prefixed).
pub fn _signals(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_signals");
    // sh:14  zparseopts -D -K -E 'p=minus' 'a=last' 's=pre'
    let has_p = args.iter().any(|a| a == "-p");
    let has_a = args.iter().any(|a| a == "-a");
    let has_s = args.iter().any(|a| a == "-s");
    let mut rest: Vec<String> = args
        .iter()
        .filter(|a| !matches!(a.as_str(), "-p" | "-a" | "-s"))
        .cloned()
        .collect();

    // sh:15 — index range (skip EXIT + trailing pseudo-signals unless -a).
    let (first, last) = if !has_a { (2, -3) } else { (1, -1) };
    // sh:22
    let minus = if has_p { "-" } else { "" };
    // sh:22  drop a leading bare `-` / `--`.
    if matches!(rest.first().map(|s| s.as_str()), Some("-") | Some("--")) {
        rest.remove(0);
    }

    let curcontext = getsparam("curcontext").unwrap_or_default();
    let sig_ctx = format!(":completion:{}:signals", curcontext);
    let prefix = getsparam("PREFIX").unwrap_or_default();

    // sh:26-28 — gate.
    let proceed = minus.is_empty()
        || !zstyle_t_default_true(&sig_ctx, "prefix-needed")
        || prefix.starts_with('-');
    if !proceed {
        return 1;
    }

    // sh:16 — the live `$signals` special array.
    let signals = getaparam("signals").unwrap_or_default();
    let range = slice_1based(&signals, first, last);

    // sh:29-32 — prefix-hidden display strings (unprefixed names).
    let mut disp_flag: Vec<String> = Vec::new();
    let mut disp_list: Vec<String> = Vec::new();
    if zstyle_t(&sig_ctx, "prefix-hidden") {
        disp_list = range.clone();
        disp_flag = vec!["-d".to_string(), "_signals_disp".to_string()];
    }

    // sh:35-38 — optional `SIG`-prefixed variants (numeric names excluded).
    let sigs: Vec<String> = if has_s && prefix.starts_with(&format!("{}S", minus)) {
        let mut v: Vec<String> = range
            .iter()
            .filter(|s| !(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())))
            .map(|s| format!("{}SIG{}", minus, s))
            .collect();
        // sh:40 — when displaying, also show the plain names for the SIG set.
        if !disp_flag.is_empty() {
            disp_list.extend(range.clone());
        }
        v
    } else {
        Vec::new()
    };
    if !disp_flag.is_empty() {
        setaparam("_signals_disp", disp_list);
    }

    // sh:47 — prefix EVERY signal name with `minus` (plan9 cross-product).
    let prefixed: Vec<String> = range.iter().map(|s| format!("{}{}", minus, s)).collect();

    // sh:45-47  _wanted signals expl signal compadd "$@" "$disp[@]"
    //   -M 'm:{a-z}={A-Z}' - <prefixed> <sigs>
    let mut wanted_argv: Vec<String> = vec![
        "signals".to_string(),
        "expl".to_string(),
        "signal".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(rest);
    wanted_argv.extend(disp_flag);
    wanted_argv.push("-M".to_string());
    wanted_argv.push("m:{a-z}={A-Z}".to_string());
    wanted_argv.push("-".to_string());
    wanted_argv.extend(prefixed);
    wanted_argv.extend(sigs);
    _wanted(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_skips_exit_and_trailing_by_default() {
        // sh:15 — [2,-3] of (EXIT HUP INT QUIT ZERR DEBUG) => HUP INT QUIT.
        let v: Vec<String> = ["EXIT", "HUP", "INT", "QUIT", "ZERR", "DEBUG"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(slice_1based(&v, 2, -3), vec!["HUP", "INT", "QUIT"]);
        // -a => whole array.
        assert_eq!(slice_1based(&v, 1, -1).len(), 6);
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        // No executor/tags registered => _wanted returns 1.
        assert_eq!(_signals(&[]), 1);
    }
}
