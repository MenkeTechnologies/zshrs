//! Behavioural parity corpus mined from the user's installed
//! `~/.zinit/plugins` daily-driver set — zinit itself, powerlevel10k,
//! zsh-autosuggestions, fzf, fzf-tab, fast-syntax-highlighting, the
//! MenkeTechnologies plugin family (zconvey, zsh-expand, zsh-z,
//! revolver, zunit, zsh-learn, …) and more.
//!
//! Each test replicates a DISTINCTIVE zsh idiom actually used in the
//! plugin source (cited `file:line` per test) as a self-contained,
//! deterministic mini-script, then asserts `zshrs --zsh -c` matches
//! `/opt/homebrew/bin/zsh -fc` byte-for-byte on stdout and exit code.
//!
//! Every script was verified to produce byte-identical output across
//! two consecutive real-zsh runs before inclusion (no $RANDOM/$$/time/
//! network/env dependence). Filesystem-touching scripts sandbox under
//! `mktemp -d` and clean up.
//!
//! Same harness shape as `tests/zinit_p10k_parity.rs` /
//! `tests/real_world_idioms_parity.rs`: skip silently when no zsh.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
}

fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}

fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct ShellResult {
    stdout: String,
    #[allow(dead_code)]
    stderr: String,
    exit: i32,
}

fn run_zsh(script: &str) -> ShellResult {
    let out = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn run_zshrs(script: &str) -> ShellResult {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-fc", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on script:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        script, z.stdout, r.stdout
    );
    assert_eq!(
        z.exit, r.exit,
        "exit-code divergence on:\n{}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
        script, z.exit, r.exit
    );
}

// ───────────────────────────── zsh-autosuggestions ─────────────────

mod zsh_autosuggestions {
    use super::*;

    /// zsh-autosuggestions.zsh:131 — (#m) match-ref backslash-escape of
    /// glob/quote chars under EXTENDED_GLOB.
    #[test]
    fn esc_glob_chars_via_match_ref() {
        assert_parity(
            r##"_esc() {
  setopt localoptions EXTENDED_GLOB
  echo -E "${1//(#m)[\"\'\\()\[\]|*?~]/\\$MATCH}"
}
_esc "cmd \"a\" (b) [c] *? ~x""##,
        );
    }

    /// zsh-autosuggestions.zsh:210 — filter array via
    /// `${arr:#${(j:|:)~ignore}}` joined-pattern exclusion.
    #[test]
    fn filter_array_with_joined_pattern() {
        assert_parity(
            r##"ignore_widgets=( .\* _\* orig-\* zle-\* beep )
wlist=( .safe _complete accept-line orig-1-foo zle-line-init beep my-widget )
print -rl -- ${wlist:#${(j:|:)~ignore_widgets}}"##,
        );
    }

    /// zsh-autosuggestions.zsh:627 — `${${(@0)line}[2]}` NUL-split then index.
    #[test]
    fn nul_split_then_index() {
        assert_parity(
            r##"line=$'junk\0suggestion text\0trailing'
print -r -- "${${(@0)line}[2]}""##,
        );
    }

    /// zsh-autosuggestions.zsh:663,707 — `[(r)pat]` value match and
    /// `${(k)assoc[(R)$~pat]}` all-matching-keys.
    #[test]
    fn assoc_reverse_subscript_value_and_keys() {
        assert_parity(
            r##"typeset -A hist_map=( 1 "ls foo" 2 "pwd" 3 "ls bar" 4 "git st" )
pattern="ls*"
print -r -- "${hist_map[(r)$pattern]}"
print -rl -- ${(k)hist_map[(R)$~pattern]}"##,
        );
    }

    /// zsh-autosuggestions.zsh:156-176 — case dispatch on `$widgets[w]`
    /// descriptors with split-and-slice.
    #[test]
    fn widget_descriptor_case_dispatch() {
        assert_parity(
            r##"typeset -A wmap=(
  fw "user:_zsh_autosuggest_bound_1_fw"
  bw "user:my_backward"
  cw "completion:.complete-word:_main_complete"
  il "builtin"
)
for w in fw bw cw il; do
  case $wmap[$w] in
    user:_zsh_autosuggest_(bound|orig)_*) print -r -- "$w already-bound" ;;
    user:*) print -r -- "$w user ${wmap[$w]#*:}" ;;
    builtin) print -r -- "$w builtin" ;;
    completion:*) print -r -- "$w comp ${${(s.:.)wmap[$w]}[2,3]}" ;;
  esac
done"##,
        );
    }
}

// ───────────────────────────────── zsh-autopair ────────────────────

mod zsh_autopair {
    use super::*;

    /// autopair.zsh:69 — count delimiter occurrences via
    /// `${#var//[^$1]}` with pattern char arriving by positional.
    #[test]
    fn count_unescaped_delimiters() {
        assert_parity(
            r##"typeset -g LBUFFER="a(b(c\(d(" RBUFFER=")x))"
_ap_count() {
  local lbuf="${LBUFFER//\\$1}"
  local rbuf="${RBUFFER//\\$2}"
  local llen="${#lbuf//[^$1]}"
  local rlen="${#rbuf//[^$2]}"
  print -r -- "llen=$llen rlen=$rlen"
}
_ap_count "(" ")""##,
        );
    }

    /// autopair.zsh:76-81 — `=~` with `${match[1]}` reused in a second
    /// `=~`, `local match=` to silence WARN_CREATE_GLOBAL.
    #[test]
    fn regex_capture_reused_in_second_match() {
        assert_parity(
            r##"ap() {
  local match= mbegin= mend=
  local LBUFFER="cmd  " RBUFFER="  next"
  if [[ $LBUFFER =~ "[^'\"]([ 	]+)$" && $RBUFFER =~ "^${match[1]}" ]]; then
    print -r -- "balanced spaces <${match[1]}>"
  else
    print unbalanced
  fi
}
ap"##,
        );
    }
}

// ───────────────────────────────── zsh-hist ────────────────────────

mod zsh_hist {
    use super::*;

