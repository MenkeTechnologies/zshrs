//! Port of `_fuse_values` from `Completion/Linux/Type/_fuse_values`.
//!
//! Full upstream body (71 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local ret stateset fvals cvalsvar cvalind
//! sh: 4  typeset -a fvals opts
//! sh: 6  if [[ $1 = -O* ]]; then opts+=$1; shift; fi
//! sh:10  opts+=(-s , -S =)
//! sh:12  cvalind=$argv[(I)-A*]                       # last `-A*` index
//! sh:14  while (( cvalind )); do                     # strip all `-A*`
//! sh:15    if [[ ${(P)cvalind} = -A?* ]]; then       #   glued `-Aname`
//! sh:16      cvalsvar=${(P)cvalind[3,-1]}; argv[cvalind]=()
//! sh:18    else                                      #   separate `-A name`
//! sh:19      cvalsvar=$argv[cvalind+1]; argv[cvalind,cvalind+1]=()
//! sh:21    fi
//! sh:22    cvalind=$argv[(I)-A*]
//! sh:23  done
//! sh:25  fvals=( 'ro[…]' 'allow_other[…]' … 'attr_timeout[…]:timeout (s)' )
//! sh:47  [[ -n $cvalsvar ]] && set -- "$@" ${(P)cvalsvar}
//! sh:49  if [[ $# -eq 0 ]]; then set -- 'mount option' $fvals
//! sh:51  else set -- "$@" $fvals; fi
//! sh:55  if [[ -n $state ]]; then stateset=$state; state=; fi
//! sh:60  _values $opts "$@" && ret=0
//! sh:62  if [[ -n $state ]]; then compstate[restore]=
//! sh:64  elif [[ -n $stateset ]]; then state=$stateset
//! sh:66  else unset state; fi
//! sh:70  return $ret
//! ```

use crate::compsys::ported::_values::_values;
use crate::ported::params::{getaparam, getsparam, setsparam, unsetparam};
use crate::ported::zle::compcore::set_compstate_str;

/// sh:25-45 — the FUSE mount-option value specs (`name[desc]` / `name[desc]:arg`).
const FVALS: &[&str] = &[
    "ro[mount filesystem read-only]",
    "allow_other[allow access to other users]",
    "allow_root[allow access to root]",
    "nonempty[allow mounts over non-empty file/dir]",
    "default_permissions[enable permission checking by kernel]",
    "fsname[set filesystem name]:name",
    "large_read[issue large read requests (linux v2.4 only)]",
    "max_read[set maximum size of read requests]:size",
    "hard_remove[immediate removal (do not hide files)]",
    "use_ino[let filesystem set inode numbers]",
    "readdir_ino[try to fill in d_ino in readdir]",
    "direct_io[use direct I/O]",
    "kernel_cache[cache files in kernel]",
    "umask[set file permissions]:permission mask (octal)",
    "uid[set file owner]:user id:",
    "gid[set file group]:group id:",
    "entry_timeout[cache timeout for names]:timeout (s)",
    "negative_timeout[cache timeout for deleted names]:timeout (s)",
    "attr_timeout[cache timeout for attributes]:timeout (s)",
];

/// sh:6-23 — build the `opts` list and strip every `-A*` element from the
/// incoming argv, returning `(opts, rest, cvalsvar)`.
///
/// * A leading `-O*` (checked against `$1` only) is moved into `opts`,
///   followed unconditionally by `-s , -S =`.
/// * Both the glued `-Aname` and the separate `-A name` forms are removed.
///   The shell processes `-A*` elements highest-index-first, reassigning the
///   scalar `cvalsvar` each pass, so the *leftmost* `-A` wins — replicated
///   here by only setting `cvalsvar` on the first `-A` seen left-to-right.
fn parse_opts_and_cvals(args: &[String]) -> (Vec<String>, Vec<String>, Option<String>) {
    let mut opts: Vec<String> = Vec::new();
    let mut i = 0usize;

    // sh:6-9 — leading `-O*` on `$1`.
    if args.first().map_or(false, |s| s.starts_with("-O")) {
        opts.push(args[0].clone());
        i = 1;
    }
    // sh:10
    opts.push("-s".to_string());
    opts.push(",".to_string());
    opts.push("-S".to_string());
    opts.push("=".to_string());

    // sh:12-23
    let mut rest: Vec<String> = Vec::new();
    let mut cvalsvar: Option<String> = None;
    while i < args.len() {
        let a = &args[i];
        if a == "-A" {
            // sh:18-20 — separate `-A name`: value is the next element.
            let name = args.get(i + 1).cloned().unwrap_or_default();
            if cvalsvar.is_none() {
                cvalsvar = Some(name);
            }
            i += if i + 1 < args.len() { 2 } else { 1 };
        } else if a.starts_with("-A") {
            // sh:15-17 — glued `-Aname`: value is chars after `-A`.
            if cvalsvar.is_none() {
                cvalsvar = Some(a[2..].to_string());
            }
            i += 1;
        } else {
            rest.push(a.clone());
            i += 1;
        }
    }

    (opts, rest, cvalsvar)
}

