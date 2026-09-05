//! Port of `_remote_files` from `Completion/Unix/Type/_remote_files`.
//!
//! Full upstream body (114 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh: 36  local expl rempat remfiles remdispf{,q} remdispd{,q} args cmd suf ret=1
//! sh: 38  local glob host dir esc dirprefix
//! sh: 40  if zstyle -T ":completion:${curcontext}:files" remote-access; then
//! sh: 43    zparseopts -D -E -a args / g:=glob h:=host W:=dir Q:=esc
//! sh: 44    (( $#host)) && shift host || host="${IPREFIX%:}"
//! sh: 49    args=( ${argv[1,(i)--]} ); shift ${#args}
//! sh: 48    [[ $args[-1] = -- ]] && args[-1]=()
//! sh: 53    cmd="$1"; shift
//! sh: 56    if [[ $cmd == ssh ]]; then zparseopts -D -E -a cmd_args p: 1 2 4 6 F:
//! sh: 58      cmd_args=( -o BatchMode=yes "$cmd_args[@]" -a -x ); else cmd_args=( "$@" ); fi
//! sh: 62    (( $#dir )) && dirprefix=${dir}/
//! sh: 65    rempat="${dirprefix}${PREFIX%%[^./][^/]#}\*"   (Q-quoted if $QIPREFIX)
//! sh: 71    remfiles=(${(M)${(f)"$(_call_program files $cmd $cmd_args $host command ls -d1FL -- "$rempat")"}%%[^/]#(|/)})
//! sh: 76    compset -P '*/'
//! sh: 77    compset -S '/*' || (( ${args[(I)-/]} )) || suf='remote file'
//! sh: 80    remdispf=(${remfiles:#*/}); remdispd=(${(M)remfiles:#*/})
//! sh: 83    if (( $#glob )); then match=( '(|[*=|])' )
//! sh: 85      glob[2]="${glob[2]/(#b)\(((|^)[p=\*])\)(#e)/}"
//! sh: 86      glob[2]+="${${match[1]/p/\|}/\*/\*}"
//! sh: 87      remdispf=( ${(M)remdispf:#${~glob[2]}} ); fi
//! sh: 90    if (( $#esc )); then remdispfq=(${${remdispf%[*=|]}//(#b)(${~esc[2]})/\\$match[1]})
//! sh: 92      remdispdq=(${${remdispd%/}//(#b)(${~esc[2]})/\\$match[1]})
//! sh: 94    else remdispfq=(${(q)remdispf%[*=|]}); remdispdq=(${(q)remdispd%/}); fi
//! sh: 99    [[ -o autoremoveslash ]] && autoremove=(-r "/ \t\n\-")
//! sh:101    _tags remote-files; while _tags; do while _next_label remote-files expl ${suf:-remote directory}; do
//! sh:104      [[ -n $suf ]] && compadd "$args[@]" "$expl[@]" -d remdispf -- $remdispfq && ret=0
//! sh:106      compadd ${suf:+-S/} $autoremove "$args[@]" "$expl[@]" -d remdispd -- $remdispdq && ret=0
//! sh:108    done; (( ret )) || return 0; done; return ret
//! sh:112  else _message -e remote-files 'remote file'; fi
//! ```
//!
//! sh:65 the `${PREFIX%%…}` component strip and the sh:94 `(q)` quoting are
//! string-op equivalents of the zsh expansion (marked `// sh:N approx`);
//! the `-g` glob filter (sh:83-88) and `-Q` escape (sh:90-92) run the real
//! ported glob matcher.

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_next_label::_next_label;
use crate::compsys::ported::_tags::_tags;
use crate::ported::glob::{matchpat, tokenize};
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zle::complete::{bin_compadd, bin_compset};
use crate::ported::zsh_h::{isset, options, AUTOREMOVESLASH, CASEGLOB, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}
fn compset(argv: Vec<String>) -> i32 {
    bin_compset("compset", &argv, &make_ops(), 0)
}

/// zsh `zstyle -T` — true when unset OR set true; false only when set false.
fn zstyle_t_default_true(ctx: &str, style: &str) -> bool {
    !matches!(
        lookupstyle(ctx, style).first().map(|s| s.as_str()),
        Some("no") | Some("false") | Some("off") | Some("0")
    )
}