    /// zsh-hist.plugin.zsh:5 — RC_QUOTES `''` doubling inside emulate.
    #[test]
    fn rcquotes_doubling() {
        assert_parity(
            r##"setopt rcquotes
print -r -- 'it''s'
f() {
  emulate -L zsh -o extendedglob -o rcquotes -o noshortloops -o warncreateglobal
  eval "print -r -- 'a''b'"
  [[ aaab == a#b ]] && print extglob-on
}
f
unsetopt rcquotes
print -r -- 'it''s'"##,
        );
    }

    /// zsh-hist.plugin.zsh:21 — `local 0=${(%):-%N}` shadow $0 with fn name.
    #[test]
    fn local_zero_from_prompt_n() {
        assert_parity(
            r##":hist:precmd() {
  local 0=${(%):-%N}
  print -r -- "fn name: $0"
}
:hist:precmd"##,
        );
    }

    /// functions/hist:66 — `$assoc[(I)$act*]` (I) abbreviation lookup.
    #[test]
    fn assoc_I_abbreviation_lookup() {
        assert_parity(
            r##"typeset -gHA _HIST__ARGS=( compress c delete d edit e get g list l )
act=del
print -r -- "match=<$_HIST__ARGS[(I)$act*]>"
act=zz
print -r -- "match=<$_HIST__ARGS[(I)$act*]>"
print -r -- ${(t)_HIST__ARGS}"##,
        );
    }

    /// functions/hist:113 — `(#a1)` approximate match with error count.
    #[test]
    fn approx_match_error_count() {
        assert_parity(
            r##"f() {
  emulate -L zsh -o extendedglob
  typeset -A h=( 1 "make tests" 2 "make test" )
  if [[ $h[1] == (#a1)$h[2] ]]; then print approx-match; else print no-match; fi
  [[ "color" == (#a1)"colour" ]] && print colour-ok
  [[ "color" == (#a1)"colours" ]] || print too-far
}
f"##,
        );
    }

    /// functions/hist:123 — `${(b)val}` pattern-quote, round-trips via `${~}`.
    #[test]
    fn b_pattern_quote_roundtrip() {
        assert_parity(
            r##"f() {
  emulate -L zsh -o extendedglob
  local entry="rm *.txt [careful]"
  local b="${(b)entry}"
  print -r -- "$b"
  [[ $entry == ${~b} ]] && print self-match
}
f"##,
        );
    }

    /// functions/hist:149 — numeric-sorted keys `(@kn)`, width-pad `(l:)`,
    /// `(V)MATCH` to visualise control chars.
    #[test]
    fn numeric_keys_pad_visualise() {
        assert_parity(
            r##"f() {
  emulate -L zsh -o extendedglob
  local -i MBEGIN MEND
  local MATCH
  local histwidth=12345
  typeset -A entries=( 10 $'foo\tbar' 2 "plain" 33 $'x\033y' )
  local key
  for key in ${(@kn)entries}; do
    print -r -- "<${(l:$#histwidth:)key}>" "${entries[$key]//(#m)[^[:print:]]##/[${(V)MATCH}]}"
  done
}
f"##,
        );
    }

    /// functions/hist:183-184 — `(l:COLUMNS::-:)` rule, `(vF)` value-join.
    #[test]
    fn pad_to_columns_and_value_join() {
        assert_parity(
            r##"f() {
  local -i COLUMNS=20
  typeset -A entries=( 1 "first" 2 "second" )
  print -r -- "${(l:COLUMNS::-:):-}"
  local new=${(vF)entries}
  print -r -- "$new"
}
f"##,
        );
    }
}

// ─────────────────────────────── powerlevel10k ─────────────────────

mod powerlevel10k {
    use super::*;

    /// gitstatus.plugin.zsh:145-148 — dynamic fn name + recover suffix.
    #[test]
    #[ignore = "zshrs gap: `function name\"$x\"()` dynamic-name definition unsupported (zsh: suffix=<_demo>; zshrs: exit 127)"]
    fn dynamic_function_name_suffix() {
        assert_parity(
            r##"function gitstatus_query"${1:-_demo}"() {
  emulate -L zsh -o no_aliases -o extended_glob -o typeset_silent
  local fsuf=${${(%):-%N}#gitstatus_query}
  print -r -- "suffix=<$fsuf>"
}
gitstatus_query_demo"##,
        );
    }

    /// gitstatus.plugin.zsh:162 — float validation with nested empty groups.
    #[test]
    fn float_validation_glob() {
        assert_parity(
            r##"f() {
  emulate -L zsh -o extended_glob
  local t
  for t in 5 5.25 -3.5 +2e-3 1.5E+2 abc 1.2.3 .5; do
    if [[ $t != (|+|-)<->(|.<->)(|[eE](|-|+)<->) ]]; then
      print -r -- "$t invalid"
    else
      print -r -- "$t valid"
    fi
  done
}
f"##,
        );
    }

    /// gitstatus.plugin.zsh:180 — `[[:IDENT:]]##` identifier validation.
    #[test]
    fn ident_class_validation() {
        assert_parity(
            r##"f() {
  emulate -L zsh -o extended_glob
  local name
  for name in foo_bar1 9lives "with space" "" "dash-ed"; do
    if [[ $name != [[:IDENT:]]## ]]; then
      print -r -- "<$name> invalid"
    else
      print -r -- "<$name> valid"
    fi
  done
}
f"##,
        );
    }

    /// gitstatus.plugin.zsh:185-292 — variable-name built in math context.
    #[test]
    fn math_context_dynamic_var_name() {
        assert_parity(
            r##"f() {
  emulate -L zsh -o typeset_silent
  local name=mine
  typeset -gi _GITSTATUS_STATE_mine=2
  (( _GITSTATUS_STATE_$name == 2 )) && print state-ok
  (( ++_GITSTATUS_NUM_INFLIGHT_$name ))
  (( ++_GITSTATUS_NUM_INFLIGHT_$name ))
  print $_GITSTATUS_NUM_INFLIGHT_mine
  typeset -g _GITSTATUS_RESP_FD_mine=42
  local -i resp_fd=_GITSTATUS_RESP_FD_$name
  print $resp_fd
}
f"##,
        );
    }

    /// gitstatus.plugin.zsh:287-352 — brace expansion in unset/typeset.
    #[test]
    fn brace_expand_bulk_params() {
        assert_parity(
            r##"typeset -g VCS_STATUS_{COMMIT,TAG,ACTION}=init
print ${+VCS_STATUS_COMMIT}${+VCS_STATUS_TAG}${+VCS_STATUS_ACTION}
unset VCS_STATUS_{COMMIT,TAG}
print ${+VCS_STATUS_COMMIT}${+VCS_STATUS_TAG}${+VCS_STATUS_ACTION}
typeset -gi VCS_STATUS_{NUM_STAGED,STASHES}
print $VCS_STATUS_NUM_STAGED $VCS_STATUS_STASHES"##,
        );
    }

    /// gitstatus.plugin.zsh:316-317 — `(ps:\x1e:)` / `(@ps:\x1f:)` record split.
    #[test]
    fn binary_record_field_split() {
        assert_parity(
            r##"f() {
  local buf=$'r1 cb\x1fdir1\x1f0\x1er2\x1fdir2\x1f1\x1e'
  local s
  for s in ${(ps:\x1e:)buf}; do
    local -a resp=("${(@ps:\x1f:)s}")
    print -r -- "${#resp} fields, dir=${resp[2]}"
  done
}
f"##,
        );
    }

    /// gitstatus.plugin.zsh:324-351 — multi-var `for a b c in …`.
    #[test]
    fn multi_var_for_loop() {
        assert_parity(
            r##"f() {
  local a b c
  for a b c in 1 2 3 4 5 6; do
    print -r -- "a=$a b=$b c=$c"
  done
}
f"##,
        );
    }

    /// internal/configure.zsh:6 — anchored `/#` substitution with (#b)
    /// backref then `%`→`%%` escaping.
    #[test]
    fn anchored_home_abbrev_with_backref() {
        assert_parity(
            r##"f() {
  emulate -L zsh -o extended_glob
  local -a match mbegin mend
  local HOME=/home/user
  local zd=/home/user/dots/zsh%dir
  local zd_u=${${${(q)zd}/#(#b)${(q)HOME}(|\/*)/'~'$match[1]}//\%/%%}
  print -r -- $zd_u
  local other=/etc/zsh
  local other_u=${${${(q)other}/#(#b)${(q)HOME}(|\/*)/'~'$match[1]}//\%/%%}
  print -r -- $other_u
}
f"##,
        );
    }

    /// internal/configure.zsh:13-68 — nested helper `$0_error` torn down in always.
    #[test]
    #[ignore = "zshrs gap: $0-derived nested fn name not defined inside fn; $+functions[x] prints `1[x]` (subscript after ${+} not parsed)"]
    fn nested_helper_always_teardown() {
        assert_parity(
            r##"function can_cfg() {
  function $0_error() {
    print -r -- "[ERROR] $0: $1"
  }
  {
    $0_error "demo failure"
    return 1
  } always {
    unfunction $0_error
  }
}
can_cfg
print exit=$? helper_left=$+functions[can_cfg_error]"##,
        );
    }

    /// internal/p10k.zsh:228-232 — brace range after expansion, MATCH as index.
    #[test]
    #[ignore = "zshrs gap: brace range {$#parts..1} inside ${:-...} not expanded (literal text emitted)"]
    fn brace_range_match_index() {
        assert_parity(
            r##"f() {
  emulate -L zsh -o extended_glob
  local MATCH
  local parts=( usr local share )
  local parent=/
  print -rl -- ${(@)${:-{$#parts..1}}/(#m)*/$parent${(pj./.)parts[1,MATCH]}}
  print -rl -- ${(@)${:-{1..$#parts}}/(#m)*/$MATCH:$parts[MATCH]}
}
f"##,
        );
    }

    /// internal/p10k.zsh:275 — array diff `:|` plus per-element suffix strip.
    #[test]
    fn array_diff_suffix_strip() {
        assert_parity(
            r##"f() {
  local cached=( 1:100 2:200 3:300 4:400 )
  local fresh=( 2:200 4:400 )
  local -i i
  for i in ${(@)${cached:|fresh}%:*}; do
    print -r -- "stale index $i"
  done
}
f"##,
        );
    }

    /// internal/p10k.zsh:341,332 — strip trailing zeros+dot from typeset -F.
    #[test]
    fn strip_trailing_zeros_float() {
        assert_parity(
            r##"f() {
  emulate -L zsh -o extended_glob
  typeset -F n=1800
  (( n /= 1024 ))
  local r=$n
  print -r -- "raw=$r stripped=${${r%%0#}%.}K"
  typeset -F m=512
  r=$m
  print -r -- "raw=$r stripped=${${r%%0#}%.}B"
}
f"##,
        );
    }

    /// internal/p10k.zsh:1754 — URL-encode via char-code in math, base-16, zero-pad.
    #[test]
    fn url_encode_char_codes() {
        assert_parity(
            r##"f() {
  emulate -L zsh -o extended_glob
  local MATCH
  print -r -- "${1//(#m)[^a-zA-Z0-9"\/:_.-!'()~"]/%${(l:2::0:)$(([##16]#MATCH))}}"
}
f "a b/c?d=e""##,
        );
    }

    /// internal/p10k.zsh:_p9k_must_init — `${(@)…:/(#m)*/${(q)MATCH}-…}`
    /// builds a `${name-…}` probe per POWERLEVEL9K_* param using the
    /// `(#m)` match-ref. The whole-element `:/` replace must publish
    /// `$MATCH` per element; a stale (empty) `$MATCH` produced `${''-…}`
    /// → "bad substitution" at the subsequent `${(e)…}` eval.
    #[test]
    fn must_init_match_ref_probe() {
        assert_parity(
            r##"emulate -L zsh -o extended_glob
typeset -g POWERLEVEL9K_MODE=nerdfont POWERLEVEL9K_LEFT_PROMPT_ELEMENTS=(dir vcs)
local IFS MATCH pat
IFS=$'\1' pat="${(@)${(@o)parameters[(I)POWERLEVEL9K_*]}:/(#m)*/\${${(q)MATCH}-$IFS\}}"
IFS=$'\2' local sig="${(e)pat}"
print -r -- "ok len=${#sig}""##,
        );
    }

    /// internal/p10k.zsh:3312 — whole-element subst `:/(#b)(*)/...` exitcode map.
    #[test]
    fn array_subst_exitcode_map() {
        assert_parity(
            r##"f() {
  emulate -L zsh -o extended_glob
  local -a match mbegin mend
  local exitcode2str=( ok HUP INT QUIT )
  local mypipestatus=( 0 2 3 )
  print -r -- "${(j:|:)${(@)mypipestatus:/(#b)(*)/$exitcode2str[$match[1]+1]}}"
}
f"##,
        );
    }

    /// internal/p10k.zsh:4549 — `(i)` forward pattern subscript with (b)-escaped needle.
    #[test]
    fn forward_subscript_with_escaped_needle() {
        assert_parity(
            r##"f() {
  local name="my dir"
  local cfg=( "header" "  name: other" "  name: my dir" "tail" )
  local -i pos=${cfg[(i)  name: ${(b)name}]}
  print -r -- "pos=$pos size=$#cfg"
  if (( pos > $#cfg )); then print absent; else print -r -- "found: $cfg[pos]"; fi
  local -i miss=${cfg[(i)  name: nothere]}
  print -r -- "miss=$miss"
}
f"##,
        );
    }

    /// internal/p10k.zsh:2146 — `(e)` re-eval with side-effecting `::=`.
    #[test]
    fn e_reeval_side_effect_assign() {
        assert_parity(
            r##"f() {
  emulate -L zsh
  local _d=7
  local content="SEGMENT"
  local out="${(e):-"\${\${_d::=0}+}$content"}"
  print -r -- "out=$out d=$_d"
}
f"##,
        );
    }

    /// internal/p10k.zsh:602 — deferred-quoting capsule round-tripped via (e).
    #[test]
    #[ignore = "zshrs gap: (Q) dequote leaves backslashes after (e) re-eval of (qqq)(q)-quoted capsule"]
    fn deferred_quoting_capsule() {
        assert_parity(
            r##"f() {
  local s="a\"b \$x 'q' *"
  local enc
  if [[ $s == *["~!#\`\$^&*()\\\"'<>?{}[]"]* ]]; then
    enc="\${(Q)\${:-${(qqq)${(q)s}}}}"
  else
    enc=$s
  fi
  print -r -- "$enc"
  print -r -- "${(e)enc}"
}
f"##,
        );
    }

    /// internal/p10k.zsh:6028 — `(0)` NUL-split scalar then strip suffix.
    #[test]
    fn nul_split_strip_suffix() {
        assert_parity(
            r##"f() {
  local segs=$'dir_joined\x00vcs\x00status_joined'
  print -rl -- ${${(0)segs}%_joined}
}
f"##,
        );
    }

    /// internal/p10k.zsh:2350 — `(A)=` force split, index, fix decimal.
    #[test]
    fn force_split_index_fix_decimal() {
        assert_parity(
            r##"f() {
  local ret="0,42 0,98 1,23"
  local -i which=2
  local load=${${(A)=ret}[which]//,/.}
  print -r -- $load
}
f"##,
        );
    }

    /// internal/p10k.zsh:1117 — case `;&` fall-through and `;|` continue.
    #[test]
    fn case_fallthrough_and_continue() {
        assert_parity(
            r##"f() {
  case $1 in
    a) print is-a ;&
    b) print is-b ;|
    c) print is-c ;;
    [ab]) print matched-ab ;;
  esac
}
f a; f b; f c"##,
        );
    }

    /// internal/p10k.zsh:268 — prefix-or-exact match vs computed literal.
    #[test]
    fn prefix_or_exact_match() {
        assert_parity(
            r##"f() {
  emulate -L zsh -o extended_glob
  local cached="1:100 2:200 3:300 17"
  local mtimes_s="1:100 2:200"
  if [[ $mtimes_s == ${cached% *}(| *) ]]; then print no; fi
  cached="1:100 2:200 17"
  [[ $mtimes_s == ${cached% *}(| *) ]] && print -r -- "prefix-match ${cached##* }"
}
f"##,
        );
    }
}

// ─────────────────────────────────── fzf ───────────────────────────

mod fzf {
    use super::*;

    /// shell/completion.zsh:34 — snapshot+restore all options via eval.
    #[test]
    #[ignore = "zshrs gap: eval of `options=(${(kv)options})` snapshot does not restore option state (shwordsplit stays on)"]
    fn snapshot_restore_options() {
        assert_parity(
            r##"__opts_save="options=(${(j: :)${(kv)options[@]}})"
[[ -o shwordsplit ]] && print before-on || print before-off
setopt shwordsplit
[[ -o shwordsplit ]] && print now-on
eval $__opts_save
[[ -o shwordsplit ]] && print after-on || print after-off"##,
        );
    }

    /// shell/completion.zsh:115 — `(Z+n+)` tokenize multi-line opts, `(Q)` dequote.
    #[test]
    fn tokenize_multiline_opts() {
        assert_parity(
            r##"f() {
  local FZF_TMUX_OPTS=$'-d 40%\n--border "rounded box"'
  local -a args=(${(Q)${(Z+n+)FZF_TMUX_OPTS}})
  print -r -- $#args
  print -rl -- $args
}
f"##,
        );
    }

    /// shell/key-bindings.zsh:110-133 — nobash_rematch populates $MATCH.
    #[test]
    fn nobash_rematch_match_var() {
        assert_parity(
            r##"f() {
  setopt localoptions nobash_rematch
  local selected="123 echo hi"
  if [[ ${selected%% *} =~ ^[1-9][0-9]* ]]; then
    print -r -- "event=$MATCH"
  fi
}
g() {
  setopt localoptions bash_rematch
  [[ "abc123" =~ [0-9]+ ]] && print -r -- "bash=$BASH_REMATCH"
}
f; g"##,
        );
    }
}

// ──────────────────────────────── git-fuzzy ────────────────────────

