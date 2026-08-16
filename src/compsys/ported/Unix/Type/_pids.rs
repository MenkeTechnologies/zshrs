//! Port of `_pids` from `Completion/Unix/Type/_pids`.
//!
//! Full upstream body (60 lines verbatim):
//! ```text
//! sh: 1  #compdef pflags pcred pldd psig pstack pfiles pwdx pstop prun pwait
//! sh: 6  local out pids list expl match desc listargs all nm ret=1
//! sh: 8  _tags processes || return 1
//! sh:10  if [[ "$1" = -m ]]; then
//! sh:11    all=()
//! sh:12    match="(*[[:blank:]]|)${PREFIX}[0-9]#${SUFFIX}[[:blank:]]*(/|[[:blank:]]-(#c,1))${2}([[:blank:]]*|)"
//! sh:13    shift 2
//! sh:14  elif [[ "$PREFIX$SUFFIX" = ([%-]*|[0-9]#) ]]; then
//! sh:15    all=()
//! sh:16    match="(*[[:blank:]]|)${PREFIX}[0-9]#${SUFFIX}[[:blank:]]*"
//! sh:17  else
//! sh:18    all=(-P "$IPREFIX" -S "$ISUFFIX" -U)
//! sh:19    match="*[[:blank:]]*[[/[:blank:]]$PREFIX*$SUFFIX*"
//! sh:20    nm="$compstate[nmatches]"
//! sh:21  fi
//! sh:23  while _tags; do
//! sh:24    if _requested processes; then
//! sh:25      while _next_label processes expl 'process ID'; do
//! sh:26        out=( "${(@f)$(_call_program $curtag ps 2>/dev/null)}" )
//! sh:27        desc="$out[1]"
//! sh:28        out=( "${(@M)out[2,-1]:#${~match}}" )
//! sh:30        if [[ "$desc" = (#i)(|*[[:blank:]])pid(|[[:blank:]]*) ]]; then
//! sh:31          pids=( "${(@)${(@M)out#${(l.${#desc[1,(r)(#i)[[:blank:]]pid]}..?.)~:-}[^[:blank:]]#}##*[[:blank:]]}" )
//! sh:32        else
//! sh:33          pids=( "${(@)${(@M)out##[^0-9]#[0-9]#}##*[[:blank:]]}" )
//! sh:34        fi
//! sh:36        if zstyle -T ":completion:${curcontext}:$curtag" verbose; then
//! sh:37          list=( "${(@Mr:COLUMNS-1:)out}" )
//! sh:38          desc=(-ld list)
//! sh:39        else
//! sh:40          desc=()
//! sh:41        fi
//! sh:42        compadd "$@" "$expl[@]" "$desc[@]" "$all[@]" -a pids && ret=0
//! sh:43      done
//! sh:44    fi
//! sh:45    (( ret )) || break
//! sh:46  done
//! sh:48  if [[ -n "$all" ]]; then
//! sh:49    zstyle -s ":completion:${curcontext}:processes" insert-ids out || out=menu
//! sh:51    case "$out" in
//! sh:52    menu)   compstate[insert]=menu ;;
//! sh:53    single) [[ $compstate[nmatches] -ne nm+1 && $compstate[insert] != menu ]] &&
//! sh:54                compstate[insert]= ;;
//! sh:55    *)      [[ ${#:-$PREFIX$SUFFIX} -gt ${#compstate[unambiguous]} ]] &&
//! sh:56                compstate[insert]=menu ;;
//! sh:57    esac
//! sh:58  fi
//! sh:60  return ret
//! ```
//!
//! `$out` doubles as the `ps` output array (sh:26) and as the
//! `insert-ids` style value (sh:49); the port keeps the two apart as
//! `out` / `insert_ids` because Rust has no untyped parameter.

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_next_label::_next_label;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::shared::{FnScope, LocalScope};
use crate::ported::glob::tokenize;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::pattern::{patcompile, pattry, Patprog};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// Compile a shell pattern the way a `${~var}` / `[[ x = pat ]]` use
/// site does: `tokenize` first (`Src/glob.c:2415`), then
/// `patcompile` (`Src/pattern.c:521`). Every pattern here relies on
/// EXTENDED_GLOB (`[0-9]#`, `(#i)`, `(#c,1)`), which `_comp_setup`
/// has already switched on for the whole completion
/// (`compinit` sh:180-190, `COMP_OPTIONS`).
fn compile_pat(pat: &str) -> Option<Patprog> {
    let mut tok = pat.to_string();
    tokenize(&mut tok);
    patcompile(&tok, 0, None)
}

