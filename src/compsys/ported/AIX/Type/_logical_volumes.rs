//! Port of `_logical_volumes` from `Completion/AIX/Type/_logical_volumes`.
//!
//! Full upstream body (16 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl list names disp sep
//! sh: 5  list=( $(lsvg -l $(lsvg)|sed  -e '2d'|awk '/[^:]* / {if ( $7 != "N/A" ) print $1 ":" $7; else print $1}' ) )
//! sh: 6  names=(${list%%:*})
//! sh: 7  if zstyle -T ":completion:${curcontext}:" verbose; then
//! sh: 8    zstyle -s ":completion:${curcontext}:" list-separator sep || sep=--
//! sh: 9    zformat -a list " $sep " $list
//! sh:10    disp=(-d list)
//! sh:11  else
//! sh:12    disp=()
//! sh:13  fi
//! sh:14  _wanted logicalvolumes expl 'logical volume' \
//! sh:15      compadd "$disp[@]" "$@" - "$names[@]"
//! ```
//!
//! `lsvg` and `lsvg -l <groups>` are run as real subprocesses (no `_call_program`
//! wrapper in the original — it's a bare `$(...)` command substitution), and the
//! `sed -e '2d' | awk '...'` pipeline stage is replicated in pure Rust for
//! testability (mirrors the `_printers.rs` convention of porting `sed`/`awk`
//! text-processing stages as pure functions rather than shelling back out).

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getsparam, setaparam};

/// zsh `zstyle -T ctx style` — true unless explicitly set false.
fn zstyle_t_default_true(ctx: &str, style: &str) -> bool {
    !matches!(
        lookupstyle(ctx, style).first().map(|s| s.as_str()),
        Some("no") | Some("false") | Some("off") | Some("0")
    )
}

/// sh:9 `zformat -a list " $sep " entries` — split each entry on the FIRST
/// `:` into `left:right`, pad every `left` to the widest, then join
/// `left<pad><sep>right`. Entries with no `:` have an empty right part.
fn zformat_align(sep: &str, entries: &[String]) -> Vec<String> {
    let split: Vec<(&str, &str)> = entries
        .iter()
        .map(|e| match e.split_once(':') {
            Some((l, r)) => (l, r),
            None => (e.as_str(), ""),
        })
        .collect();
    let width = split
        .iter()
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0);
    split
        .iter()
        .map(|(l, r)| {
            let pad = width - l.chars().count();
            format!("{}{}{}{}", l, " ".repeat(pad), sep, r)
        })
        .collect()
}

/// sh:6 `${list%%:*}` — strip everything from (and including) the first `:`.
fn names_from_list(list: &[String]) -> Vec<String> {
    list.iter()
        .map(|e| e.split(':').next().unwrap_or(e).to_string())
        .collect()
}

/// `sed -e '2d'` — delete exactly the second line (1-based line 2), keep
/// everything else in order.
fn sed_2d(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 1)
        .map(|(_, l)| l.clone())
        .collect()
}

/// `awk '/[^:]* / {if ( $7 != "N/A" ) print $1 ":" $7; else print $1}'` —
/// the pattern `/[^:]* /` matches any line containing a literal space (the
/// `[^:]*` can match zero-width); for matching lines, split on whitespace
/// (awk default field splitting) and emit `$1:$7` unless `$7` is `N/A`, in
/// which case emit bare `$1`. Awk treats a missing field 7 as `""`.
fn awk_lv_filter(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for line in lines {
        if !line.contains(' ') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let f1 = fields.first().copied().unwrap_or("");
        let f7 = fields.get(6).copied().unwrap_or("");
        if f7 != "N/A" {
            out.push(format!("{}:{}", f1, f7));
        } else {
            out.push(f1.to_string());
        }
    }
    out
}

/// Run `cmd args...` and capture stdout. Returns an empty string on spawn
/// failure (AIX-only commands like `lsvg` won't exist off-AIX; the shell's
/// `$(...)` likewise yields empty output when the command can't run).
fn run_capture(cmd: &str, args: &[String]) -> String {
    match std::process::Command::new(cmd).args(args).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => String::new(),
    }
}

/// sh:5 `list=( $(lsvg -l $(lsvg)|sed -e '2d'|awk '...') )` — enumerate
/// volume groups via bare `lsvg`, then run `lsvg -l <groups...>` and pipe
/// through the `sed`/`awk` stages above.
fn build_list() -> Vec<String> {
    let groups: Vec<String> = run_capture("lsvg", &[])
        .split_whitespace()
        .map(String::from)
        .collect();
    let mut lsvg_l_args = vec!["-l".to_string()];
    lsvg_l_args.extend(groups);
    let raw = run_capture("lsvg", &lsvg_l_args);
    let lines: Vec<String> = raw.lines().map(String::from).collect();
    awk_lv_filter(&sed_2d(&lines))
}

