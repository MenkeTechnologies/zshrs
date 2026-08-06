//! Port of `_pids` from `Completion/Unix/Type/_pids`.
//!
//! Full upstream body (60 lines, abridged):
//! ```text
//! sh: 1  #compdef pflags pcred pldd psig pstack pfiles pwdx pstop prun pwait
//! sh: 6  local out pids list expl match desc listargs all nm ret=1
//! sh: 8  _tags processes || return 1
//! sh:10  if [[ "$1" = -m ]]; then …match on command line… shift 2
//! sh:14  elif [[ "$PREFIX$SUFFIX" = ([%-]*|[0-9]#) ]]; then …numeric-only…
//! sh:18  else all=(-P "$IPREFIX" -S "$ISUFFIX" -U); … nm=$compstate[nmatches] fi
//! sh:23  while _tags; do
//! sh:24    if _requested processes; then
//! sh:25      while _next_label processes expl 'process ID'; do
//! sh:26        out=( "${(@f)$(_call_program $curtag ps 2>/dev/null)}" )
//! sh:28        desc="$out[1]"; out=( matching lines )
//! sh:30-34    pids=( extract PID column )
//! sh:36-42    verbose list handling
//! sh:42        compadd "$@" "$expl[@]" "$desc[@]" "$all[@]" -a pids && ret=0
//! sh:45    done; fi
//! sh:45    (( ret )) || break
//! sh:46  done
//! sh:50-58 insert-ids compstate tuning (approximated)
//! sh:60  return ret
//! ```
//!
//! sh:30-34 approx — the PID column is located by scanning the header
//! row for the `PID` label rather than the `${(l.…)}`-padding trick.
//! sh:10 `-m PATTERN` command-line filter and sh:50-58 `insert-ids`
//! compstate tuning are approximated (the compadd prefix matcher does
//! the real filtering).

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_next_label::_next_label;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::ported::params::{getaparam, getsparam};
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

/// sh:30-34 — pull the PID column out of `ps` output. `header` is the
/// first line; the PID field index is found from the `PID` label, else
/// the first all-digit token on each row is used.
fn extract_pids(header: &str, rows: &[&str]) -> Vec<String> {
    let pid_col = header
        .split_whitespace()
        .position(|h| h.eq_ignore_ascii_case("PID"));
    let mut pids = Vec::new();
    for row in rows {
        let fields: Vec<&str> = row.split_whitespace().collect();
        let pid = match pid_col.and_then(|c| fields.get(c)) {
            Some(f) if f.chars().all(|c| c.is_ascii_digit()) && !f.is_empty() => Some(*f),
            _ => fields
                .iter()
                .find(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_digit()))
                .copied(),
        };
        if let Some(p) = pid {
            pids.push(p.to_string());
        }
    }
    pids
}

/// `_pids` — complete process IDs from `ps`.
pub fn _pids(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_pids");
    let mut ret = 1;
    // sh:8  _tags processes || return 1
    if _tags(&["processes".to_string()]) != 0 {
        return 1;
    }

    // sh:10-22 — decide the `all` (-P/-S/-U insert) args. The `-m PATTERN`
    // and numeric-only special cases only change the match filter, which
    // the compadd prefix matcher already applies; keep the general branch.
    let (all, rest): (Vec<String>, Vec<String>) = if args.first().map(|s| s.as_str()) == Some("-m")
    {
        // sh:10-13 — `-m PATTERN`: filter by command line (approx: skip
        // the two consumed args, no extra insert flags).
        (Vec::new(), args.iter().skip(2).cloned().collect())
    } else {
        let ps = format!(
            "{}{}",
            getsparam("PREFIX").unwrap_or_default(),
            getsparam("SUFFIX").unwrap_or_default()
        );
        let numeric_only = ps.is_empty()
            || ps.chars().all(|c| c.is_ascii_digit())
            || ps.starts_with('%')
            || ps.starts_with('-');
        if numeric_only {
            // sh:14-16
            (Vec::new(), args.to_vec())
        } else {
            // sh:18-22  all=(-P "$IPREFIX" -S "$ISUFFIX" -U)
            let ipre = getsparam("IPREFIX").unwrap_or_default();
            let isuf = getsparam("ISUFFIX").unwrap_or_default();
            (
                vec![
                    "-P".to_string(),
                    ipre,
                    "-S".to_string(),
                    isuf,
                    "-U".to_string(),
                ],
                args.to_vec(),
            )
        }
    };

    // sh:24-48  the tag / requested / next-label loop.
    loop {
        if _tags(&[]) != 0 {
            break;
        }
        // sh:24  _requested processes
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
                // sh:27  out=( "${(@f)$(_call_program $curtag ps)}" )
                let curtag = getsparam("curtag").unwrap_or_else(|| "processes".to_string());
                let _ = _call_program(&[curtag, "ps".to_string()]);
                let raw = getsparam("REPLY").unwrap_or_default();
                let lines: Vec<&str> = raw.lines().collect();
                if lines.is_empty() {
                    continue;
                }
                // sh:28  desc="$out[1]"  (header row)
                let header = lines[0];
                let rows = &lines[1..];
                // sh:30-34
                let pids = extract_pids(header, rows);
                // sh:42  compadd "$@" "$expl[@]" "$all[@]" -a pids
                let expl = getaparam("expl").unwrap_or_default();
                let mut cadd: Vec<String> = rest.clone();
                cadd.extend(expl);
                cadd.extend(all.clone());
                cadd.push("--".to_string());
                cadd.extend(pids);
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
    // sh:60
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
    fn extract_pids_uses_pid_column() {
        // sh:30-34 — header locates the PID column.
        let header = "  PID TTY           TIME CMD";
        let rows = [
            "  501 ttys000    0:00.10 zsh",
            " 1234 ttys001    0:00.01 vim",
        ];
        assert_eq!(extract_pids(header, &rows), vec!["501", "1234"]);
    }
}