/// sh:65 approx — strip the trailing (non-dotfile) filename component of
/// `PREFIX`, keeping the directory part and any leading `.` of the tail.
fn dir_component(prefix: &str) -> String {
    match prefix.rfind('/') {
        Some(i) => {
            let (head, tail) = prefix.split_at(i + 1);
            if tail.starts_with('.') {
                format!("{}.", head)
            } else {
                head.to_string()
            }
        }
        None if prefix.starts_with('.') => ".".to_string(),
        None => String::new(),
    }
}

/// Match `text` against zsh glob `pat` via the real ported matcher.
/// `(...)` grouping / alternation (`(|[*=|])`) needs EXTENDEDGLOB semantics,
/// so matchpat is called with `extended = true` regardless of the option.
fn glob_match(pat: &str, text: &str) -> bool {
    let mut p = pat.to_string();
    tokenize(&mut p);
    matchpat(&p, text, true, isset(CASEGLOB))
}

/// sh:85 — strip a trailing `(p)` / `(=)` / `(*)` / `(^p)` / `(^=)` / `(^*)`
/// glob-qualifier group from a `-g` pattern (only that shape, only at the end).
fn strip_trailing_qualifier(g: &str) -> String {
    let b = g.as_bytes();
    let n = b.len();
    if n >= 3 && b[n - 1] == b')' {
        // `(X)`
        if b[n - 3] == b'(' && matches!(b[n - 2], b'p' | b'=' | b'*') {
            return g[..n - 3].to_string();
        }
        // `(^X)`
        if n >= 4 && b[n - 4] == b'(' && b[n - 3] == b'^' && matches!(b[n - 2], b'p' | b'=' | b'*')
        {
            return g[..n - 4].to_string();
        }
    }
    g.to_string()
}