/// `_logical_volumes` — complete AIX logical volume names (optionally with
/// a mount-point display column when the `verbose` style is on).
pub fn _logical_volumes(args: &[String]) -> i32 {
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let bctx = format!(":completion:{}:", curcontext);

    // sh:5
    let list = build_list();
    // sh:6
    let names = names_from_list(&list);

    // sh:7-13
    let mut disp: Vec<String> = Vec::new();
    if zstyle_t_default_true(&bctx, "verbose") {
        // sh:8
        let sep = lookupstyle(&bctx, "list-separator")
            .into_iter()
            .next()
            .unwrap_or_else(|| "--".to_string());
        // sh:9
        setaparam("list", zformat_align(&format!(" {} ", sep), &list));
        // sh:10
        disp = vec!["-d".to_string(), "list".to_string()];
    }

    // sh:14-15  _wanted logicalvolumes expl 'logical volume' \
    //             compadd "$disp[@]" "$@" - "$names[@]"
    let mut w = vec![
        "logicalvolumes".to_string(),
        "expl".to_string(),
        "logical volume".to_string(),
        "compadd".to_string(),
    ];
    w.extend(disp);
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.extend(names);
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sed_2d_removes_only_second_line() {
        let lines = vec![
            "rootvg:".to_string(),
            "LV NAME  TYPE  LPs  PPs  PVs  LV STATE  MOUNT POINT".to_string(),
            "hd5  boot  1  1  1  closed/syncd  N/A".to_string(),
            "hd6  paging  8  8  1  open/syncd  N/A".to_string(),
        ];
        let out = sed_2d(&lines);
        assert_eq!(
            out,
            vec![
                "rootvg:".to_string(),
                "hd5  boot  1  1  1  closed/syncd  N/A".to_string(),
                "hd6  paging  8  8  1  open/syncd  N/A".to_string(),
            ]
        );
    }

    #[test]
    fn sed_2d_noop_on_short_input() {
        let lines = vec!["only".to_string()];
        assert_eq!(sed_2d(&lines), lines);
    }

    #[test]
    fn awk_filter_skips_lines_without_a_space() {
        // "rootvg:" has no space -> pattern /[^:]* / doesn't match -> dropped.
        let lines = vec!["rootvg:".to_string()];
        assert_eq!(awk_lv_filter(&lines), Vec::<String>::new());
    }

    #[test]
    fn awk_filter_emits_bare_name_when_mount_is_na() {
        // fields: 1=hd5 2=boot 3=1 4=1 5=1 6=closed/syncd 7=N/A
        let lines = vec!["hd5 boot 1 1 1 closed/syncd N/A".to_string()];
        assert_eq!(awk_lv_filter(&lines), vec!["hd5".to_string()]);
    }

    #[test]
    fn awk_filter_emits_name_colon_mountpoint_when_not_na() {
        // fields: 1=lv00 2=jfs2 3=1 4=1 5=1 6=open/syncd 7=/home
        let lines = vec!["lv00 jfs2 1 1 1 open/syncd /home".to_string()];
        assert_eq!(awk_lv_filter(&lines), vec!["lv00:/home".to_string()]);
    }

    #[test]
    fn awk_filter_treats_missing_field7_as_empty_not_na() {
        // Only 3 fields; $7 is "" in awk, which != "N/A" -> "$1:" (empty right).
        let lines = vec!["hd8 sparevg 1".to_string()];
        assert_eq!(awk_lv_filter(&lines), vec!["hd8:".to_string()]);
    }

    #[test]
    fn names_from_list_strips_colon_suffix() {
        assert_eq!(
            names_from_list(&["lv00:/home".to_string(), "hd5".to_string()]),
            vec!["lv00".to_string(), "hd5".to_string()]
        );
    }

    #[test]
    fn zformat_aligns_left_column() {
        let out = zformat_align(" -- ", &["a:one".to_string(), "bbb:two".to_string()]);
        assert_eq!(
            out,
            vec!["a   -- one".to_string(), "bbb -- two".to_string()]
        );
        let out2 = zformat_align(" -- ", &["solo".to_string()]);
        assert_eq!(out2, vec!["solo -- ".to_string()]);
    }

    #[test]
    fn build_list_returns_empty_when_lsvg_missing() {
        // `lsvg` doesn't exist off-AIX -> run_capture spawn fails -> empty
        // groups -> `lsvg -l` also fails to spawn -> empty raw -> empty list.
        assert_eq!(build_list(), Vec::<String>::new());
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_logical_volumes(&[]), 1);
    }
}