/// `[[ $string = $pattern ]]`.
fn pat_matches(pat: &str, s: &str) -> bool {
    match compile_pat(pat) {
        Some(p) => pattry(&p, s),
        None => pat == s,
    }
}

/// `zstyle -T <ctx> <style>` — true when the style is unset, or set
/// with a boolean-true first value (`Src/Modules/zutil.c:700-724`).
fn style_true_or_unset(ctx: &str, style: &str) -> bool {
    match lookupstyle(ctx, style).first() {
        Some(v) => matches!(v.as_str(), "true" | "yes" | "on" | "1"),
        None => true,
    }
}

/// `[[:blank:]]` — space and tab.
fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// `${x##*[[:blank:]]}` — drop everything up to and including the last
/// blank.
fn strip_to_last_blank(s: &str) -> String {
    match s.char_indices().filter(|(_, c)| is_blank(*c)).last() {
        Some((i, c)) => s[i + c.len_utf8()..].to_string(),
        None => s.to_string(),
    }
}

/// sh:31's `${#desc[1,(r)(#i)[[:blank:]]pid]}` — the length of the
/// header up to and including the FIRST case-insensitive
/// `[[:blank:]]pid`, i.e. the column where the PID field ends. A
/// scalar `[1,(r)pat]` subscript that finds no match yields the empty
/// string, so an unmatched header gives 0.
fn pid_column_end(desc: &str) -> usize {
    let chars: Vec<char> = desc.chars().collect();
    for i in 0..chars.len() {
        if !is_blank(chars[i]) || i + 3 >= chars.len() {
            continue;
        }
        let (p, d, x) = (chars[i + 1], chars[i + 2], chars[i + 3]);
        if p.eq_ignore_ascii_case(&'p') && d.eq_ignore_ascii_case(&'i') && x.eq_ignore_ascii_case(&'d')
        {
            return i + 4;
        }
    }
    0
}

/// sh:31's `${(M)row#<n ?>[^[:blank:]]#}` — the first `n` characters
/// plus the non-blank run that follows them. `(M)` yields the empty
/// string when the pattern does not match, which is what a row shorter
/// than the header's PID column produces.
fn pid_field_at_column(row: &str, n: usize) -> String {
    let chars: Vec<char> = row.chars().collect();
    if chars.len() < n {
        return String::new();
    }
    let mut end = n;
    while end < chars.len() && !is_blank(chars[end]) {
        end += 1;
    }
    let matched: String = chars[..end].iter().collect();
    strip_to_last_blank(&matched)
}

/// sh:33's `${(M)row##[^0-9]#[0-9]#}` — the longest leading run of
/// non-digits followed by the longest run of digits.
fn pid_field_leading_digits(row: &str) -> String {
    let mut end = 0;
    let chars: Vec<char> = row.chars().collect();
    while end < chars.len() && !chars[end].is_ascii_digit() {
        end += 1;
    }
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    let matched: String = chars[..end].iter().collect();
    strip_to_last_blank(&matched)
}

/// sh:37's `${(r:COLUMNS-1:)line}` — truncate to `width` characters,
/// or right-pad with blanks when shorter.
fn rpad_to(line: &str, width: usize) -> String {
    let chars: Vec<char> = line.chars().collect();
    if chars.len() >= width {
        chars[..width].iter().collect()
    } else {
        let mut s: String = chars.iter().collect();
        s.extend(std::iter::repeat(' ').take(width - chars.len()));
        s
    }
}