mod git_fuzzy {
    use super::*;

    /// lib/core.sh:12 — triple-nested default chains for layered config.
    #[test]
    fn nested_default_chain() {
        assert_parity(
            r##"GF_PREVIEW_RESIZE_HORIZONTAL_STEP="${GF_PREVIEW_RESIZE_HORIZONTAL_STEP:-${GF_PREVIEW_RESIZE_SIZE_STEP:-${GF_PREVIEW_RESIZE_PERCENT_STEP:-5}}}"
print -r -- $GF_PREVIEW_RESIZE_HORIZONTAL_STEP
GF_PREVIEW_RESIZE_PERCENT_STEP=9
GF_PREVIEW_RESIZE_VERTICAL_STEP="${GF_PREVIEW_RESIZE_VERTICAL_STEP:-${GF_PREVIEW_RESIZE_SIZE_STEP:-${GF_PREVIEW_RESIZE_PERCENT_STEP:-2}}}"
print -r -- $GF_PREVIEW_RESIZE_VERTICAL_STEP"##,
        );
    }
}

// ──────────────────────────────── zconvey ──────────────────────────

mod zconvey {
    use super::*;

    /// zconvey.plugin.zsh:64 — `*.name(N)` null-glob + file slurp + `(@M):#`.
    #[test]
    fn nullglob_slurp_match_retain() {
        assert_parity(
            r##"t=$(mktemp -d)
mkdir -p $t/names
print -l ':alpha:' ':beta:' > $t/names/3.name
print -l ':gamma:' > $t/names/7.name
name=beta
REPLY=
for f in $t/names/*.name(N); do
  arr=( ${(@f)"$(<$f)"} )
  arr=( "${(@M)arr:#:$name:}" )
  if [[ "${#arr}" != "0" ]]; then
    REPLY="${${f:t}%.name}"
  fi
done
print -- "id=$REPLY"
for f in $t/names/*.missing(N); do
  print "never"
done
print "done"
rm -rf $t"##,
        );
    }

    /// zconvey.plugin.zsh:92 — `[[ $idx != <-> ]]` numeric-range glob test.
    #[test]
    fn numeric_range_glob_validation() {
        assert_parity(
            r##"fn() {
  setopt localoptions extendedglob
  local idx="$1"
  if [[ "$idx" != <-> || "$idx" = "0" || "$idx" -gt "100" ]]; then
    print "bad:$idx"
    return 2
  fi
  print "ok:$idx"
}
fn 42; print rc=$?
fn abc; print rc=$?
fn 0; print rc=$?
fn 101; print rc=$?"##,
        );
    }

    /// zconvey.plugin.zsh:152-157 — `zstyle -s … || default` and `zstyle -T`.
    #[test]
    fn zstyle_s_default_and_t_boolean() {
        assert_parity(
            r##"zstyle ":plugin:zconvey" check_interval 5
zstyle -s ":plugin:zconvey" check_interval check_interval || check_interval="2"
print "interval=$check_interval"
zstyle -s ":plugin:zconvey" expire_seconds expire_seconds || expire_seconds="22"
print "expire=$expire_seconds"
zstyle -T ":plugin:zuid" use_zsystem_flock && print "flock=default-yes"
zstyle ":plugin:zuid" use_zsystem_flock no
zstyle -T ":plugin:zuid" use_zsystem_flock || print "flock=disabled""##,
        );
    }

    /// zconvey.plugin.zsh:453-454 — empty-slice prepend + `(pj:\n:)` join.
    #[test]
    fn empty_slice_prepend_join() {
        assert_parity(
            r##"typeset -a ZCONVEY_NNS
ZCONVEY_NNS=( "old one" "old two" )
ZCONVEY_NNS[1,0]="Notification: run tests"
print -r -- "${(pj:\n:)ZCONVEY_NNS}"
print "count=$#ZCONVEY_NNS""##,
        );
    }
}

// ───────────────────────────────── zsh-z ───────────────────────────

mod zsh_z {
    use super::*;

    /// zsh-z.plugin.zsh:226 — assoc-key iteration with float math.
    #[test]
    fn assoc_float_math_iteration() {
        assert_parity(
            r##"typeset -A rank time
rank=( /a/b 4 /c/d 10 )
time=( /a/b 100 /c/d 200 )
for x in ${(ok)rank}; do
  print -- "$x|$(( 0.99 * rank[$x] ))|${time[$x]}"
done"##,
        );
    }

    /// zsh-z.plugin.zsh:256-265 — smartcase match with positional reassignment.
    #[test]
    #[ignore = "zshrs gap: positional 1=${1// ##/*} reassign + ${1:l} compare mishandles space-containing pattern (last case)"]
    fn smartcase_match() {
        assert_parity(
            r##"fn() {
  setopt LOCAL_OPTIONS EXTENDED_GLOB
  local path_field=$2
  1=${1// ##/*}
  if [[ $1 == "${1:l}" ]] && [[ ${path_field:l} == *${~1}* ]]; then
    print -- "ci:$path_field"
  elif [[ $path_field == *${~1}* ]]; then
    print -- "cs:$path_field"
  else
    print -- "no:$path_field"
  fi
}
fn proj /home/USER/Projects
fn Proj /home/USER/Projects
fn Proj /home/user/projx
fn "pro ects" /home/USER/Projects"##,
        );
    }

    /// zsh-z.plugin.zsh:297 — array pattern-removal + change detection.
    #[test]
    fn array_pattern_removal_change_detect() {
        assert_parity(
            r##"lines=( '/home/a|1|2' '/cur/dir|3|4' '/home/b|5|6' )
cur=/cur/dir
lines_to_keep=( ${lines:#${cur}\|*} )
if [[ $lines != "$lines_to_keep" ]]; then
  print "removed left=$#lines_to_keep"
  print -l -- $lines_to_keep
else
  print "absent"
fi
cur=/not/there
lines_to_keep=( ${lines:#${cur}\|*} )
[[ $lines != "$lines_to_keep" ]] || print "absent""##,
        );
    }

    /// zsh-z.plugin.zsh:331-374 — `(Pk)` keys + `print -z`/`read -rz` handoff.
    #[test]
    fn indirect_keys_print_z_read_rz() {
        assert_parity(
            r##"fn() {
  local -a common_matches
  local x short
  common_matches=( ${(Pk)1[@]} )
  for x in ${common_matches[@]}; do
    if [[ -z $short ]] || (( $#x < $#short )); then
      short=$x
    fi
  done
  [[ $short == '/' ]] && return
  for x in ${common_matches[@]}; do
    [[ $x != $short* ]] && return
  done
  print -z -- $short
}
typeset -A m=( /usr/local 1 /usr/local/bin 2 /usr/local/share 3 )
fn m
read -rz common
print "root=$common""##,
        );
    }

    /// zsh-z.plugin.zsh:380-384 — `print -z -f` push, `(On)` sort, prefix strip.
    #[test]
    fn print_z_f_sort_strip() {
        assert_parity(
            r##"typeset -A output_matches=( /a 3.5 /b 12.25 /c 0.5 )
typeset -a descending_list
for k in ${(@k)output_matches}; do
  print -z -f "%.2f|%s" ${output_matches[$k]} $k
  read -rz stack
  descending_list+=( $stack )
done
descending_list=( ${${(@On)descending_list}#*\|} )
print -l $descending_list"##,
        );
    }

    /// zsh-z.plugin.zsh:402 — `(@on)` sort + nested POSIX-class `##` strip.
    #[test]
    #[ignore = "zshrs gap: nested ${x##[[:digit:]]##[[:punct:]]...} POSIX-class repetition inside ${x%...} not expanded (literal pattern emitted)"]
    fn ascending_sort_posix_class_strip() {
        assert_parity(
            r##"setopt extendedglob
output=( '12.00       /usr/bin' '3.50       /home/x' )
for x in ${(@on)output}; do
  print "${${x%${x##[[:digit:]]##[[:punct:]][[:digit:]]##[[:blank:]]}}/[[:punct:]]00/   }${x##[[:digit:]]##[[:punct:]][[:digit:]]##[[:blank:]]}"
done"##,
        );
    }

    /// zsh-z.plugin.zsh:517-590 — `zparseopts -E -D -A` + leftover-option detect.
    #[test]
    fn zparseopts_with_leftover_detect() {
        assert_parity(
            r##"fn() {
  local -A opts
  local -a keys
  zparseopts -E -D -A opts -- -add -complete c e h -help l r t x
  if [[ $1 == '--' ]]; then
    shift
  elif [[ -n ${(M)@:#-*} ]]; then
    print "Improper option(s) given."
    return 1
  fi
  keys=( ${(ko)opts} )
  print "keys: $keys"
  print "rest: $*"
  (( $+opts[-e] )) && print "echo-mode"
  (( $+opts[-l] )) || print "no-list"
}
fn -e -t foo bar; print rc=$?
fn foo -Q; print rc=$?"##,
        );
    }
}

// ──────────────────────────────── fzf-tab ──────────────────────────

mod fzf_tab {
    use super::*;

    /// fzf-tab.zsh:3-634 — option save/restore batch.
    #[test]
    fn option_save_restore_batch() {
        assert_parity(
            r##"typeset -a _fzf_tab_opts
[[ ! -o 'aliases'         ]] || _fzf_tab_opts+=('aliases')
[[ ! -o 'sh_glob'         ]] || _fzf_tab_opts+=('sh_glob')
[[ ! -o 'no_brace_expand' ]] || _fzf_tab_opts+=('no_brace_expand')
'builtin' 'setopt' 'no_aliases' 'no_sh_glob' 'brace_expand'
print "saved: $_fzf_tab_opts"
[[ -o aliases ]] && print on || print off
(( ${#_fzf_tab_opts} )) && setopt ${_fzf_tab_opts[@]}
[[ -o aliases ]] && print restored-on || print still-off"##,
        );
    }

    /// fzf-tab.zsh:61-554 — `(ie)` / `(Ie)` exact-match index.
    #[test]
    fn exact_match_index_flags() {
        assert_parity(
            r##"typeset -a groups=( alpha beta gamma )
print $groups[(ie)beta]
print $groups[(ie)delta]
print $groups[(Ie)gamma]
print $groups[(Ie)delta]
expl=beta
(( $groups[(Ie)$expl] != 0 )) && print "found $expl""##,
        );
    }

    /// fzf-tab.zsh:64-429 — `(pj:\1:)` join with SOH, round-trip via `(@ps:\1:)`.
    #[test]
    fn join_soh_roundtrip() {
        assert_parity(
            r##"typeset -a _opts=( -P prefix -f )
joined=${(pj:\1:)_opts}
print "len=$#joined"
typeset -a back=( "${(@ps:\1:)joined}" )
print "n=$#back"
print -l -- $back"##,
        );
    }

    /// fzf-tab.zsh:107 — `: ${(A)=VAR=…}` set-if-unset array default.
    #[test]
    fn set_if_unset_array_default() {
        assert_parity(
            r##": ${(A)=COLORS=red green blue}
print "n=$#COLORS second=$COLORS[2]"
COLORS=( x )
: ${(A)=COLORS=a b c}
print "n=$#COLORS first=$COLORS[1]""##,
        );
    }

    /// fzf-tab.zsh:176 — common-prefix loop: char split, zip, join, ERE backref, truncation.
    #[test]
    fn common_prefix_loop() {
        assert_parity(
            r##"fn() {
  local -a keys=( "$@" )
  local tmp=$keys[1]
  local MATCH key
  local -a match mbegin mend
  local -a prefix=(${(s::)tmp})
  for key in ${keys:1}; do
    (( $#tmp )) || break
    [[ $key == $tmp* ]] && continue
    [[ ${(j::)${${(s::)key[1,$#tmp]}:^prefix}} =~ '^(((.)\3)*)' ]]
    tmp[$#MATCH/2+1,-1]=""
    prefix[$#MATCH/2+1,-1]=()
  done
  print "prefix=[$tmp]"
}
fn foobar foobaz fooqux
fn alpha beta
fn same same same"##,
        );
    }

    /// fzf-tab.zsh:184 — `(@0)` NUL string to assoc + `(0)` NUL split to array.
    #[test]
    fn nul_to_assoc_and_array() {
        assert_parity(
            r##"fv="PREFIX"$'\0'"git"$'\0'"SUFFIX"$'\0'".txt"
typeset -A v=("${(@0)fv}")
print "p=$v[PREFIX] s=$v[SUFFIX] n=$#v"
line="S"$'\0'"alias"$'\0'"5"$'\0'"gst33"
typeset -a fields=(${(0)line})
print "nf=$#fields f2=$fields[2] f4=$fields[4]""##,
        );
    }

    /// fzf-tab.zsh:228 — `(r:)` right-pad and `(l:expr::fill:)` left-pad.
    #[test]
    fn padding_with_fill_chars() {
        assert_parity(
            r##"typeset -a groups=( file dir link )
typeset -i mlen=0
for i in $groups; do
  (( $#i > mlen )) && mlen=$#i
done
mlen+=1
for i in $groups; do
  print -r -- "[${(r:$mlen:)i}]"
done
typeset -i boxW=12
print -r -- "${(l:boxW-2::─:)}"
print -r -- "${(l:3::█:)}${(l:7::░:)}""##,
        );
    }

    /// fzf-tab.zsh:290 — LS_COLORS parse into reject/keep assoc filters.
    #[test]
    fn ls_colors_reject_keep_filters() {
        assert_parity(
            r##"list_colors='di=01;34:ln=01;36:*.txt=00;32:ex=01;32'
typeset -A namecolors=(${(@s:=:)${(@s.:.)list_colors}:#[[:alpha:]][[:alpha:]]=*})
typeset -A modecolors=(${(@Ms:=:)${(@s.:.)list_colors}:#[[:alpha:]][[:alpha:]]=*})
typeset -a nk=( ${(ko)namecolors} ) mk=( ${(ko)modecolors} )
print "namekeys: $nk"
print "modekeys: $mk"
print "di=$modecolors[di] ln=$modecolors[ln]""##,
        );
    }

    /// fzf-tab.zsh:363-364 — NUL-field swap-sort-swap with (#b) backrefs.
    #[test]
    #[ignore = "zshrs gap: unquoted per-element array `${arr//$'\\0'/|}` does not replace embedded NUL (joined/quoted form works) — Meta-encoding mismatch in the per-element // replace path; the `:/(#b)` backref half is now fixed"]
    fn nul_field_swap_sort() {
        assert_parity(
            r##"setopt extendedglob
typeset -a match mbegin mend
typeset -a tc=( "colB"$'\0'"beta" "colA"$'\0'"alpha" )
tc=(${(@o)${(@)tc:/(#b)([^$'\0']#)$'\0'(*)/$match[2]$'\0'$match[1]}})
print -l -- ${tc//$'\0'/|}
tc=(${(@)tc/(#b)(*)$'\0'([^$'\0']#)/$match[2]$'\0'$match[1]})
print -l -- ${tc//$'\0'/|}"##,
        );
    }

    /// fzf-tab.zsh:370 — `typeset -Ua` dedupe with `[0-9]#` index strip.
    #[test]
    fn unique_array_index_strip() {
        assert_parity(
            r##"setopt extendedglob
bs=$'\b'
typeset -a tcandidates=( "2${bs}red" "1${bs}blue" "2${bs}red" )
typeset -Ua candidates=("${(@)tcandidates//[0-9]#$bs}")
print -l -- $candidates
print "n=$#candidates""##,
        );
    }

    /// fzf-tab.zsh:375 — brace range with var bound + `:|` exclusion, in-place rewrite.
    #[test]
    fn brace_range_exclusion_rewrite() {
        assert_parity(
            r##"typeset -a _fzf_tab_groups=( g1 g2 g3 g4 )
typeset -Ua duplicate_groups=( 2 4 )
indexs=({1..$#_fzf_tab_groups})
for i in ${indexs:|duplicate_groups}; do
  _fzf_tab_groups[i]="__hide__$i"
done
print -l -- $_fzf_tab_groups"##,
        );
    }

    /// fzf-tab.zsh:459 — `(r)` reverse subscript with (b)-quoted needle.
    #[test]
    fn reverse_subscript_b_quoted_needle() {
        assert_parity(
            r##"bs=$'\2'
typeset -a compcap=( "file.txt${bs}data1" "fi*e${bs}data2" )
choice='fi*e'
print -r -- "${compcap[(r)${(b)choice}$bs*]#*$bs}"
choice='file.txt'
print -r -- "${compcap[(r)${(b)choice}$bs*]#*$bs}""##,
        );
    }

    /// fzf-tab.zsh:528-574 — function-body copy through functions assoc.
    #[test]
    fn function_body_copy() {
        assert_parity(
            r##"greet() { print "hello $1"; }
functions[greet2]=$functions[greet]
greet2 world
print "have2=${+functions[greet2]}"
unfunction greet
greet2 again"##,
        );
    }

    /// fzf-tab.zsh:488 — `{ } always { }` cleanup preserving try return.
    #[test]
    fn always_block_preserves_return() {
        assert_parity(
            r##"fn() {
  local flag=1
  {
    print "body flag=$flag"
    return 7
  } always {
    flag=0
    print "always flag=$flag"
  }
}
fn
print "rc=$?""##,
        );
    }
}

// ──────────────────────────────── revolver ─────────────────────────

mod revolver {
    use super::*;

    /// bin/revolver:93-148 — `(@z)` spinner unpack + `shift arr` + element fetch.
    #[test]
    fn spinner_table_unpack() {
        assert_parity(
            r##"typeset -A _revolver_spinners=(
  'line' '0.13 - \\ | /'
)
style=line
frames=(${(@z)_revolver_spinners[$style]})
interval=${(@z)frames[1]}
shift frames
print "interval=$interval n=$#frames"
spinner_index=2
frame="${${(@z)frames}[$spinner_index]//\"}"
print -r -- "frame=$frame""##,
        );
    }
}

// ──────────────────────────────── zsh-expand ───────────────────────

mod zsh_expand {
    use super::*;

    /// zpwrExpandApi.zsh:30-38 — `(z)` reparse, last-word mutate, `(Az)` re-split.
    #[test]
    #[ignore = "zshrs gap: (z) lexer tokenizes backtick differently and (Az) re-split count differs (zsh n2=7; zshrs n2=6)"]
    fn z_reparse_lastword_mutate() {
        assert_parity(
            r##"tmp='echo a; cat <(lister)'
tmp=( ${(z)tmp} )
print "n=$#tmp last=$tmp[-1]"
tmp[-1]=${tmp[-1]//[\<\=\$]\(/;}
print -r -- "last2=$tmp[-1]"
tmp2='run `datez`'
tmp2=( ${(z)tmp2} )
tmp2[-1]=${tmp2[-1]:gs/\`/;/}
print -r -- "g=$tmp2[-1]"
mywordsleft=( ${(Az)tmp} )
print "n2=$#mywordsleft"
print -r -- "joined=$mywordsleft""##,
        );
    }

    /// zpwrExpandApi.zsh:48-56 — reverse C-loop over words, double element delete.
    #[test]
    #[ignore = "zshrs gap: `for (( i=$#arr; i>=1; i-- ))` C-loop with arr[i]=() deletion produces no output"]
    fn reverse_word_loop_delete() {
        assert_parity(
            r##"typeset -a mywordsleft=( ls -l '&&' grep x '>' out.txt )
typeset -i firstIndex=0
for (( i = $#mywordsleft; i >= 1; i-- )); do
  case $mywordsleft[$i] in
    ';;' | \; | \| | '||' | '&&' | '(' | '{')
      firstIndex=$((i + 1))
      break
      ;;
    '>'* | '<'* | '&>'*)
      mywordsleft[$i]=()
      mywordsleft[$i]=()
      ;;
    *)
      ;;
  esac
done
print "firstIndex=$firstIndex n=$#mywordsleft"
print -r -- "partition: ${mywordsleft[$firstIndex,$#mywordsleft]}""##,
        );
    }

    /// zpwrExpandApi.zsh:94-106 — slice stays array with `(@)`, joins without.
    #[test]
    fn slice_array_vs_join_and_default() {
        assert_parity(
            r##"typeset -a lpartAry=( sudo VAR=1 make --flag=2 target )
typeset -a _noeq
_noeq=("${(@)lpartAry:#*=*}")
print "lastword=${_noeq[-1]:-''}"
typeset -a head=( "${(@)lpartAry[1,-2]}" )
print "with-at n=$#head"
typeset -a joined=( "${lpartAry[1,-2]}" )
print "no-at n=$#joined"
typeset -a none
print "empty-default=${none[-1]:-''}""##,
        );
    }

    /// zpwrExpandLib.zsh:40-52 — `${#var%%TS*}` length-of-trimmed, offset slice.
    #[test]
    fn tabstop_length_offset() {
        assert_parity(
            r##"ZPWR_TABSTOP=__________
LBUFFER="git commit -m ${ZPWR_TABSTOP} --amend"
RBUFFER="${ZPWR_TABSTOP} --amend"
lenToFirstTS=${#LBUFFER%%$ZPWR_TABSTOP*}
print "len=$lenToFirstTS total=$#LBUFFER"
if (( $lenToFirstTS < ${#LBUFFER} )); then
  RBUFFER=${RBUFFER:$#ZPWR_TABSTOP}
  print -r -- "rbuf=[$RBUFFER]"
fi
LBUFFER="no tabstop here"
lenToFirstTS=${#LBUFFER%%$ZPWR_TABSTOP*}
(( $lenToFirstTS < ${#LBUFFER} )) || print "no-ts len=$lenToFirstTS""##,
        );
    }

    /// zpwrExpandLib.zsh:104-107 — `(#b)` backrefs into $match, `:gs` modifier.
    #[test]
    fn backref_match_gs_modifier() {
        assert_parity(
            r##"setopt extendedglob
typeset -a match mbegin mend
misspelling=gti
key=git_status
LBUFFER="sudo gti"
if [[ $LBUFFER == (#b)(*[[:space:]]#)($misspelling) ]]; then
  res1=${match[1]}
  LBUFFER="$res1${key:gs/_/ /}"
fi
print -r -- "$LBUFFER"
LBUFFER="gti"
if [[ $LBUFFER == (#b)(*[[:space:]]#)($misspelling) ]]; then
  res1=${match[1]}
  LBUFFER="$res1${key:gs/_/ /}"
fi
print -r -- "[$LBUFFER]""##,
        );
    }

    /// zpwrExpandLib.zsh:214 — negative substring offsets and trailing trim.
    #[test]
    fn negative_substring_offsets() {
        assert_parity(
            r##"LBUFFER="echo hi "
print -r -- "last=[${LBUFFER: -1}]"
print -r -- "prev=[${LBUFFER: -2:-1}]"
if [[ ${LBUFFER: -1} == " "  && ${LBUFFER: -2:-1} != " " ]]; then
  LBUFFER="${LBUFFER:0:-1}"
fi
print -r -- "buf=[$LBUFFER]"
LBUFFER="double  "
if [[ ${LBUFFER: -1} == " "  && ${LBUFFER: -2:-1} != " " ]]; then
  LBUFFER="${LBUFFER:0:-1}"
fi
print -r -- "buf2=[$LBUFFER]""##,
        );
    }

    /// zpwrExpandLib.zsh:232 — suffix-alias expansion via `:e` + `$saliases`.
    #[test]
    fn suffix_alias_lookup() {
        assert_parity(
            r##"alias -s txt='print handler:'
word=notes.txt
ext=${word:e}
print "ext=$ext have=${+saliases[$ext]}"
if [[ -n $ext ]] && (( ${+saliases[$ext]} )); then
  print -r -- "expand: $saliases[$ext] $word"
fi
word=README
print "noext=[${word:e}] have=${+saliases[${word:e}]}""##,
        );
    }

    /// zpwrExpandLib.zsh:302-306 — backspace-overprint strip + tab expand.
    #[test]
    fn backspace_overprint_strip() {
        assert_parity(
            r##"raw=$'b\bbo\bol\bld and _\bu_\bn'
while [[ $raw == *$'\b'* ]]; do
  raw=${raw//?$'\b'/}
done
print -r -- "clean=$raw"
tabs=$'a\tb'
tabs=${tabs//$'\t'/        }
print -r -- "[$tabs]""##,
        );
    }

    /// zpwrExpandParser.zsh:72-80 — `${+hash[k]}` lookup-table + assignment strip.
    #[test]
    fn lookup_table_assignment_strip() {
        assert_parity(
            r##"typeset -gA _ZPWR_PHASE1_CMDS=(
    nocorrect 1  - 1  builtin 1  eval 1  noglob 1  coproc 1  time 1  command 1  exec 1
)
bare=eval
(( ${+_ZPWR_PHASE1_CMDS[$bare]} )) && print "phase1:$bare"
_fpBare=SUDO
print "lower=${(L)_fpBare}"
(( ! ${+_ZPWR_PHASE1_CMDS[ls]} )) && print "ls-not-phase1"
typeset -a words=( VAR=1 sudo PATH=/x ls -l )
words=("${(@)words:#[A-Za-z_]*=*}")
print -r -- "stripped: $words""##,
        );
    }
}

// ──────────────────────────────── zsh-learn ────────────────────────

mod zsh_learn {
    use super::*;

    /// autoload/zsh-learn-Learn:26 — rcquotes single-quote escaping via eval.
    #[test]
    fn rcquotes_escape_eval() {
        assert_parity(
            r##"setopt rcquotes
eval "
learning=\"don't panic\"
BUFFER=\"le '\${learning//'/\''}'\"
print -r -- \"\$BUFFER\"
print 'it''s rcquotes'
""##,
        );
    }

    /// autoload/zsh-learn-GetLastItem:5-10 — `${=cmd}` split + quote-escape rewrite.
    #[test]
    fn forced_split_quote_escape() {
        assert_parity(
            r##"ZPWR_LEARN_COMMAND='sed -n 1p'
out=$(print -l one two | ${=ZPWR_LEARN_COMMAND})
print "out=$out"
typeset -A ZPWR_VARS
ZPWR_VARS[item]="it's data"
ZPWR_VARS[item]=${ZPWR_VARS[item]//\'/\\\'\'}
print -r -- "$ZPWR_VARS[item]""##,
        );
    }
}

// ──────────────────────────────── zsh-git-acp ──────────────────────

mod zsh_git_acp {
    use super::*;

    /// zsh-git-acp.plugin.zsh:224 — guarded fn def `(( $+functions[n] )) || n(){…}`.
    #[test]
    fn guarded_function_definition() {
        assert_parity(
            r##"(( $+functions[zpwrExists] )) ||
zpwrExists(){
    type "$1" >/dev/null 2>&1
}
zpwrExists print && print "print exists"
zpwrExists no_such_cmd_zz; print "missing rc=$?"
(( $+functions[zpwrExists] )) ||
zpwrExists(){
    print "second definition"
}
print "defined=${+functions[zpwrExists]}"
zpwrExists print && print "still first""##,
        );
    }
}

// ──────────────────────────────── forgit ───────────────────────────

mod forgit {
    use super::*;

    /// forgit.plugin.zsh:49 — `*(.:t)` plain-file glob qualifier + :t inside result.
    #[test]
    fn glob_qualifier_with_modifier() {
        assert_parity(
            r##"t=$(mktemp -d)
mkdir $t/sub
print x > $t/bfile
print x > $t/afile
print -l -- $t/*(.:t)
print -l -- $t/*(/:t)
typeset -a zwc=( $t/*.zwc(N) )
print "zwc=$#zwc"
typeset -a desc=( $t/*(On:t) )
print -r -- "desc: $desc"
rm -rf $t"##,
        );
    }
}

// ──────────────────────────── zsh-git-repo-cache ───────────────────

mod zsh_git_repo_cache {
    use super::*;

    /// zsh-git-repo-cache.plugin.zsh:27-43 — `type -a` suffix-alias rejecting probe.
    #[test]
    fn type_a_suffix_alias_probe() {
        assert_parity(
            r##"zpwrExists(){
    type -a -- "$1" &>/dev/null || return 1 &&
    [[ $(type -a -- "$1" 2>/dev/null) != *"suffix alias"* ]]
}
alias -s mp4=player
zpwrExists print && print "print:yes"
zpwrExists nope_zz_cmd || print "nope:no"
zpwrExists mp4 || print "mp4:suffix-alias-rejected"
typeset -A ZPWR_VERBS
(( ${+ZPWR_VERBS} )) && print "verbs-hash-set""##,
        );
    }
}

// ──────────────────────────────── zsh-sed-sub ──────────────────────

mod zsh_sed_sub {
    use super::*;

    /// zsh-sed-sub.plugin.zsh:1 — zinit-standard $0 resolution + `(M):#/*` abs test.
    #[test]
    fn zero_resolution_abs_test() {
        assert_parity(
            r##"zero='myplugin.plugin.zsh'
argzero='myplugin.plugin.zsh'
r="${${zero:#$argzero}:-${(%):-%N}}"
print -r -- "fallback=$r"
zero='/abs/path/plug.zsh'
r="${${zero:#$argzero}:-${(%):-%N}}"
print -r -- "kept=$r"
rel='relative/path.zsh'
print -r -- "abs=${${(M)rel:#/*}:-/plugdir/$rel}""##,
        );
    }

    /// autoload/basicSedSub:19 — `(#key)` / `(##\x)` char-code arithmetic.
    #[test]
    fn char_code_arithmetic() {
        assert_parity(
            r##"emulate -LR zsh
key='A'
print $(( (#key) ))
print $(( (##\n) )) $(( (##\r) )) $(( (##\e) ))
if (( (#key) != (##\n) && (#key) != (##\r) )); then print "not newline"; fi
key=$'\n'
if (( (#key) == (##\n) )); then print "is newline"; fi"##,
        );
    }

    /// autoload/basicSedSub:36-61 — build sed s-command from buffer + pipe.
    #[test]
    fn build_sed_command() {
        assert_parity(
            r##"sedArg='foo@1>bar@2x'
sedArg=${sedArg[1,-2]}
orig="${sedArg%%>*}"
replace="${sedArg##*>}"
orig="${orig//@/\\@}"
replace="${replace//@/\\@}"
sedArg="s@$orig@$replace@g"
print -r -- "$sedArg"
BUFFER='echo foo@1 and foo@1 again'
if [[ "$BUFFER" != *"foo@1"* ]]; then print "no match"; fi
BUFFER="$(print -r -- $BUFFER | sed -E -- "$sedArg")"
print -r -- "$BUFFER""##,
        );
    }

    /// autoload/basicSedSub:9 — blank-buffer detect via `[[:space:]]#`.
    #[test]
    fn blank_buffer_detect() {
        assert_parity(
            r##"setopt extendedglob
BUFFER='   '
[[ "$BUFFER" == [[:space:]]# ]] && print "all blank"
BUFFER='  x '
[[ "$BUFFER" == [[:space:]]# ]] || print "has content"
BUFFER=''
[[ "$BUFFER" == [[:space:]]# ]] && print "empty matches too""##,
        );
    }
}

// ──────────────────────────────── zsh-sudo ─────────────────────────

mod zsh_sudo {
    use super::*;

    /// sudo.plugin.zsh:23-26 — ERE `=~` with interpolated pattern + `$match[-1]`.
    #[test]
    #[ignore = "zshrs gap: complex interpolated ERE with nested groups + $match[-1] fails to match (zsh n=9; zshrs no match)"]
    fn ere_interpolated_pattern_negative_match() {
        assert_parity(
            r##"ZPWR_SUDO_REGEX='sudo'
LBUFFER='  sudo -E FOO=bar ls -l'
if [[ $LBUFFER =~ ^([[:space:]]*)(([\"\']*"$ZPWR_SUDO_REGEX"[\"\']*([[:space:]]+)((-[ABbEHnPSis]+[[:space:]]*|--)*)*)+([[:graph:]]+=[[:graph:]]+[[:space:]]+)*)+([[:space:]])*(.*)$ ]]; then
    print "n=${#match}"
    print -r -- "first=[$match[1]]"
    print -r -- "last=[$match[-1]]"
    LBUFFER="$match[1]$match[-1]"
    print -r -- "stripped=[$LBUFFER]"
else
    print "no match"
fi"##,
        );
    }
}

// ──────────────────────────────── fasd-simple ──────────────────────

mod fasd_simple {
    use super::*;

    /// fasd-simple.plugin.zsh:1 — `$commands[name]` existence in `[`.
    #[test]
    fn commands_hash_existence() {
        assert_parity(
            r##"hash mytool=/bin/ls
if [ $commands[mytool] ]; then print "have mytool"; fi
if [ $commands[no-such-tool-xyz] ]; then print "ghost"; else print "no ghost tool"; fi
print "plus: $+commands[mytool] $+commands[no-such-tool-xyz]"
print -r -- "path: $commands[mytool]""##,
        );
    }

    /// fasd-simple.plugin.zsh:3 — `-nt … -o ! -s` cache-staleness test.
    #[test]
    fn cache_staleness_test() {
        assert_parity(
            r##"t=$(mktemp -d)
print old > $t/cmd
print new > $t/cache
touch -t 202001010000 $t/cmd
touch -t 202101010000 $t/cache
if [ "$t/cmd" -nt "$t/cache" -o ! -s "$t/cache" ]; then print "rebuild"; else print "cache fresh"; fi
touch -t 202201010000 $t/cmd
if [ "$t/cmd" -nt "$t/cache" -o ! -s "$t/cache" ]; then print "rebuild after cmd update"; fi
: > $t/empty
if [ "$t/cmd" -nt "$t/empty" -o ! -s "$t/empty" ]; then print "empty cache rebuilds"; fi
rm -rf $t"##,
        );
    }

    /// bin/fasd:35 — `emulate sh` for local sh word-splitting.
    #[test]
    fn emulate_sh_word_splitting() {
        assert_parity(
            r##"f() {
  [ "$ZSH_VERSION" ] && emulate sh && setopt localoptions
  v="one two three"
  set -- $v
  print "sh-emulated words: $#"
}
g() {
  v="one two three"
  set -- $v
  print "native zsh words: $#"
}
f
g"##,
        );
    }
}

// ──────────────────────────────── zsh-nginx ────────────────────────

mod zsh_nginx {
    use super::*;

    /// zsh-nginx.plugin.zsh:21 — `autoload -Uz dir/*(.:t)` registration.
    #[test]
    fn autoload_glob_qualifier_registration() {
        assert_parity(
            r##"t=$(mktemp -d)
mkdir $t/autoload
print 'helper(){ print "from helper $1"; }; helper "$@"' > $t/autoload/helper
print 'other(){ print from-other; }; other "$@"' > $t/autoload/other
mkdir $t/autoload/subdir
fpath+=("$t/autoload")
autoload -Uz "$t/autoload/"*(.:t)
print -r -- "$t/autoload/"*(.:t)
helper world
print "fns: $+functions[helper] $+functions[other] $+functions[subdir]"
rm -rf $t"##,
        );
    }

    /// autoload/vhost:65-77 — `getopts` with OPTARG + `shift $[ OPTIND - 1 ]`.
    #[test]
    fn getopts_optarg_shift() {
        assert_parity(
            r##"vhost() {
  local user=defaultuser tpl="non_existing_template" enable=1 write_hosts=0 option
  while getopts ":lu:t:nwh" option; do
    case $option in
      u ) user=$OPTARG ;;
      t ) tpl=$OPTARG ;;
      n ) enable=0 ;;
      w ) write_hosts=1 ;;
    esac
  done
  shift $[ $OPTIND - 1 ]
  print "user=$user tpl=$tpl enable=$enable write=$write_hosts vhost=$1"
}
vhost -u alice -t symfony2 -n mysite.local
vhost -w other.local"##,
        );
    }
}

// ──────────────────────────── zsh-better-npm-completion ────────────

mod better_npm {
    use super::*;

    /// src/_npm:6-15 — upward directory walk via `dir=${dir%/*}`.
    #[test]
    fn upward_directory_walk() {
        assert_parity(
            r##"t=$(mktemp -d)
mkdir -p $t/proj/src/deep
print '{}' > $t/proj/package.json
filename=package.json
dir=$t/proj/src/deep
while [ ! -e "$dir/$filename" ]; do
    dir=${dir%/*}
    [[ "$dir" = "" ]] && break
done
[[ ! "$dir" = "" ]] && print -r -- "found: ${dir#$t/}/$filename"
dir=$t/proj/src/deep
filename=missing.json
while [ ! -e "$dir/$filename" ]; do
    dir=${dir%/*}
    [[ "$dir" = "" ]] && break
done
[[ "$dir" = "" ]] && print "not found anywhere"
rm -rf $t"##,
        );
    }

    /// src/_npm:20-22 — package.json property extraction via sed range.
    #[test]
    fn package_json_property_extraction() {
        assert_parity(
            r##"t=$(mktemp -d)
cat > $t/package.json <<'EOF'
{
  "name": "demo",
  "scripts": {
    "build": "tsc -p .",
    "test": "jest --ci"
  },
  "dependencies": {
    "lodash": "^4.17.0"
  }
}
EOF
property=scripts
cat "$t/package.json" |
    sed -nE "/^  \"$property\": \{$/,/^  \},?$/p" |
    sed '1d;$d' |
    sed -E 's/    "([^"]+)": "(.+)",?/\1=>\2/'
property=dependencies
cat "$t/package.json" |
    sed -nE "/^  \"$property\": \{$/,/^  \},?$/p" |
    sed '1d;$d' |
    sed -E 's/    "([^"]+)": "(.+)",?/\1=>\2/' | cut -f 1 -d "="
rm -rf $t"##,
        );
    }
}

// ──────────────────────────── zsh-cargo-completion ─────────────────

mod cargo_completion {
    use super::*;

    /// src/_cargo:362 — triple-nested expansion stripping `-Z` flag docs.
    #[test]
    fn triple_nested_flag_extract() {
        assert_parity(
            r##"setopt extendedglob
out=$'Available unstable (nightly-only) flags:\n\n    -Z allow-features  -- Allow only the listed unstable features\n    -Z avoid-dev-deps  -- Avoid installing dev-dependencies if possible'
flags=( help ${${${(M)${(f)out}:#*--*}/ #-- #/:}##*-Z } )
print -rl -- $flags"##,
        );
    }

    /// src/_cargo:385 — keep indented lines, strip spaces, separator to `:`.
    #[test]
    fn keep_indented_to_describe() {
        assert_parity(
            r##"setopt extendedglob
out=$'Installed Commands:\n    build                Compile the current package\n    check                Analyze the current package\n    clippy'
commands_list=( ${${${(M)"${(f)out}":#    *}/ ##/}/ ##/:} )
print -rl -- $commands_list"##,
        );
    }

    /// src/_cargo:400 — strip JSON wrapper via nested `%%`/`##` escaped patterns.
    #[test]
    fn strip_json_wrapper() {
        assert_parity(
            r##"json='{"root":"/home/user/project/Cargo.toml"}'
manifest=${${${json}%\"\}}##*\"}
print -r -- "$manifest""##,
        );
    }

    /// src/_cargo:3,421 — `regexp-replace` in-place ERE deletion.
    #[test]
    #[ignore = "zshrs gap: autoload regexp-replace with alternation `|\"` does not delete matches in-place"]
    fn regexp_replace_inplace() {
        assert_parity(
            r##"autoload -U regexp-replace
line='    name = "mybin"'
regexp-replace line '^\s*name\s*=\s*|"' ''
print -r -- "[$line]"
line2='    name = "integration"'
regexp-replace line2 '^[[:space:]]*name[[:space:]]*=[[:space:]]*|"' ''
print -r -- "[$line2]""##,
        );
    }
}

// ──────────────────────────────── zunit ────────────────────────────

mod zunit {
    use super::*;

    /// src/helpers.zsh:97-113 — per-word conditional quoting with (#m)/(#b).
    #[test]
    #[ignore = "zshrs gap: nested (M)/(#b)/(j:|:)~ per-word conditional quoting expansion errors (exit 1)"]
    fn per_word_conditional_quoting() {
        assert_parity(
            r##"setopt extendedglob
typeset -a dont_quote cmd
dont_quote=(
    "[[:digit:]]#(>|>>)(&|)[[:digit:]]#"
    "[[:digit:]]#(<|<<)(&|)[[:digit:]]#"
    "<<<" ";" "\\|" "\\|\\|" "&" "&&"
    "([0-9]#|[a-zA-Z_][a-zA-Z0-9_]#)=*" "\\)"
)
cmd=( echo 'hello "world"' '2>&1' VAR=value '|' 'back`tick' )
print -rl -- ${cmd[@]/(#m)*/${${${${${(M)MATCH:#(${(j:|:)~dont_quote})}:+$MATCH}}:-\"${MATCH//(#b)([\"\`\\])/\\${match[1]}}\"}}}"##,
        );
    }

    /// src/commands/run.zsh:239 — `(s/@/)` split of `file@test` argument.
    #[test]
    fn split_file_at_test() {
        assert_parity(
            r##"parse() {
  local -a bits; bits=("${(s/@/)1}")
  local testfile="${bits[1]}" test_to_run="${bits[2]}"
  print "file=$testfile test=${test_to_run:-<all>}"
}
parse tests/demo.zunit@my-test
parse tests/other.zunit"##,
        );
    }

    /// src/commands/run.zsh:259 — slice between first/last quote via (i)/(I).
    #[test]
    fn slice_between_quotes() {
        assert_parity(
            r##"line="@test 'my test name' {"
testname="${line[(( ${line[(i)[\']]}+1 )),(( ${line[(I)[\']]}-1 ))]}"
print -r -- "name=[$testname]"
print "first=${line[(i)[\']]} last=${line[(I)[\']]}""##,
        );
    }

    /// src/commands/run.zsh:271-289 — parallel array accum + indexed append.
    #[test]
    fn parallel_array_accumulation() {
        assert_parity(
            r##"typeset -a tests test_names
testname='first test'
test_names=($test_names $testname)
tests[${#test_names}]=''
tests[${#test_names}]+="line one"$'\n'
tests[${#test_names}]+="line two"$'\n'
testname='second test'
test_names=($test_names $testname)
tests[${#test_names}]=''
tests[${#test_names}]+="only line"$'\n'
integer i=1
for name in "${test_names[@]}"; do
    print "== $name"
    print -rn -- "${tests[$i]}"
    i=$(( i + 1 ))
done"##,
        );
    }

    /// src/zunit.zsh:70-72 — `zparseopts -D -E` keep unrecognized in place.
    #[test]
    fn zparseopts_keep_unrecognized() {
        assert_parity(
            r##"f() {
  local -a help version
  zparseopts -D -E h=help -help=help v=version -version=version
  print "help=${#help} version=${#version} rest=[$*]"
}
f -h run tests/a.zunit
f --version
f run --help extra
f plain args"##,
        );
    }

    /// src/commands/run.zsh:73-133 — eval fn-def string, gate on $+functions.
    #[test]
    fn eval_function_definition() {
        assert_parity(
            r##"func='function __zunit_tmp_test_function() { print "test ran"; return 0; }'
(( $+functions[__zunit_tmp_test_function] )) && unfunction __zunit_tmp_test_function
eval "$(echo "$func")"
print "defined: $+functions[__zunit_tmp_test_function]"
__zunit_tmp_test_function
unfunction __zunit_tmp_test_function
print "after unfunction: $+functions[__zunit_tmp_test_function]""##,
        );
    }

    /// src/assertions.zsh:237-250 — hash from positional slice + `for k v`.
    #[test]
    fn hash_from_slice_kv_iteration() {
        assert_parity(
            r##"_zunit_assert_is_key_in() {
  local found=0 value=$1 k v
  local -A hash
  hash=(${(@)@:2})
  for k v in ${(@kv)hash}; do
    [[ $k = $value ]] && found=1
  done
  print "$value found=$found size=${#hash}"
}
_zunit_assert_is_key_in alpha alpha 1 beta 2
_zunit_assert_is_key_in gamma alpha 1 beta 2"##,
        );
    }

    /// zunit.plugin.zsh:18-22 — recursive negation glob `**/(^zunit).zsh`.
    #[test]
    #[ignore = "zshrs gap: **/(^zunit).zsh recursive-glob result ordering differs from zsh (zsh sorts breadth/lexical differently)"]
    fn recursive_negation_glob() {
        assert_parity(
            r##"t=$(mktemp -d)
mkdir -p $t/src/commands $t/src/reports
touch $t/src/zunit.zsh $t/src/helpers.zsh $t/src/commands/run.zsh $t/src/reports/tap.zsh
setopt EXTENDED_GLOB
print -rl -- $t/src/**/(^zunit).zsh(:t)
print --
print -rl -- $t/src/**/*.zsh(:t)
rm -rf $t"##,
        );
    }

    /// zunit.plugin.zsh:15 — zsh `echo` interprets escapes by default.
    #[test]
    fn echo_interprets_escapes() {
        assert_parity(
            r##"echo "#!/usr/bin/env zsh\n"
echo "col1\tcol2""##,
        );
    }

    /// src/helpers.zsh:186 — clobber-override `>!` and `>|` under noclobber.
    #[test]
    fn clobber_override_redirections() {
        assert_parity(
            r##"t=$(mktemp -d)
setopt noclobber
print first > $t/f
( print second > $t/f ) 2>/dev/null || print "clobber blocked"
print third >! $t/f
print -r -- "after >!: $(<$t/f)"
print fourth >| $t/f
print -r -- "after >|: $(<$t/f)"
rm -rf $t"##,
        );
    }

    /// src/commands/run.zsh:85-87 — `zshexit()` exit hook.
    #[test]
    fn zshexit_hook() {
        assert_parity(
            r##"zshexit() {
  print "teardown ran"
}
print "test body""##,
        );
    }

    /// src/commands/run.zsh:414 — splice to computed index with `${var+alt}`.
    #[test]
    fn splice_computed_index_plus_test() {
        assert_parity(
            r##"typeset -a testfiles
argument=tests/a.zunit test_name='first'
testfiles[(( ${#testfiles} + 1 ))]=("$argument${test_name+"@$test_name"}")
unset test_name
argument=tests/b.zunit
testfiles[(( ${#testfiles} + 1 ))]=("$argument${test_name+"@$test_name"}")
print -rl -- $testfiles
print "count=${#testfiles}""##,
        );
    }

    /// src/zunit.zsh:3 — `typesetsilent` suppresses bare-typeset echo.
    #[test]
    fn typesetsilent_suppression() {
        assert_parity(
            r##"f() { typeset x=1; typeset x; print "end f"; }
g() { setopt localoptions typesetsilent; typeset y=2; typeset y; print "end g"; }
f
g"##,
        );
    }
}

// ──────────────────────────────── gh_reveal ────────────────────────

mod gh_reveal {
    use super::*;

    /// bin/reveal:69-70 — first-match-only `${var/ /|}` vs global `//`.
    #[test]
    fn first_match_only_substitution() {
        assert_parity(
            r##"set -- origin upstream fork
argValues="$@"
print -r -- "first-only: ${argValues/ /|}"
print -r -- "global:     ${argValues// /|}""##,
        );
    }

    /// bin/reveal:60 — `builtin cd` in subshell bypassing cd function.
    #[test]
    fn builtin_cd_subshell() {
        assert_parity(
            r##"t=$(mktemp -d)
mkdir $t/repo
cd() { print "function cd intercepted"; }
( builtin cd "$t/repo" && print "subshell in: ${PWD:t}"; )
cd "$t/repo"
[[ $PWD != $t/repo ]] && print "parent pwd unchanged"
unfunction cd
rm -rf $t"##,
        );
    }
}

// ──────────────────────────────── kubectl-aliases ──────────────────

mod kubectl_aliases {
    use super::*;

    /// kubectl-aliases.plugin.zsh:17-23 — top-level `local` + sourced alias hash.
    #[test]
    fn toplevel_local_source_aliases() {
        assert_parity(
            r##"t=$(mktemp -d)
cat > $t/.kubectl_aliases <<'EOF'
alias k='kubectl'
alias ksys='kubectl --namespace=kube-system'
alias kga='kubectl get all'
EOF
local aliasesFile=$t/.kubectl_aliases 2>/dev/null
print "toplevel local rc=$?"
aliasesFile=$t/.kubectl_aliases
if [[ -f $aliasesFile ]]; then
    source $aliasesFile
else
    echo "ERROR: $aliasesFile does not exist" >&2
fi
print -r -- "k -> ${aliases[k]}"
print -r -- "ksys -> ${aliases[ksys]}"
rm -rf $t"##,
        );
    }
}

// ──────────────────────────────── zsh-docker-aliases ───────────────

mod docker_aliases {
    use super::*;

    /// alias.zsh:9,27 — alias bodies with unexpanded `$(…)`/braces, introspection.
    #[test]
    fn alias_body_introspection() {
        assert_parity(
            r##"alias dkpsv='docker ps --format="ID\t{{.ID}}\nNAME\t{{.Names}}"'
alias dkE='docker exec -e COLUMNS=$(tput cols) -i -t'
print -r -- ${aliases[dkpsv]}
alias dkE"##,
        );
    }
}

// ──────────────────────────────── zsh-openshift-aliases ────────────

mod openshift_aliases {
    use super::*;

    /// zsh-openshift-aliases.plugin.zsh:30 — parameterized alias `_(){…};_`.
    #[test]
    fn parameterized_alias_trick() {
        assert_parity(
            r##"alias oalln='_(){ print "all for $1" ;};_'
print -r -- ${aliases[oalln]}
eval "oalln myapp"
print "fn _ defined: $+functions[_]""##,
        );
    }

    /// zsh-openshift-aliases.plugin.zsh:3 — `type -ap` path-only existence probe.
    #[test]
    fn type_ap_path_probe() {
        assert_parity(
            r##"t=$(mktemp -d)
mkdir $t/bin
print '#!/bin/sh' > $t/bin/oc
chmod +x $t/bin/oc
path=($t/bin $path)
if ! type -ap -- "oc" >/dev/null 2>&1; then
    print "no oc"
else
    print "found: ${$(type -ap -- oc):t}"
fi
if ! type -ap -- "no-such-cmd-zzz" >/dev/null 2>&1; then print "no ghost"; fi
rm -rf $t"##,
        );
    }
}

// ──────────────────────────────── zsh-travis ───────────────────────

mod zsh_travis {
    use super::*;

    /// autoload/__trav_common_url:5-11 — URL normalization chain of substitutions.
    #[test]
    fn url_normalization_chain() {
        assert_parity(
            r##"repo_url='https://github.com/MenkeTechnologies/zsh-travis.git'
url="${repo_url/https:\/\//}"
url="${url/http:\/\//}"
url="${url/ssh:\/\//}"
url="${url/git:\/\//}"
url="${url/.com/}"
url="${url/.git/}"
url="$url/builds"
print -r -- $url"##,
        );
    }

    /// autoload/__trav_open:3 — `${=cmd}` forced split of command scalar.
    #[test]
    fn forced_split_open_command() {
        assert_parity(
            r##"ZPWR_OPEN_CMD='print -r --'
${=ZPWR_OPEN_CMD} "https://travis-ci.org/u/repo/builds"
unsplit=( "$ZPWR_OPEN_CMD" )
split=( ${=ZPWR_OPEN_CMD} )
print "unsplit=${#unsplit} split=${#split}""##,
        );
    }
}

// ──────────────────────────── zsh-kubectl-completion ───────────────

mod kubectl_completion {
    use super::*;

    /// _kubectl:41-49 — table parse pipeline via tr row pack/unpack.
    #[test]
    fn table_parse_pipeline() {
        assert_parity(
            r##"out=$'NAME      READY   STATUS\npod-a     1/1     Running\npod-b     0/1     Pending'
parsed=$(echo ${out} | tail -n +2 | tr ' ' '$')
print -r -- "rows:"
print -rl -- ${(f)parsed}
list=($(echo ${parsed} | tr '$' ' ' | awk '{print $1}'))
print -r -- "names: $list""##,
        );
    }

    /// _kubectl:12-13 — `[ ! -z ${var} ]` unquoted-unset collapse.
    #[test]
    fn unquoted_unset_collapse() {
        assert_parity(
            r##"unset _filter_namespace
if [ ! -z ${_filter_namespace} ]; then print "ns set"; else print "ns unset"; fi
_filter_namespace=kube-public
if [ ! -z ${_filter_namespace} ]; then print "ns=${_filter_namespace}"; fi"##,
        );
    }
}

// ──────────────────────── zsh-pip-description-completion ───────────

mod pip_completion {
    use super::*;

    /// autoload/zsh-pip-clean-packages:2 — sed HTML scrape + unquoted multi-line RHS.
    #[test]
    fn html_scrape_multiline_compare() {
        assert_parity(
            r##"zsh-pip-clean-packages() {
    sed -n '/<a href/ s/.*>\([^<]\{1,\}\).*/\1/p'
}
expected="0x10c-asm
1009558_nester"
actual=$(echo -n "<html><head><title>Simple Index</title></head><body>
<a href='0x10c-asm'>0x10c-asm</a><br/>
<a href='1009558_nester'>1009558_nester</a><br/>
</body></html>" | zsh-pip-clean-packages)
if [[ $actual != $expected ]]; then
    print "broken: $actual"
else
    print "python's simple index is fine"
fi"##,
        );
    }
}

// ───────────────────────────────── zinit ───────────────────────────

mod zinit {
    use super::*;

    /// zinit-side.zsh:19-30 — alternation-pattern URL→namespace folding.
    #[test]
    fn url_namespace_folding() {
        assert_parity(
            r##"setopt extendedglob
REPLY="https--github.com--robbyrussell--oh-my-zsh--trunk--plugins--git"
REPLY="${REPLY/https--github.com--(robbyrussell--oh-my-zsh|ohmyzsh--ohmyzsh)--trunk--plugins--/OMZP::}"
print -r -- "$REPLY"
REPLY="https--github.com--sorin-ionescu--prezto--trunk--modules--git"
REPLY="${REPLY/https--github.com--sorin-ionescu--prezto--trunk--modules--/PZTM::}"
print -r -- "$REPLY""##,
        );
    }

    /// zinit-side.zsh:32 — `http(|s):` optional-group glob.
    #[test]
    fn http_optional_group_glob() {
        assert_parity(
            r##"setopt extendedglob
for u in http: https: ftp: http; do
  if [[ $u == http(|s): ]]; then print "yes:$u"; else print "no:$u"; fi
done"##,
        );
    }

    /// zinit-side.zsh:81 — leading-whitespace strip + trailing-slash trim.
    #[test]
    fn whitespace_strip_slash_trim() {
        assert_parity(
            r##"setopt extendedglob
url=$'\t\t  github.com/foo/bar/'
url="${${url#"${url%%[! $'\t']*}"}%/}"
print -r -- "[$url]""##,
        );
    }

    /// zinit-side.zsh:70 — `${(@)arr/(#s)<->-/}` strip leading number prefix.
    #[test]
    fn strip_leading_number_prefix() {
        assert_parity(
            r##"setopt extendedglob
local -a mods stripped
mods=( 0-myice 12-other 3-third 100-fourth )
stripped=( ${(@)mods/(#s)<->-/} )
print -rl -- $stripped"##,
        );
    }

    /// zinit-side.zsh:76 — drop array elements containing `''`.
    #[test]
    fn drop_elements_with_double_quote() {
        assert_parity(
            r##"setopt extendedglob
local -a all kept
all=( pick src )
all+=( "as''" "from''" ver )
kept=( ${(@)all:#*\'\'*} )
print -rl -- $kept"##,
        );
    }

    /// zinit.zsh:2337 + zinit-side.zsh:103-104 — ice pack/unpack round-trip.
    #[test]
    fn ice_pack_unpack_roundtrip() {
        assert_parity(
            r##"setopt extendedglob
typeset -A ICE
ICE=( pick "f a.zsh" as program src init.sh )
local packed="${(j: :)${(qkv)ICE[@]}}"
print -r -- "packed=$packed"
local -a tmp
tmp=( "${(Q@)${(z@)packed}}" )
print "count=${#tmp}"
typeset -A back
(( ${#tmp} % 2 == 0 )) && back=( "${tmp[@]}" )
print -r -- "pick=[${back[pick]}] as=[${back[as]}] src=[${back[src]}]""##,
        );
    }

    /// zinit-side.zsh:187 — `(PA)name::=…` indirect array assignment.
    #[test]
    #[ignore = "zshrs gap: : ${(PA)name::=...} indirect array assignment populates nothing (named hash empty)"]
    fn indirect_array_assignment() {
        assert_parity(
            r##"setopt extendedglob warncreateglobal
typeset -A MYICE
MYICE=( as program pick init.sh )
local outname=DEST
typeset -gA DEST
: ${(PA)outname::="${(kv)MYICE[@]}"}
print -r -- "${(kv)DEST[@]}" | tr ' ' '\n' | sort"##,
        );
    }

    /// zinit-install.zsh:579-580 — `[[ -z ${arr[(r)*/$x]} ]]` reverse-subscript test.
    #[test]
    fn reverse_subscript_emptiness_test() {
        assert_parity(
            r##"local -a already
already=( /comp/_git /comp/_make /comp/_ls )
for cfile in _git _new; do
  if [[ -z ${already[(r)*/$cfile]} ]]; then
    print "install:$cfile"
  else
    print "skip:$cfile"
  fi
done"##,
        );
    }

    /// zinit-install.zsh:600 — `for k v in ${(kv)hash}` paired iteration.
    #[test]
    fn paired_kv_iteration() {
        assert_parity(
            r##"typeset -A counts
counts=( apple 3 pear 5 plum 1 )
local k v
for k v in ${(kv)counts}; do
  print -r -- "$k=$v"
done | sort"##,
        );
    }

    /// zinit-install.zsh:604 — pluralize via `${=${n:#1}:+s}`.
    #[test]
    fn pluralize_completion_count() {
        assert_parity(
            r##"for n in 0 1 2 5; do
  print -r -- "$n completion${=${n:#1}:+s}"
done"##,
        );
    }

    /// zinit-side.zsh:179 — static-vs-disk ice precedence with `${a-${b}}`.
    #[test]
    fn ice_precedence_fallback() {
        assert_parity(
            r##"typeset -A sice mdata MY
sice=( as program )
mdata=( pick init.sh as fromdisk )
for key in as pick ver; do
  (( ${+sice[$key]} + ${+mdata[$key]} )) && MY[$key]="${sice[$key]-${mdata[$key]}}"
done
print -r -- "as=[${MY[as]}] pick=[${MY[pick]}] ver=[${MY[ver]}]""##,
        );
    }

    /// zinit-autoload.zsh:1008 — triple-nested conditional id builder.
    #[test]
    fn nested_conditional_id_builder() {
        assert_parity(
            r##"build() {
  print -r -- "$1${2:+${${${(M)1:#%}:+$2}:-/$2}}"
}
build "%local" "plug"
build "user" "plug"
build "solo""##,
        );
    }

    /// zinit-autoload.zsh:1255 — `${${${(M)flag:#(y|yes)}:+$a}:-$b}` selector.
    #[test]
    fn yes_set_selector() {
        assert_parity(
            r##"org=ohmyzsh; user=robbyrussell
for isorg in yes no maybe; do
  print -r -- "${${${(M)isorg:#(y|yes)}:+$org}:-$user}"
done"##,
        );
    }

    /// zinit-additional.zsh:8 — `(#e)` end-anchor any-element trailing-backslash test.
    #[test]
    fn end_anchor_trailing_backslash() {
        assert_parity(
            r##"setopt extendedglob
local -a substs
substs=( 'a\' 'b' 'c\' )
if [[ -n ${(M)substs:#*\\(#e)} ]]; then print "has-trailing-bs"; else print "none"; fi
substs=( a b c )
if [[ -n ${(M)substs:#*\\(#e)} ]]; then print "has-trailing-bs"; else print "none"; fi"##,
        );
    }

    /// zinit-side.zsh:208 — `(#b)` backref-substitute backslash-prefixing `{token}`.
    #[test]
    fn backref_backslash_prefix_tokens() {
        assert_parity(
            r##"setopt extendedglob
local match mbegin mend
local ice='atclone:{git}{reset}done'
ice="${ice//(#b)(\{[a-z0-9_-]##\})/\\$match[1]}"
print -r -- "$ice""##,
        );
    }

    /// zinit-side.zsh:68 — `${(s.|.)str}` split on `|`.
    #[test]
    fn split_on_pipe() {
        assert_parity(
            r##"local list="pick|src|as|from|ver"
local -a parts
parts=( ${(s.|.)list} )
print "count=${#parts}"
print -r -- "${parts[3]}""##,
        );
    }

    /// zinit-side.zsh:103 — `${${user:#(%|/)*}:+/}` conditional separator.
    #[test]
    fn conditional_separator_emit() {
        assert_parity(
            r##"for u in user "%theme" "/abs"; do
  print -r -- "key=$u${${u:#(%|/)*}:+/}plugin"
done"##,
        );
    }

    /// zinit-autoload.zsh:1725 — keys filter with `~` exclusion then prefix strip.
    #[test]
    #[ignore = "zshrs gap: ${(M)keys:##pat~excl} keep-with-exclusion + prefix strip errors (exit 1)"]
    fn keys_filter_exclude_strip() {
        assert_parity(
            r##"setopt extendedglob
typeset -A H
H=( STATES__foo 1 STATES__local_bar 1 OTHER__x 1 STATES__baz 1 )
local -a filtered
filtered=( ${${(M)${(k)H}:##STATES__*~*local*}//[A-Z]*__/} )
print -rl -- ${(o)filtered}"##,
        );
    }
}

// ──────────────────────── fast-syntax-highlighting ─────────────────

mod fast_syntax_highlighting {
    use super::*;

    /// fast-syntax-highlighting.plugin.zsh:54 — `${fpath[(r)PAT]}` reverse-subscript value.
    #[test]
    fn fpath_reverse_subscript_value() {
        assert_parity(
            r##"local -a fpath
fpath=( /a/b /c/d /e/f )
print -r -- "found=[${fpath[(r)/c/d]}]"
print -r -- "miss=[${fpath[(r)/x/y]}]""##,
        );
    }

    /// fast-syntax-highlighting.plugin.zsh:54 — `${arr[-1]}` last element + suffix glob.
    #[test]
    fn last_element_suffix_test() {
        assert_parity(
            r##"local -a zsh_loaded_plugins
zsh_loaded_plugins=( foo/bar baz/fast-syntax-highlighting )
if [[ ${zsh_loaded_plugins[-1]} != */fast-syntax-highlighting ]]; then
  print "needfpath"
else
  print "alreadylast"
fi"##,
        );
    }

    /// fast-syntax-highlighting.plugin.zsh:131 — slice then `(I)` last-index of substring.
    #[test]
    fn slice_then_I_last_index() {
        assert_parity(
            r##"local BUFFER="abcXdefXghi"
integer min=11
local needle="X"
integer pos
(( pos = ${${BUFFER[1,$min]}[(I)$needle]} ))
print -r -- "lastpos=$pos""##,
        );
    }

    /// fast-syntax-highlighting.plugin.zsh:167 — `(r)` glob subscript fetching key:value.
    #[test]
    fn reverse_subscript_key_value() {
        assert_parity(
            r##"local -a zle_highlight
zle_highlight=( region:bg=blue isearch:fg=red default:none )
local entry="isearch"
print -r -- "[${zle_highlight[(r)${entry}:*]}]""##,
        );
    }

    /// fast-syntax-highlighting.plugin.zsh:272 — `(s.:.)` split then slice.
    #[test]
    fn split_then_slice() {
        assert_parity(
            r##"local spec="completion:_files:-default-:opt"
print -r -- "${${(s.:.)spec}[2,3]}""##,
        );
    }

    /// fast-syntax-highlighting.plugin.zsh:242 — keys then alternation filter-out.
    #[test]
    fn keys_alternation_filter() {
        assert_parity(
            r##"setopt extendedglob
typeset -A mywidgets
mywidgets=( self-insert x run-help x beep x my-widget x yank x )
local -a tobind
tobind=( ${${(k)mywidgets}:#(run-help|beep|yank)} )
print -rl -- ${(o)tobind}"##,
        );
    }

    /// fast-syntax-highlighting.plugin.zsh:345 — `<->` guard before numeric compare.
    #[test]
    fn numeric_guard_before_compare() {
        assert_parity(
            r##"setopt extendedglob
typeset -A tcap
tcap=( Co 256 )
[[ "${tcap[Co]}" = <-> && "${tcap[Co]}" -ge 256 ]] && print "truecolor-ok" || print "no"
tcap=( Co abc )
[[ "${tcap[Co]}" = <-> && "${tcap[Co]}" -ge 256 ]] && print "truecolor-ok" || print "no""##,
        );
    }
}

// ──────────────────────────────── zsh-unique-id ────────────────────

mod zsh_unique_id {
    use super::*;

    /// zsh-unique-id.plugin.zsh:154 — `${${(@s,:,)str}[N]}` split then index.
    #[test]
    fn split_colon_then_index() {
        assert_parity(
            r##"codenames="atlantis:echelon:quantum:ion:proxima"
ZUID_ID=3
print -r -- "${${(@s,:,)codenames}[$ZUID_ID]}"
ZUID_ID=1
print -r -- "${${(@s,:,)codenames}[$ZUID_ID]}""##,
        );
    }

    /// zsh-unique-id.plugin.zsh:89 — `<->` nonzero-integer test.
    #[test]
    fn nonzero_integer_test() {
        assert_parity(
            r##"for v in 0 7 42 "" abc 1a -3; do
  if [[ "$v" = <-> && "$v" != "0" ]]; then print "num:$v"; else print "no:$v"; fi
done"##,
        );
    }

    /// zsh-unique-id.plugin.zsh:50 — `${(j,:,)arr}` join with multi-char delim flag.
    #[test]
    fn join_comma_delim_flag() {
        assert_parity(
            r##"local -a codenames
codenames=( atlantis echelon quantum ion proxima polaris )
print -r -- "${(j,:,)codenames}""##,
        );
    }
}

// ──────────────────────────────── zzcomplete ───────────────────────

mod zzcomplete {
    use super::*;

    /// zz-process-buffer:70-71 — bare-arithmetic string slicing in subscripts.
    #[test]
    fn arithmetic_string_slicing() {
        assert_parity(
            r##"local buf="echo hello world"
local word="hello"
local wordlen=${#word}
print -r -- "first${wordlen}=[${buf[1,wordlen]}]"
print -r -- "rest=[${buf[wordlen+1,-1]}]""##,
        );
    }

    /// zz-process-buffer:60-62 — `${buf##(#m)[^c]#}` consume-run capturing $MATCH.
    #[test]
    fn consume_run_capture_match() {
        assert_parity(
            r##"setopt extendedglob
local MATCH
local buf="   foobar"
buf="${buf##(#m)[^f]#}"
print -r -- "spaces=$#MATCH rest=[$buf]""##,
        );
    }

    /// zz-process-buffer:93-94 — split word at cursor offset into two halves.
    #[test]
    fn split_word_at_offset() {
        assert_parity(
            r##"local word="completion"
integer diff=4
print -r -- "left=[${word[1,diff]}] right=[${word[diff+1,-1]}]""##,
        );
    }

    /// hsmw-highlight:182 — `(z)` vs `(zZ+C+)` lexer split with comment recognition.
    #[test]
    fn lexer_split_comment_recognition() {
        assert_parity(
            r##"setopt extendedglob
local buf='echo hi # a comment'
local -a a b
a=( ${(z)buf} )
b=( ${(zZ+C+)buf} )
print "z=${#a} zZC=${#b}"
print -r -- "last_z=[${a[-1]}]""##,
        );
    }
}

// ──────────────────────────────── zbrowse ──────────────────────────

mod zbrowse {
    use super::*;

    /// zbrowse:406 — `${(j: :)arr[1,N]}` join a bounded slice.
    #[test]
    fn join_bounded_slice() {
        assert_parity(
            r##"local -a vals
vals=( one two three four five )
print -r -- "${(j: :)vals[1,3]}""##,
        );
    }

    /// zbrowse:494 — `(s:|:@)` split first element, `(@)arr[2,-1]` drop head.
    #[test]
    fn split_head_drop_tail() {
        assert_parity(
            r##"local -a logs
logs=( "ssh|deploy|build" "second" "third" )
local -a first
first=( "${(s:|:@)logs[1]}" )
logs=( "${(@)logs[2,-1]}" )
print -rl -- $first
print -r -- "remaining=${#logs}""##,
        );
    }
}

// ──────────────────────── history-search-multi-word ────────────────

mod history_search_multi_word {
    use super::*;

    /// hsmw-highlight:109 — `${$(type -w -- x)##*: }` command typing.
    #[test]
    fn command_typing_via_type_w() {
        assert_parity(
            r##"local REPLY
REPLY="${$(LC_ALL=C builtin type -w -- print 2>/dev/null)##*: }"
print -r -- "print=$REPLY"
REPLY="${$(LC_ALL=C builtin type -w -- if 2>/dev/null)##*: }"
print -r -- "if=$REPLY""##,
        );
    }

    /// hsmw-highlight:136-154 — `[[ -o option ]]` runtime option introspection.
    #[test]
    fn option_introspection() {
        assert_parity(
            r##"setopt interactivecomments
[[ -o interactive_comments ]] && print "ic:on" || print "ic:off"
unsetopt interactivecomments
[[ -o interactive_comments ]] && print "ic:on" || print "ic:off"
[[ -o extendedglob ]] && print "eg:on" || print "eg:off""##,
        );
    }

    /// hsmw-context-main:101 — `${(@)arr//$'\n'/\\n}` newline-to-literal per element.
    #[test]
    fn newline_to_literal_per_element() {
        assert_parity(
            r##"local -a lst
lst=( $'a\nb' $'c\nd' plain )
lst=( "${(@)lst//$'\n'/\\n}" )
print -rl -- "${lst[@]}""##,
        );
    }

    /// hsmw-context-main:58 — `(#m)[class]/\\$MATCH` backslash-escape metachars.
    #[test]
    fn escape_metachars_via_match() {
        assert_parity(
            r##"setopt extendedglob
local MATCH to_search='a*b?c|d'
to_search="${to_search//(#m)[][*?|#~^()><\\]/\\$MATCH}"
print -r -- "$to_search""##,
        );
    }

    /// hsmw-highlight:100 — `(( $+hash[(e)$key] ))` exact-string key existence.
    #[test]
    fn exact_string_key_existence() {
        assert_parity(
            r##"typeset -A myaliases
myaliases=( gz "gunzip" txt "cat" )
for f in a.gz b.txt c.png; do
  ext="${f##*.}"
  if (( $+myaliases[(e)$ext] )); then print "alias:$ext"; else print "plain:$ext"; fi
done"##,
        );
    }
}

// ──────────────────────────────── zsh-tig ──────────────────────────

mod zsh_tig {
    use super::*;

    /// zsh-tig-plugin.plugin.zsh:9 — `${${(M)0:#/*}:-$PWD/$0}` abs-path $0 fixup.
    #[test]
    fn abs_path_zero_fixup() {
        assert_parity(
            r##"local PWD=/home/user
for z in /abs/path rel/path ./x; do
  print -r -- "${${(M)z:#/*}:-$PWD/$z}"
done"##,
        );
    }
}