/// `_fuse_values` — complete FUSE mount options as a comma-separated value
/// list (`_values -s , -S = …`).
pub fn _fuse_values(args: &[String]) -> i32 {
    // sh:6-23
    let (opts, mut rest, cvalsvar) = parse_opts_and_cvals(args);

    // sh:47 — `[[ -n $cvalsvar ]] && set -- "$@" ${(P)cvalsvar}`:
    // append the (word-split) expansion of the caller-named parameter.
    if let Some(name) = cvalsvar.as_deref() {
        if !name.is_empty() {
            let extra = getaparam(name).unwrap_or_else(|| {
                getsparam(name)
                    .map(|s| s.split_whitespace().map(String::from).collect())
                    .unwrap_or_default()
            });
            rest.extend(extra);
        }
    }

    // sh:49-53 — with no positionals, lead with a description; otherwise
    // append the FUSE value specs after the caller's positionals.
    let mut positional: Vec<String> = if rest.is_empty() {
        vec!["mount option".to_string()]
    } else {
        rest
    };
    positional.extend(FVALS.iter().map(|s| s.to_string()));

    // sh:55-58 — save any pre-existing `$state`, then clear it so `_values`
    // can set it fresh.
    let state = getsparam("state").unwrap_or_default();
    let stateset = if !state.is_empty() {
        setsparam("state", "");
        state
    } else {
        String::new()
    };

    // sh:60 — `_values $opts "$@" && ret=0`.
    let mut cadd = opts;
    cadd.extend(positional);
    let ret = if _values(&cadd) == 0 { 0 } else { 1 };

    // sh:62-68 — restore state plumbing.
    if !getsparam("state").unwrap_or_default().is_empty() {
        // sh:63 — `_values` set a state; let the caller keep it.
        set_compstate_str("restore", "");
    } else if !stateset.is_empty() {
        // sh:65 — restore the state we saved.
        setsparam("state", &stateset);
    } else {
        // sh:67 — no state either way; clear it.
        unsetparam("state");
    }

    // sh:70
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opts_always_carry_separator_flags() {
        let (opts, _, _) = parse_opts_and_cvals(&[]);
        assert_eq!(opts, vec!["-s", ",", "-S", "="]);
    }

    #[test]
    fn leading_o_flag_moves_into_opts() {
        let (opts, rest, cval) = parse_opts_and_cvals(&["-Oxx".into(), "foo".into()]);
        assert_eq!(opts, vec!["-Oxx", "-s", ",", "-S", "="]);
        assert_eq!(rest, vec!["foo".to_string()]);
        assert_eq!(cval, None);
    }

    #[test]
    fn glued_a_form_strips_and_captures_name() {
        let (_, rest, cval) = parse_opts_and_cvals(&["-Amyvar".into(), "keep".into()]);
        assert_eq!(rest, vec!["keep".to_string()]);
        assert_eq!(cval.as_deref(), Some("myvar"));
    }

    #[test]
    fn separate_a_form_consumes_next_element() {
        let (_, rest, cval) =
            parse_opts_and_cvals(&["a".into(), "-A".into(), "myvar".into(), "b".into()]);
        assert_eq!(rest, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(cval.as_deref(), Some("myvar"));
    }

    #[test]
    fn leftmost_a_wins_across_multiple() {
        let (_, rest, cval) =
            parse_opts_and_cvals(&["-Afirst".into(), "x".into(), "-Asecond".into()]);
        assert_eq!(rest, vec!["x".to_string()]);
        assert_eq!(cval.as_deref(), Some("first"));
    }

    #[test]
    fn dangling_separate_a_yields_empty_name() {
        let (_, rest, cval) = parse_opts_and_cvals(&["-A".into()]);
        assert!(rest.is_empty());
        assert_eq!(cval.as_deref(), Some(""));
    }

    #[test]
    fn fvals_specs_are_intact() {
        assert_eq!(FVALS.len(), 19);
        assert_eq!(FVALS[0], "ro[mount filesystem read-only]");
        assert_eq!(
            FVALS[FVALS.len() - 1],
            "attr_timeout[cache timeout for attributes]:timeout (s)"
        );
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        // Outside a completion, `compvalues -i` fails → `_values` returns 1.
        assert_eq!(_fuse_values(&[]), 1);
    }
}