/// sh:94 approx — zsh `${(q)s}` backslash quoting for a remote filename:
/// escape whitespace and shell/glob metacharacters so the inserted word is
/// literal (the character set zsh's `quotestring` escapes with `QT_BACKSLASH`).
fn quote_q(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_whitespace() || "\\'\"$`*?[]()<>|&;#~^={}!".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// sh:91 — `${str//(#b)(${~esc[2]})/\\$match[1]}`: backslash-escape every
/// char of `str` that matches the `-Q` pattern `esc` (evaluated as a glob).
fn escape_by_pattern(s: &str, esc: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let ch = c.to_string();
        if glob_match(esc, &ch) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `_remote_files` — complete files on a remote host via ssh/rsh.
pub fn _remote_files(args_in: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_remote_files");
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let files_ctx = format!(":completion:{}:files", curcontext);

    // sh:40 — honour the remote-access style (default on).
    if !zstyle_t_default_true(&files_ctx, "remote-access") {
        // sh:112
        return _message(&[
            "-e".to_string(),
            "remote-files".to_string(),
            "remote file".to_string(),
        ]);
    }

    // sh:46 — parse _remote_files options; `-/` (dirs-only) stays in `args`
    //   as a compadd passthrough. `-g`/`-Q` capture a value; the rest splits
    //   at `--` (before = passthrough compadd args, after = remote command).
    let mut host: Option<String> = None;
    let mut dir: Option<String> = None;
    let mut glob: Option<String> = None; // sh:43 g:=glob
    let mut esc: Option<String> = None; // sh:43 Q:=esc
    let mut dirs_only = false;
    let mut passthru: Vec<String> = Vec::new();
    let mut cmdline: Vec<String> = Vec::new();
    let mut it = args_in.iter().cloned().peekable();
    let mut after_dashdash = false;
    while let Some(a) = it.next() {
        if after_dashdash {
            cmdline.push(a);
            continue;
        }
        match a.as_str() {
            "--" => after_dashdash = true,
            "-/" => {
                dirs_only = true;
                passthru.push(a); // sh:43 `/` stays in $args (compadd -/)
            }
            "-h" => host = it.next(),
            "-W" => dir = it.next(),
            "-g" => glob = it.next(), // sh:43 g:=glob
            "-Q" => esc = it.next(),  // sh:43 Q:=esc
            _ => passthru.push(a),
        }
    }

    // sh:44 — default host = ${IPREFIX%:}.
    let host = host.unwrap_or_else(|| {
        getsparam("IPREFIX")
            .unwrap_or_default()
            .trim_end_matches(':')
            .to_string()
    });

    // sh:53-59 — remote command + its args (ssh gets non-interactive flags).
    let cmd = cmdline.first().cloned().unwrap_or_default();
    let cmd_rest: Vec<String> = cmdline.iter().skip(1).cloned().collect();
    let cmd_args: Vec<String> = if cmd == "ssh" {
        let mut v = vec!["-o".to_string(), "BatchMode=yes".to_string()];
        v.extend(cmd_rest);
        v.push("-a".to_string());
        v.push("-x".to_string());
        v
    } else {
        cmd_rest
    };

    // sh:62-65 — remote pattern from the working dir + PREFIX component.
    let dirprefix = dir.map(|d| format!("{}/", d)).unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let rempat = format!("{}{}*", dirprefix, dir_component(&prefix));

    // sh:71 — remote `ls -d1FL` listing; keep the classifier-tagged names.
    let mut call: Vec<String> = vec!["files".to_string(), cmd];
    call.extend(cmd_args);
    call.push(host);
    call.extend(["command", "ls", "-d1FL", "--"].map(String::from));
    call.push(rempat);
    let _ = call_program_capture(&call);
    let listing = getsparam("REPLY").unwrap_or_default();

    // sh:80-81 — split into files vs directories. Directory names keep their
    //   trailing `/` classifier; file names keep `*`/`=`/`@`/`|` — the ls -F
    //   classifier is preserved for the `-d` DISPLAY column (sh:105).
    let mut remdispf: Vec<String> = Vec::new();
    let mut remdispd: Vec<String> = Vec::new();
    for raw in listing.lines() {
        if raw.is_empty() {
            continue;
        }
        if raw.ends_with('/') {
            remdispd.push(raw.to_string());
        } else {
            remdispf.push(raw.to_string());
        }
    }

    // sh:83-88 — apply the `-g` glob filter to the file display list.
    if let Some(g) = glob.as_ref() {
        let effective = format!("{}(|[*=|])", strip_trailing_qualifier(g));
        remdispf.retain(|f| glob_match(&effective, f));
    }

    // sh:90-96 — quoted candidate lists: strip the ls -F classifier, then
    //   either escape the `-Q` pattern's chars (sh:91) or `(q)`-quote (sh:94).
    let strip_file = |s: &str| -> String { s.trim_end_matches(['*', '=', '|']).to_string() };
    let strip_dir = |s: &str| -> String { s.trim_end_matches('/').to_string() };
    let (remdispfq, remdispdq): (Vec<String>, Vec<String>) = if let Some(e) = esc.as_ref() {
        (
            remdispf
                .iter()
                .map(|f| escape_by_pattern(&strip_file(f), e))
                .collect(),
            remdispd
                .iter()
                .map(|d| escape_by_pattern(&strip_dir(d), e))
                .collect(),
        )
    } else {
        (
            remdispf.iter().map(|f| quote_q(&strip_file(f))).collect(),
            remdispd.iter().map(|d| quote_q(&strip_dir(d))).collect(),
        )
    };

    // sh:76-77 — component compset + suffix decision.
    let _ = compset(vec!["-P".to_string(), "*/".to_string()]);
    let suf_is_file = compset(vec!["-S".to_string(), "/*".to_string()]) != 0 && !dirs_only;

    // sh:99 — autoremoveslash: strip a trailing `/` on the next keystroke.
    let autoremove: Vec<String> = if isset(AUTOREMOVESLASH) {
        vec!["-r".to_string(), "/ \t\n-".to_string()]
    } else {
        Vec::new()
    };

    // sh:101 — `_tags remote-files`.
    //
    //   This registration is what makes the `_next_label remote-files` below
    //   succeed: `_next_label`:8 is `comptags -A "$1" curtag __spec`, which
    //   answers out of the tag set the ENCLOSING `_tags` published. Most
    //   callers do not publish one — `_ssh`:714,728 and `_rlogin`:42
    //   call `_remote_files` bare — so with the registration missing
    //   `_next_label` failed on its first call, the loop broke before either
    //   compadd, and the function returned 1 with no matches at all.
    let _ = _tags(&["remote-files".to_string()]);

    // sh:102-110 — offer files (when not dirs-only) and directories, using the
    //   classifier DISPLAY (`-d`) list and the quoted candidate (`--`) list.
    let mut ret = 1;
    // sh:102  while _tags; do
    while _tags(&[]) == 0 {
        // sh:103  while _next_label remote-files expl ${suf:-remote directory}; do
        loop {
            let descr = if suf_is_file {
                "remote file"
            } else {
                "remote directory"
            };
            if _next_label(&[
                "remote-files".to_string(),
                "expl".to_string(),
                descr.to_string(),
            ]) != 0
            {
                break;
            }
            let expl = getaparam("expl").unwrap_or_default();
            // sh:105 — files: -d remdispf (classifier display) -- remdispfq.
            if suf_is_file && !remdispfq.is_empty() {
                setaparam("remdispf", remdispf.clone());
                let mut cadd = passthru.clone();
                cadd.extend(expl.clone());
                cadd.push("-d".to_string());
                cadd.push("remdispf".to_string());
                cadd.push("--".to_string());
                cadd.extend(remdispfq.clone());
                if bin_compadd("compadd", &cadd, &make_ops(), 0) == 0 {
                    ret = 0;
                }
            }
            // sh:107 — directories: ${suf:+-S/} $autoremove -d remdispd -- remdispdq.
            if !remdispdq.is_empty() {
                setaparam("remdispd", remdispd.clone());
                let mut cadd: Vec<String> = Vec::new();
                if suf_is_file {
                    cadd.push("-S".to_string());
                    cadd.push("/".to_string());
                }
                cadd.extend(autoremove.clone());
                cadd.extend(passthru.clone());
                cadd.extend(expl.clone());
                cadd.push("-d".to_string());
                cadd.push("remdispd".to_string());
                cadd.push("--".to_string());
                cadd.extend(remdispdq.clone());
                if bin_compadd("compadd", &cadd, &make_ops(), 0) == 0 {
                    ret = 0;
                }
            }
        }
        // sh:109  (( ret )) || return 0
        if ret == 0 {
            return 0;
        }
    }
    // sh:111  return ret
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_component_strips_trailing_name() {
        // sh:65 approx
        assert_eq!(dir_component("foo/bar"), "foo/");
        assert_eq!(dir_component("foo/.ba"), "foo/.");
        assert_eq!(dir_component("bar"), "");
        assert_eq!(dir_component(".ba"), ".");
    }

    #[test]
    fn strip_trailing_qualifier_removes_only_p_eq_star() {
        // sh:85
        assert_eq!(strip_trailing_qualifier("*.txt(p)"), "*.txt");
        assert_eq!(strip_trailing_qualifier("*.txt(=)"), "*.txt");
        assert_eq!(strip_trailing_qualifier("*.txt(*)"), "*.txt");
        assert_eq!(strip_trailing_qualifier("*.txt(^p)"), "*.txt");
        // not a p/=/* qualifier — left intact.
        assert_eq!(strip_trailing_qualifier("*.txt(.)"), "*.txt(.)");
        assert_eq!(strip_trailing_qualifier("*.txt"), "*.txt");
    }

    #[test]
    fn glob_filter_keeps_matching_files() {
        // sh:87 — `-g '*.rs'` keeps only .rs names, tolerating the ls -F
        // classifier suffix via the appended `(|[*=|])`.
        let effective = format!("{}(|[*=|])", strip_trailing_qualifier("*.rs"));
        let files = ["main.rs", "main.rs*", "readme.md", "lib.rs="];
        let kept: Vec<&str> = files
            .iter()
            .copied()
            .filter(|f| glob_match(&effective, f))
            .collect();
        assert_eq!(kept, vec!["main.rs", "main.rs*", "lib.rs="]);
    }

    #[test]
    fn quote_q_escapes_metachars() {
        // sh:94 approx
        assert_eq!(quote_q("a b"), "a\\ b");
        assert_eq!(quote_q("f*o"), "f\\*o");
        assert_eq!(quote_q("plain"), "plain");
    }

    #[test]
    fn escape_by_pattern_escapes_matched_chars() {
        // sh:91 — -Q '[ab]' escapes only a and b.
        assert_eq!(escape_by_pattern("cabbage", "[ab]"), "c\\a\\b\\b\\age");
    }

    #[test]
    fn returns_one_or_status_without_context() {
        let _g = crate::test_util::global_state_lock();
        let _ = _remote_files(&["--".to_string(), "ssh".to_string()]);
    }
}