/// `_pids` — complete process IDs from `ps`.
pub fn _pids(args: &[String]) -> i32 {
    let _fn_scope = FnScope::enter("_pids");
    // sh:6  local out pids list expl match desc listargs all nm ret=1
    //
    // Declared as scalars, exactly as upstream does: `match` in
    // particular must NOT be an array, because `lookupstyle`'s
    // `savematch`/`restorematch` bracket (`Src/Modules/zutil.c:450-459`)
    // reads it with `getaparam` and would otherwise snapshot and
    // restore it around every style lookup. `pids` and `list` become
    // arrays on assignment, the same way `pids=( … )` converts the
    // scalar upstream.
    let _scope = LocalScope::declare(
        &[
            "out", "pids", "list", "expl", "match", "desc", "listargs", "all", "nm",
        ],
        0,
    );
    let mut ret = 1; // sh:6 ret=1

    // sh:8  _tags processes || return 1
    if _tags(&["processes".to_string()]) != 0 {
        return 1;
    }

    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();

    // sh:10-21 — `all` (the -P/-S/-U insert flags) and `match` (the
    // filter applied to `ps` output at sh:28) are chosen together.
    let all: Vec<String>;
    let match_pat: String;
    let mut argv: Vec<String> = args.to_vec();
    let mut nm = String::new();
    if argv.first().map(String::as_str) == Some("-m") {
        // sh:11-13
        all = Vec::new();
        let pat = argv.get(1).cloned().unwrap_or_default();
        match_pat = format!(
            "(*[[:blank:]]|){}[0-9]#{}[[:blank:]]*(/|[[:blank:]]-(#c,1)){}([[:blank:]]*|)",
            prefix, suffix, pat
        );
        argv = argv.iter().skip(2).cloned().collect(); // sh:13 shift 2
    } else if pat_matches("([%-]*|[0-9]#)", &format!("{}{}", prefix, suffix)) {
        // sh:15-16
        all = Vec::new();
        match_pat = format!("(*[[:blank:]]|){}[0-9]#{}[[:blank:]]*", prefix, suffix);
    } else {
        // sh:18-20
        all = vec![
            "-P".to_string(),
            getsparam("IPREFIX").unwrap_or_default(),
            "-S".to_string(),
            getsparam("ISUFFIX").unwrap_or_default(),
            "-U".to_string(),
        ];
        match_pat = format!("*[[:blank:]]*[[/[:blank:]]{}*{}*", prefix, suffix);
        nm = get_compstate_str("nmatches").unwrap_or_default();
    }
    let match_prog = compile_pat(&match_pat);

    // sh:23-46  the tag / requested / next-label loop.
    loop {
        // sh:23  while _tags; do
        if _tags(&[]) != 0 {
            break;
        }
        // sh:24  if _requested processes; then
        if _requested(&["processes".to_string()]) == 0 {
            // sh:25  while _next_label processes expl 'process ID'; do
            loop {
                if _next_label(&[
                    "processes".to_string(),
                    "expl".to_string(),
                    "process ID".to_string(),
                ]) != 0
                {
                    break;
                }
                // sh:26  out=( "${(@f)$(_call_program $curtag ps 2>/dev/null)}" )
                //   `$(…)` strips trailing newlines, then `(@f)` splits on
                //   the remaining ones.
                let curtag = getsparam("curtag").unwrap_or_else(|| "processes".to_string());
                let _ = _call_program(&[curtag.clone(), "ps".to_string()]);
                let raw = getsparam("REPLY").unwrap_or_default();
                let out: Vec<&str> = raw.trim_end_matches('\n').split('\n').collect();
                // sh:27  desc="$out[1]"  (the header row)
                let header = out.first().copied().unwrap_or("");
                // sh:28  out=( "${(@M)out[2,-1]:#${~match}}" ) — keep only
                //   the rows the `match` pattern selects. Dropping this
                //   filter offered EVERY pid on the box: `ps a<TAB>` added
                //   118 process IDs where zsh adds one.
                let rows: Vec<&str> = out
                    .iter()
                    .skip(1)
                    .copied()
                    .filter(|line| match match_prog.as_ref() {
                        Some(p) => pattry(p, line),
                        None => *line == match_pat,
                    })
                    .collect();

                // sh:30  if [[ "$desc" = (#i)(|*[[:blank:]])pid(|[[:blank:]]*) ]]
                let pids: Vec<String> =
                    if pat_matches("(#i)(|*[[:blank:]])pid(|[[:blank:]]*)", header) {
                        // sh:31 — the PID column is located by the width of
                        //   the header up to its `PID` label.
                        let n = pid_column_end(header);
                        rows.iter().map(|r| pid_field_at_column(r, n)).collect()
                    } else {
                        // sh:33
                        rows.iter().map(|r| pid_field_leading_digits(r)).collect()
                    };

                // sh:36-41  verbose listing: the full `ps` row is the
                //   display string for each pid.
                let curcontext = getsparam("curcontext").unwrap_or_default();
                let desc_opt: Vec<String> = if style_true_or_unset(
                    &format!(":completion:{}:{}", curcontext, curtag),
                    "verbose",
                ) {
                    // sh:37  list=( "${(@Mr:COLUMNS-1:)out}" )
                    let columns = getsparam("COLUMNS")
                        .and_then(|c| c.parse::<usize>().ok())
                        .unwrap_or(80);
                    let width = columns.saturating_sub(1);
                    let list: Vec<String> = rows.iter().map(|r| rpad_to(r, width)).collect();
                    let _ = setaparam("list", list);
                    // sh:38  desc=(-ld list)
                    vec!["-ld".to_string(), "list".to_string()]
                } else {
                    // sh:40  desc=()
                    Vec::new()
                };

                // sh:42  compadd "$@" "$expl[@]" "$desc[@]" "$all[@]" -a pids
                let _ = setaparam("pids", pids);
                let expl = getaparam("expl").unwrap_or_default();
                let mut cadd: Vec<String> = argv.clone();
                cadd.extend(expl);
                cadd.extend(desc_opt);
                cadd.extend(all.clone());
                cadd.push("-a".to_string());
                cadd.push("pids".to_string());
                if bin_compadd("compadd", &cadd, &make_ops(), 0) == 0 {
                    ret = 0;
                }
            }
        }
        // sh:45  (( ret )) || break
        if ret == 0 {
            break;
        }
    }

    // sh:48  if [[ -n "$all" ]]; then
    if !all.is_empty() {
        // sh:49  zstyle -s … insert-ids out || out=menu
        let curcontext = getsparam("curcontext").unwrap_or_default();
        let insert_ids = lookupstyle(&format!(":completion:{}:processes", curcontext), "insert-ids")
            .first()
            .cloned()
            .unwrap_or_else(|| "menu".to_string());
        match insert_ids.as_str() {
            // sh:52  menu) compstate[insert]=menu
            "menu" => set_compstate_str("insert", "menu"),
            // sh:53-54  single) …
            "single" => {
                let nmatches: i64 = get_compstate_str("nmatches")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let prev: i64 = nm.parse().unwrap_or(0);
                if nmatches != prev + 1
                    && get_compstate_str("insert").as_deref() != Some("menu")
                {
                    set_compstate_str("insert", "");
                }
            }
            // sh:55-56  *) …
            _ => {
                let ps = format!(
                    "{}{}",
                    getsparam("PREFIX").unwrap_or_default(),
                    getsparam("SUFFIX").unwrap_or_default()
                );
                let unambiguous = get_compstate_str("unambiguous").unwrap_or_default();
                if ps.chars().count() > unambiguous.chars().count() {
                    set_compstate_str("insert", "menu");
                }
            }
        }
    }

    // sh:60  return ret
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_pids(&[]), 1);
    }

    #[test]
    fn pid_column_is_found_from_the_header() {
        // sh:31 — `${#desc[1,(r)(#i)[[:blank:]]pid]}`: the first
        // blank-preceded `PID`, case-insensitive. `PPID` must not win.
        assert_eq!(pid_column_end("  UID   PID  PPID   C STIME TTY"), 11);
        assert_eq!(pid_column_end("USER  pid  command"), 9);
        // No blank-preceded `pid` at all → the scalar subscript is empty.
        assert_eq!(pid_column_end("PID TTY TIME CMD"), 0);
    }

    #[test]
    fn pid_is_taken_from_the_header_column() {
        // sh:31 — n header chars plus the trailing non-blank run, then
        // everything up to the last blank is dropped.
        let n = pid_column_end("  UID   PID  PPID   C STIME TTY");
        assert_eq!(pid_field_at_column("  501 12345     1   0 ttys000", n), "12345");
        // A row that ends inside the PID column cannot match the
        // pattern, so `(M)` yields the empty string.
        assert_eq!(pid_field_at_column("  501 7", n), "");
    }

    #[test]
    fn pid_falls_back_to_the_leading_digit_run() {
        // sh:33 — `[^0-9]#[0-9]#` then `##*[[:blank:]]`.
        assert_eq!(pid_field_leading_digits("  501 ttys000 zsh"), "501");
        assert_eq!(pid_field_leading_digits("12345 ttys000 zsh"), "12345");
    }

    #[test]
    fn verbose_list_lines_are_padded_to_the_screen_width() {
        // sh:37 — `${(r:COLUMNS-1:)line}` truncates as well as pads.
        assert_eq!(rpad_to("abcdefghij", 5), "abcde");
        assert_eq!(rpad_to("abc", 5), "abc  ");
    }
}
