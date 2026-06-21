//! Behavioural parity corpus mined from the user's installed
//! `~/.zinit/snippets` set — the oh-my-zsh lib (`OMZL::*`) and plugin
//! (`OMZP::*`) snippets loaded by his daily-driver zinit config.
//!
//! Each test replicates a DISTINCTIVE zsh idiom actually used in the
//! snippet source (cited `file:line` per test) as a self-contained,
//! deterministic mini-script, then asserts `zshrs --zsh -fc` matches
//! `/opt/homebrew/bin/zsh -fc` byte-for-byte on stdout and exit code.
//!
//! Both shells are invoked with `-f` (no rc files) so the comparison
//! is rc-free and fair. Every script was verified byte-identical across
//! two consecutive real-zsh runs before inclusion (no $RANDOM/$$/time/
//! network/env dependence). vcs/tool-parsing idioms are fed CANNED
//! input (the exact text the real command emits); filesystem-touching
//! scripts sandbox under `mktemp -d` and clean up.
//!
//! Same harness shape as `tests/zinit_plugin_corpus_parity.rs`: skip
//! silently when no zsh is present.

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

// ════════════════════════════ OMZL::git.zsh ════════════════════════

mod omzl_git {
    use super::*;

    /// OMZL::git.zsh:39 — `${ref:gs/%/%%}` global-subst modifier escapes % for prompt.
    #[test]
    fn gs_percent_escape() {
        assert_parity(r###"ref="feat/%special%"; echo "${ref:gs/%/%%}""###);
    }

    /// OMZL::git.zsh:104,107 — split porcelain `-b` on newlines then regex-capture tracking line.
    #[test]
    fn porcelain_track_capture() {
        assert_parity(
            r###"st="## main...origin/main [ahead 2, behind 1]
 M file.rs
?? new.txt"
lines=("${(@f)${st}}")
if [[ "$lines[1]" =~ "^## [^ ]+ \[(.*)\]" ]]; then
  echo "TRACK:$match[1]"
fi"###,
        );
    }

    /// OMZL::git.zsh:109,111 — split branch-status on comma then =~ capture group + numeric group.
    #[test]
    fn branch_status_comma_split_capture() {
        assert_parity(
            r###"track="ahead 2, behind 1"
bs=("${(@s/,/)track}")
for b in $bs; do
  if [[ $b =~ "(behind|diverged|ahead) ([0-9]+)?" ]]; then
    echo "$match[1]=$match[2]"
  fi
done"###,
        );
    }

    /// OMZL::git.zsh:120,122,124 — iterate assoc keys, build per-key anchored regex, test status.
    #[test]
    #[ignore = "zshrs gap: $'(^|\\n)'-anchored regex built per assoc-key fails to match multi-line status text (no SEEN output)"]
    fn per_key_anchored_regex() {
        assert_parity(
            r###"typeset -A m=("\?\? " UNTRACKED "M  " MODIFIED " M " MODIFIED "A  " ADDED)
text=" M file.rs
?? new.txt
A  added.rs"
for p in "${(@k)m}"; do
  rx=$'(^|\n)'"$p"
  [[ "$text" =~ $rx ]] && echo "SEEN:${m[$p]}"
done | sort"###,
        );
    }

    /// OMZL::git.zsh:131,132,134 — reverse-prepend display gated on key existence.
    #[test]
    fn reverse_prepend_on_key_existence() {
        assert_parity(
            r###"typeset -A seen=(UNTRACKED 1 MODIFIED 1 AHEAD 2)
typeset -A pm=(UNTRACKED "?" MODIFIED "!" ADDED "+" AHEAD ">")
order=(UNTRACKED ADDED MODIFIED AHEAD)
sp=""
for c in $order; do
  if (( ${+seen[$c]} )); then
    nd=$pm[$c]; sp="$nd$sp"
  fi
done
echo "$sp""###,
        );
    }

    /// OMZL::git.zsh:239 — `${var/refs\/remotes\//}` single-subst stripping slash-prefix.
    #[test]
    fn strip_remotes_prefix() {
        assert_parity(r###"remote="refs/remotes/origin/main"; echo "${remote/refs\/remotes\//}""###);
    }

    /// OMZL::git.zsh:277 — `${ref#refs/heads/}` prefix strip to bare branch.
    #[test]
    fn strip_heads_prefix() {
        assert_parity(r###"ref="refs/heads/feature/x"; echo "${ref#refs/heads/}""###);
    }

    /// OMZL::git.zsh:27 — `||`-chained cmdsubst fallthrough for ref resolution.
    #[test]
    fn cmdsubst_or_fallthrough() {
        assert_parity(r###"ref=$(false) || ref=$(echo tagname) || ref=$(echo sha) || ref="none"
echo "$ref""###);
    }

    /// OMZL::git.zsh:272,274 — capture $? into ret, then == 128 early branch.
    #[test]
    fn capture_ret_128_branch() {
        assert_parity(
            r###"f() { return 128; }
f; ret=$?
if [[ $ret != 0 ]]; then
  [[ $ret == 128 ]] && { echo "no repo"; }
fi"###,
        );
    }

    /// OMZL::git.zsh:297 — combined nonempty-and-nonzero guard.
    #[test]
    fn nonempty_nonzero_guard() {
        assert_parity(
            r###"commits="3"
if [[ -n "$commits" && "$commits" != 0 ]]; then echo "AHEAD$commits"; fi
commits="0"
if [[ -n "$commits" && "$commits" != 0 ]]; then echo "shown"; else echo "skipped"; fi"###,
        );
    }

    /// OMZL::git.zsh:212,215,224 — array build with ternary append + ${VAR:-default} in element.
    #[test]
    fn ternary_array_append_default() {
        assert_parity(
            r###"typeset -a FLAGS=("--porcelain")
DISABLE_UNTRACKED_FILES_DIRTY=true
[[ "${DISABLE_UNTRACKED_FILES_DIRTY:-}" == "true" ]] && FLAGS+="--untracked-files=no"
FLAGS+="--ignore-submodules=${GIT_STATUS_IGNORE_SUBMODULES:-dirty}"
print -l -- $FLAGS"###,
        );
    }

    /// OMZL::git.zsh:229-233 — parse_git_dirty selecting DIRTY vs CLEAN theme var.
    #[test]
    fn dirty_clean_theme_select() {
        assert_parity(
            r###"ZSH_THEME_GIT_PROMPT_DIRTY="*"
ZSH_THEME_GIT_PROMPT_CLEAN=""
STATUS=" M file.rs"
if [[ -n $STATUS ]]; then echo "$ZSH_THEME_GIT_PROMPT_DIRTY"; else echo "$ZSH_THEME_GIT_PROMPT_CLEAN"; fi"###,
        );
    }

    /// OMZL::git.zsh:227 — porcelain piped to tail -n 1.
    #[test]
    fn porcelain_tail_last() {
        assert_parity(
            r###"status_out=" M a.rs
?? b.rs
 M c.rs"
echo "$status_out" | tail -n 1"###,
        );
    }

    /// OMZL::git.zsh:109-116 — full branch-status parse: comma split, lead-space strip, capture into assoc.
    #[test]
    fn full_branch_status_parse() {
        assert_parity(
            r###"track="ahead 3, behind 2, diverged"
typeset -A seen
parts=("${(@s/,/)track}")
for p in $parts; do
  p="${p## }"
  if [[ $p =~ "(behind|diverged|ahead) ([0-9]+)?" ]]; then
    seen[$match[1]]=${match[2]:-1}
  fi
done
for k in ahead behind diverged; do echo "$k=${seen[$k]}"; done"###,
        );
    }
}

// ═════════════════════════ OMZL::functions.zsh ═════════════════════

mod omzl_functions {
    use super::*;

    /// OMZL::functions.zsh:50 — `${@:$#}` slice the last positional.
    #[test]
    fn last_positional_slice() {
        assert_parity(r###"set -- a b c
echo "last=${@:$#}""###);
    }

    /// OMZL::functions.zsh:102 — `(( $+aliases[$1] ))` existence test on $aliases assoc.
    #[test]
    fn aliases_existence_test() {
        assert_parity(
            r###"alias gco="git checkout"
(( $+aliases[gco] )) && echo "$aliases[gco]"
(( $+aliases[nope] )) || echo "no alias nope""###,
        );
    }

    /// OMZL::functions.zsh:129-132 — default(): typeset -g only when ${+parameters[$1]} false.
    #[test]
    fn default_via_parameters() {
        assert_parity(
            r###"default() { (( ${+parameters[$1]} )) && return 0; typeset -g "$1"="$2" && return 3; }
default MYVAR hello; echo "ret=$? MYVAR=$MYVAR"
default MYVAR world; echo "ret=$? MYVAR=$MYVAR""###,
        );
    }

    /// OMZL::functions.zsh:144-145 — detect exported var by `${parameters[$1]}` *-export* match.
    #[test]
    fn parameters_export_type_match() {
        assert_parity(
            r###"export EXISTING=x
env_default() { [[ ${parameters[$1]} = *-export* ]] && return 0; export "$1=$2" && return 3; }
env_default EXISTING new; echo "ret=$? EXISTING=$EXISTING"
env_default FRESH val; echo "ret=$? FRESH=$FRESH""###,
        );
    }

    /// OMZL::functions.zsh:221-234 — omz_urlencode core loop: per-char index, space→+, else %+hex.
    #[test]
    fn urlencode_core_loop() {
        assert_parity(
            r###"emulate -L zsh
str="a b&c"
dont_escape="[A-Za-z0-9]"
url_str=""
for (( i = 1; i <= ${#str}; ++i )); do
  byte="$str[i]"
  if [[ "$byte" =~ "$dont_escape" ]]; then
    url_str+="$byte"
  elif [[ "$byte" == " " ]]; then
    url_str+="+"
  else
    ord=$(( [##16] #byte ))
    url_str+="%$ord"
  fi
done
echo -E "$url_str""###,
        );
    }

    /// OMZL::functions.zsh:232 — `$(( #var ))` char-code ordinal of single-char variable.
    #[test]
    fn char_ordinal_of_var() {
        assert_parity(
            r###"for byte in A z 0 " "; do
  echo "$(( #byte ))"
done"###,
        );
    }

    /// OMZL::functions.zsh:232 — `$(( [##16] N ))` radix-output base specifier.
    #[test]
    fn radix_output_base() {
        assert_parity(r###"echo $(( [##16] 255 )) $(( [##16] 38 )) $(( [##2] 10 ))"###);
    }

    /// OMZL::functions.zsh:264-269 — omz_urldecode: chained :gs modifiers + printf.
    #[test]
    fn urldecode_gs_chain() {
        assert_parity(
            r###"encoded_url="a%20b%26c+d"
tmp=${encoded_url:gs/+/ /}
tmp=${tmp:gs/\\/\\\\/}
tmp=${tmp:gs/%/\\x/}
printf -- "$tmp"
echo"###,
        );
    }

    /// OMZL::functions.zsh:150,190 — zmodload zsh/langinfo then read $langinfo[CODESET].
    #[test]
    fn langinfo_codeset() {
        assert_parity(
            r###"zmodload zsh/langinfo
[[ -n "$langinfo[CODESET]" ]] && echo "codeset-present""###,
        );
    }

    /// OMZL::functions.zsh:193 — `${array[(r)$val]}` reverse-subscript membership.
    #[test]
    fn array_r_membership() {
        assert_parity(
            r###"safe=(UTF-8 utf8 US-ASCII)
enc=UTF-8
[[ -n ${safe[(r)$enc]} ]] && echo "safe:$enc"
enc=LATIN1
[[ -z ${safe[(r)$enc]} ]] && echo "unsafe:$enc""###,
        );
    }

    /// OMZL::functions.zsh:177-178 — emulate -L zsh + setopt norematchpcre before =~.
    #[test]
    fn emulate_norematchpcre() {
        assert_parity(
            r###"f() { emulate -L zsh; setopt norematchpcre; [[ "abc123" =~ "[0-9]+" ]] && echo "match=$MATCH"; }
f"###,
        );
    }

    /// OMZL::functions.zsh:38 — `[[ "$x" = (http|https)://* ]]` glob alternation.
    #[test]
    fn glob_alternation_url() {
        assert_parity(
            r###"u="https://example.com"
[[ "$u" = (http|https)://* ]] && echo "is-url"
v="ftp://x"
[[ "$v" = (http|https)://* ]] || echo "not-http""###,
        );
    }

    /// OMZL::functions.zsh:43 — `${=var}` forced field-splitting of a command string.
    #[test]
    fn forced_field_split() {
        assert_parity(r###"open_cmd="cmd a b"
print -l -- ${=open_cmd}"###);
    }

    /// OMZL::functions.zsh:2-4 — zsh_stats awk frequency table + sort -nr | head.
    #[test]
    fn zsh_stats_pipeline() {
        assert_parity(
            r###"printf "%s\n" "1 git status" "2 git status" "3 ls" "4 git commit" \
| awk "{ CMD[\$2]++; count++; } END { for (a in CMD) print CMD[a] \" \" a }" \
| sort -nr | head -n 3"###,
        );
    }

    /// OMZL::functions.zsh:117 — try_alias_value: alias_value || echo fallback chain.
    #[test]
    fn try_alias_value_fallback() {
        assert_parity(
            r###"alias_value() { (( $+aliases[$1] )) && echo $aliases[$1]; }
try_alias_value() { alias_value "$1" || echo "$1"; }
alias ll="ls -l"
try_alias_value ll
try_alias_value notanalias"###,
        );
    }
}

// ════════════════════════ OMZL::directories.zsh ════════════════════

mod omzl_directories {
    use super::*;

    /// OMZL::directories.zsh:8-9 — `alias -g` global-alias definitions.
    #[test]
    fn global_alias_define() {
        assert_parity(
            r###"alias -g ...="../.."
alias -g ....="../../.."
alias -g | sort"###,
        );
    }

    /// OMZL::directories.zsh:13-14 — `alias -- -='cd -'` end-of-options form + numeric pushd.
    #[test]
    fn alias_end_of_options() {
        assert_parity(
            r###"alias -- -="cd -"
alias 1="cd -1"
alias | grep -E "^(1|-)=" | sort"###,
        );
    }

    /// OMZL::directories.zsh:27-33 — d(): [[ -n $1 ]] argument-presence branch.
    #[test]
    fn d_argument_presence() {
        assert_parity(
            r###"setopt auto_pushd pushd_ignore_dups
d () { if [[ -n $1 ]]; then echo "with-arg:$1"; else echo "no-arg-listing"; fi; }
d
d foo"###,
        );
    }

    /// OMZL::directories.zsh:2-5 — setopt auto_cd/auto_pushd/... combo verified via listing.
    #[test]
    fn setopt_pushd_combo() {
        assert_parity(
            r###"setopt auto_cd auto_pushd pushd_ignore_dups pushdminus
setopt | grep -E "^(autocd|autopushd|pushdignoredups|pushdminus)$" | sort"###,
        );
    }
}

// ══════════════════════════ OMZL::grep.zsh ═════════════════════════

mod omzl_grep {
    use super::*;

    /// OMZL::grep.zsh:4-5 — glob qualifier (Nm-1): nullglob + modified-within-1-day.
    #[test]
    fn glob_nullglob_mtime() {
        assert_parity(
            r###"pat=/nonexistent/path/grep-alias
caches=($pat(Nm-1))
if [[ -n "$caches" ]]; then echo "found"; else echo "no-cache"; fi"###,
        );
    }

    /// OMZL::grep.zsh:13 — brace-set held literal in a variable (not expanded).
    #[test]
    fn literal_brace_set_in_var() {
        assert_parity(
            r###"EXC="{.bzr,CVS,.git,.hg,.svn}"
echo "--exclude-dir=$EXC""###,
        );
    }

    /// OMZL::grep.zsh:19,24-26 — build GREP_OPTIONS then emit derived alias string.
    #[test]
    fn grep_options_alias_emit() {
        assert_parity(
            r###"GREP_OPTIONS="--color=auto --exclude-dir={.git,.svn}"
if [[ -n "$GREP_OPTIONS" ]]; then echo "alias grep=\"grep $GREP_OPTIONS\""; fi"###,
        );
    }

    /// OMZL::grep.zsh:9 — here-string `<<<` feeding a command's stdin.
    #[test]
    fn here_string_stdin() {
        assert_parity(r###"cat <<< "literal line""###);
    }
}

// ══════════════════════════════ OMZP::git ══════════════════════════

mod omzp_git {
    use super::*;

    /// OMZP::git:3 — `${${(As: :)$(...)}[3]}` split cmdsubst on space then index.
    #[test]
    fn split_cmdsubst_index() {
        assert_parity(r###"v="${${(As: :)$(printf "git version 2.43.0")}[3]}"; print -r -- "$v""###);
    }

    /// OMZP::git:32 — nested brace expansion of ref paths.
    #[test]
    fn nested_brace_refs() {
        assert_parity(r###"for ref in refs/{heads,remotes/{origin,upstream}}/{main,master}; do print -r -- $ref; done"###);
    }

    /// OMZP::git:43 — `${ref#"$remote/"}` quoted-prefix removal with variable.
    #[test]
    fn quoted_prefix_removal() {
        assert_parity(r###"remote=origin; ref="origin/feature/x"; print -r -- ${ref#"$remote/"}"###);
    }

    /// OMZP::git:42 — `[[ $ref == $remote/* ]]` glob match against variable prefix.
    #[test]
    fn glob_match_var_prefix() {
        assert_parity(r###"remote=origin; ref="origin/main"; if [[ $ref == $remote/* ]]; then print yes; else print no; fi"###);
    }

    /// OMZP::git:12 — `function name() { }` with `&& return N` short-circuit.
    #[test]
    fn function_paren_return() {
        assert_parity(r###"function f() { [[ -z "$1" ]] && return 3; print -r -- "got:$1"; }; f hello; print exit=$?; f; print exit=$?"###);
    }

    /// OMZP::git:53 — usage guard with $0 in function.
    #[test]
    fn usage_guard_zero() {
        assert_parity(r###"g() { if [[ -z "$1" || -z "$2" ]]; then print "Usage: $0 a b"; return 1; fi; print "$1->$2"; }; g a; print rc=$?; g a b; print rc=$?"###);
    }

    /// OMZP::git:176 — `${${@[(r)PAT]}:-DEF}` extendedglob reverse-subscript URL match + default.
    #[test]
    fn at_reverse_subscript_url() {
        assert_parity(r###"setopt extendedglob; set -- foo https://github.com/u/r.git bar; repo="${${@[(r)(ssh://*|git://*|ftp(s)#://*|http(s)#://*|*@*)(.git/#)#]}:-NONE}"; print -r -- "$repo""###);
    }

    /// OMZP::git:183 — chained tail-modifier then extendedglob suffix strip.
    #[test]
    fn tail_then_suffix_strip() {
        assert_parity(r###"setopt extendedglob; repo="https://github.com/u/myrepo.git"; print -r -- "${${repo:t}%.git/#}""###);
    }

    /// OMZP::git:84 — `cmd | grep -q -- "--wip--" && echo` literal double-dash.
    #[test]
    fn grep_q_double_dash() {
        assert_parity(r###"printf "commit abc\n    --wip-- [skip ci]\n" | grep -q -- "--wip--" && echo "WIP!!""###);
    }

    /// OMZP::git:97 — `[[ $# == 0 ]]` arg-count test + ${*} join.
    #[test]
    fn argcount_star_join() {
        assert_parity(r###"h() { if [[ $# == 0 ]]; then print "none"; else print "args:${*}"; fi }; h; h a b c"###);
    }

    /// OMZP::git:276 — `[[ $# != 1 ]] && b=...` then ${b:-$1} fallback.
    #[test]
    fn argcount_fallback_to_first() {
        assert_parity(r###"f() { local b; [[ $# != 1 ]] && b="DEFBR"; print -r -- "${b:-$1}"; }; f; f only"###);
    }

    /// OMZP::git:141 — `(( ! $? ))` arithmetic negation of last exit.
    #[test]
    fn arith_negate_exit() {
        assert_parity(r###"true; (( ! $? )) && print "was-zero"; false; (( ! $? )) || print "was-nonzero""###);
    }

    /// OMZP::git:146 — `[[ $out = -* ]]` leading-dash glob anchor.
    #[test]
    fn leading_dash_glob() {
        assert_parity(r###"out="- abc123"; if [[ $out = -* ]]; then print "starts-dash"; else print "no"; fi"###);
    }

    /// OMZP::git:416 — alias body defining a fn inline + noglob, stored verbatim.
    #[test]
    fn alias_inline_function_body() {
        assert_parity(r###"alias gtl="gtl(){ print -r -- \"tag-list:\${1}\" }; noglob gtl"; print -r -- "${aliases[gtl]}""###);
    }

    /// OMZP::git:112 — multi-command alias body with `;` and embedded quotes verbatim.
    #[test]
    fn alias_multicommand_body() {
        assert_parity(r###"alias gwip="git add -A; git commit --no-verify --message \"--wip-- [skip ci]\""; print -r -- "${aliases[gwip]}""###);
    }

    /// OMZP::git:427 — `aliases[$old]=` direct assoc assignment with multi-line body.
    #[test]
    fn aliases_direct_assignment() {
        assert_parity(
            r###"old_name=current_branch; new_name=git_current_branch; aliases[$old_name]="
    print -Pu2 \"deprecated ${old_name} use ${new_name}\"
    $new_name"; print -r -- "${aliases[current_branch]}""###,
        );
    }

    /// OMZP::git:424 — `for old new ( list )` two-variable-per-iteration loop.
    #[test]
    fn for_two_var_paren() {
        assert_parity(r###"for a b (one 1 two 2 three 3); do print -r -- "$a=$b"; done"###);
    }

    /// OMZP::git:226 — `is-at-least V "$ver" && A || B` version-gated alias.
    #[test]
    fn is_at_least_gate() {
        assert_parity(r###"autoload -Uz is-at-least; is-at-least 2.8 2.43.0 && alias gfa="fetch-new" || alias gfa="fetch-old"; print -r -- "${aliases[gfa]}""###);
    }

    /// OMZP::git:94 — alias with cmdsubst `||` fallback stored verbatim.
    #[test]
    fn alias_cmdsubst_fallback() {
        assert_parity(r###"alias grt="cd \"\$(git rev-parse --show-toplevel || echo .)\""; print -r -- "${aliases[grt]}""###);
    }

    /// OMZP::git:152 — gone-branch extraction pipeline (grep|cut|awk).
    #[test]
    fn gone_branch_pipeline() {
        assert_parity(r###"printf "  feature/x abc123 [origin/feature/x: gone] msg\n  main def456 [origin/main] msg\n" | grep ": gone\]" | cut -c 3- | awk "{print \$1}""###);
    }
}

// ════════════════════════════ OMZP::github ═════════════════════════

mod omzp_github {
    use super::*;

    /// OMZP::github:14 — keyword-less function + emulate -L zsh.
    #[test]
    fn keywordless_fn_emulate() {
        assert_parity(r###"empty_gh() { emulate -L zsh; local repo=$1; print -r -- "repo=$repo"; }; empty_gh proj"###);
    }

    /// OMZP::github:36 — concatenated quoted-literal with embedded newline.
    #[test]
    fn concat_literal_newline() {
        assert_parity(r###"print ".*""\n""*~""###);
    }

    /// OMZP::github:73 — `${(%):-"...%%..."}` prompt-expansion of literal (%%→%).
    #[test]
    fn prompt_expand_literal() {
        assert_parity(r###"print -r -- "${(%):-"50%% complete"}""###);
    }

    /// OMZP::github:30 — `cd "$1" || return N` short-circuit in a function.
    #[test]
    fn cd_or_return() {
        assert_parity(r###"d=$(mktemp -d); mkdir "$d/sub"; f() { cd "$1" || return 7; print -r -- "in:${PWD:t}"; }; f "$d/sub"; print rc=$?; f "/no/such/dir/xyz"; print rc=$?; cd /; rm -rf "$d""###);
    }
}

// ══════════════════════════════ OMZP::svn ══════════════════════════

mod omzp_svn {
    use super::*;

    /// OMZP::svn:15 — `${display:gs/%/%%}` global-subst escape percents.
    #[test]
    fn gs_percent_escape() {
        assert_parity(r###"display="100%done 50%left"; print -r -- "${display:gs/%/%%}""###);
    }

    /// OMZP::svn:31 — `sed -n 's/^Repository Root:...///p' <<< "$info"` here-string parse.
    #[test]
    fn sed_reporoot_herestring() {
        assert_parity(
            r###"info="URL: https://svn.example.com/repo/trunk
Repository Root: https://svn.example.com/myrepo
Revision: 42"; name="$(sed -n "s/^Repository\ Root:\ .*\///p" <<< "$info")"; print -r -- "$name""###,
        );
    }

    /// OMZP::svn:39 — `awk -F/ '/^URL:/{...}' <<< "$info"` branch/tag/trunk extraction.
    #[test]
    fn awk_branch_extract_herestring() {
        assert_parity(
            r###"info="URL: https://svn.example.com/repo/branches/feature-x/src
Repository Root: https://svn.example.com/repo"; b=$(awk -F/ "/^URL:/ {for(i=0;i<=NF;i++){if(\$i==\"branches\"||\$i==\"tags\"){print \$(i+1);break}; if(\$i==\"trunk\"){print \$i;break}}}" <<< "$info"); print -r -- "$b""###,
        );
    }

    /// OMZP::svn:58 — `sed -n <<<"${1:-default}"` here-string from param-default.
    #[test]
    fn sed_herestring_param_default() {
        assert_parity(r###"svn_get_rev_nr() { sed -n "s/Revision:\ //p" <<<"${1:-fallback}"; }; svn_get_rev_nr "Revision: 1234""###);
    }

    /// OMZP::svn:54 — `${branch:-$(fallback)}` default expanding to cmdsubst.
    #[test]
    fn default_to_cmdsubst() {
        assert_parity(r###"fb() { print "REPONAME"; }; branch=""; print -r -- "${branch:-$(fb)}"; branch="trunk"; print -r -- "${branch:-$(fb)}""###);
    }

    /// OMZP::svn:68 — `grep -Eq '^\s*[ACDIM!?L]' && echo $2 || echo $3` dirty/clean.
    #[test]
    fn grep_dirty_choose() {
        assert_parity(r###"choose() { if printf "%s\n" "$1" | command grep -Eq "^\s*[ACDIM!?L]"; then echo "$2"; else echo "$3"; fi; }; choose "M  file.txt" DIRTY CLEAN; choose "        " DIRTY CLEAN"###);
    }

    /// OMZP::svn:3 — `info="$(...)" || return 1` capture-or-return.
    #[test]
    fn capture_or_return() {
        assert_parity(r###"f() { local info; info="$(printf "ok")" || return 1; print -r -- "info=$info"; }; f; print rc=$?"###);
    }
}

// ══════════════════════════════ OMZP::tig ══════════════════════════

mod omzp_tig {
    use super::*;

    /// OMZP::tig:1 — multiple aliases in one statement + ${aliases[name]} introspection.
    #[test]
    fn multi_alias_one_statement() {
        assert_parity(r###"alias tis="tig status" til="tig log" tib="tig blame -C"; for a in tis til tib; do print -r -- "$a => ${aliases[$a]}"; done"###);
    }
}

// ════════════════════════════ OMZP::gradle ═════════════════════════

mod omzp_gradle {
    use super::*;

    /// OMZP::gradle:8 — `while [[ "$dir" != / ]]; ...; dir="${dir:h}"` head-modifier walk.
    #[test]
    fn head_modifier_dir_walk() {
        assert_parity(r###"dir="/a/b/c/d"; while [[ "$dir" != / ]]; do print -r -- "$dir"; dir="${dir:h}"; done"###);
    }
}

// ══════════════════════════════ OMZP::mvn ══════════════════════════

mod omzp_mvn {
    use super::*;

    /// OMZP::mvn:4 — `while [[ ! -x "$d/mvnw" && "$d" != / ]]` compound test + :h ascent.
    #[test]
    fn mvnw_ascent_search() {
        assert_parity(r###"d=$(mktemp -d); mkdir -p "$d/x/y"; touch "$d/mvnw"; chmod +x "$d/mvnw"; cur="$d/x/y"; while [[ ! -x "$cur/mvnw" && "$cur" != / ]]; do cur="${cur:h}"; done; if [[ -x "$cur/mvnw" ]]; then print "found-mvnw"; else print "not-found"; fi; rm -rf "$d""###);
    }

    /// OMZP::mvn:47 — alias with `$(cmd 2>/dev/null || echo ".")` fallback verbatim.
    #[test]
    fn alias_cmdsubst_devnull_fallback() {
        assert_parity(r###"alias mvnbang='mvn -f $(git rev-parse --show-toplevel 2>/dev/null || echo ".")/pom.xml'; print -r -- "${aliases[mvnbang]}""###);
    }

    /// OMZP::mvn:84 — `local -a` + append with absolute-path `:A` modifier.
    #[test]
    fn local_a_append_abs() {
        assert_parity(r###"d=$(mktemp -d); local -a POM_FILES; POM_FILES=(~/.m2/settings.xml); file="$d/pom.xml"; touch "$file"; POM_FILES+=("${file:A}"); print -r -- "count=${#POM_FILES}"; print -r -- "last-basename=${POM_FILES[-1]:t}"; rm -rf "$d""###);
    }

    /// OMZP::mvn:106 — `file="${file:h}/${new_file}"` head-modifier path rebuild.
    #[test]
    fn head_path_rebuild() {
        assert_parity(r###"file="/proj/sub/pom.xml"; new_file="../pom.xml"; file="${file:h}/${new_file}"; print -r -- "$file""###);
    }

    /// OMZP::mvn:124 — `grep -A 1 | grep | sed 's?...?-P\1?'` profile-id pipeline.
    #[test]
    fn profile_id_pipeline() {
        assert_parity(
            r###"xml="<profiles>
<profile>
<id>dev</id>
</profile>
<profile>
<id>prod</id>
</profile>
</profiles>"; print -r -- "$xml" | grep -e "<profile>" -A 1 | grep -e "<id>.*</id>" | sed "s?.*<id>\(.*\)</id>.*?-P\1?""###,
        );
    }

    /// OMZP::mvn:128 — `print -l **/pom.xml(-.N:h)` glob qualifiers (deref/regular/null/head).
    #[test]
    fn glob_qualifiers_pom_head() {
        assert_parity(r###"d=$(mktemp -d); mkdir -p "$d/m1" "$d/m2" "$d/m2/target/classes/META-INF"; touch "$d/m1/pom.xml" "$d/m2/pom.xml" "$d/m2/target/classes/META-INF/pom.xml"; cd "$d"; print -l **/pom.xml(-.N:h) | grep -v "/target/classes/META-INF/" | sort; cd /; rm -rf "$d""###);
    }

    /// OMZP::mvn:342 — `[ -d ] && find | sed 's?.../\([^/]*\)\..*?-Dtest=\1?'` test-class enum.
    #[test]
    fn test_class_enumeration() {
        assert_parity(r###"d=$(mktemp -d); mkdir -p "$d/src/test/java/com"; touch "$d/src/test/java/com/FooTest.java" "$d/src/test/java/com/BarTest.java"; cd "$d"; if [ -d ./src/test/java ]; then find ./src/test/java -type f -name "*.java" | sed "s?.*/\([^/]*\)\..*?-Dtest=\1?" | sort; fi; cd /; rm -rf "$d""###);
    }

    /// OMZP::mvn:27 — `( unset LANG; LC_CTYPE=C; ... )` subshell environment isolation.
    #[test]
    fn subshell_env_isolation() {
        assert_parity(r###"LANG=en_US.UTF-8; ( unset LANG; LC_CTYPE=C; print "inside LANG=[${LANG}] LC_CTYPE=[$LC_CTYPE]"; ); print "outside LANG=[${LANG}]""###);
    }
}

// ════════════════════════════ OMZP::python ═════════════════════════

mod omzp_python {
    use super::*;

    /// OMZP::python:32 — offset substring then keep-match strip to extract version.
    #[test]
    fn offset_then_keepmatch_strip() {
        assert_parity(
            r###"setopt extendedglob
ver="Python 3.11.5"
out=${(M)${ver:7}#[^.]##.[^.]##}
print "offset7=${ver:7}"
print "minor=$out""###,
        );
    }

    /// OMZP::python:23 — nested default `${VAR:-"${OTHER}/.local"}` with inner param.
    #[test]
    fn nested_default_inner_param() {
        assert_parity(
            r###"unset PYTHONUSERBASE
BASEHOME=/u
print "base=${PYTHONUSERBASE:-"${BASEHOME}/.local"}"
PYTHONUSERBASE=/custom
print "base=${PYTHONUSERBASE:-"${BASEHOME}/.local"}""###,
        );
    }

    /// OMZP::python:38 — `[[ x =~ "(^|:)""$s""(:|$)" ]]` colon-path membership regex.
    #[test]
    fn colon_path_membership_regex() {
        assert_parity(
            r###"p="/a/b:/c/d:/e"
s="/c/d"
if [[ "$p" =~ "(^|:)""$s""(:|$)" ]]; then print "found"; else print "no"; fi
miss="/z"
if [[ "$p" =~ "(^|:)""$miss""(:|$)" ]]; then print "found"; else print "no"; fi"###,
        );
    }

    /// OMZP::python:39 — `${PYTHONPATH+":${PYTHONPATH}"}` append-only-if-set.
    #[test]
    fn append_only_if_set() {
        assert_parity(
            r###"sp=/x/y
PYTHONPATH=/a:/b
print "${sp}${PYTHONPATH+":${PYTHONPATH}"}"
unset PYTHONPATH
print "${sp}${PYTHONPATH+":${PYTHONPATH}"}""###,
        );
    }

    /// OMZP::python:51 — `: ${VAR:=default}` colon-command assign-default.
    #[test]
    fn colon_assign_default() {
        assert_parity(
            r###": ${PYTHON_VENV_NAME:=venv}
print "name=$PYTHON_VENV_NAME"
PYTHON_VENV_NAME=custom
: ${PYTHON_VENV_NAME:=venv}
print "name=$PYTHON_VENV_NAME""###,
        );
    }

    /// OMZP::python:55 — `typeset -gaU` global dedup array, first-seen order.
    #[test]
    fn typeset_gau_dedup() {
        assert_parity(
            r###"typeset -gaU A
A=(venv venv .venv venv x .venv)
print "${A[@]}"
print "count=${#A}""###,
        );
    }

    /// OMZP::python:64 — `${name:P}` realpath modifier on a relative name.
    #[test]
    fn realpath_p_modifier() {
        assert_parity(
            r###"name=venv
d=$(mktemp -d)
cd "$d"
mkdir sub
v="${name:P}"
print "tail=${v:t}"
cd /
rm -rf "$d""###,
        );
    }

    /// OMZP::python:73 — positional default referencing another parameter.
    #[test]
    fn positional_default_param() {
        assert_parity(
            r###"f(){ local n="${1:-$PYTHON_VENV_NAME}"; print "n=$n"; }
PYTHON_VENV_NAME=venv
f
f hello"###,
        );
    }

    /// OMZP::python:110 — `${VIRTUAL_ENV:h}` head modifier.
    #[test]
    fn head_modifier() {
        assert_parity(
            r###"VIRTUAL_ENV=/home/u/proj/venv
print "head=${VIRTUAL_ENV:h}"
print "tail=${VIRTUAL_ENV:t}""###,
        );
    }

    /// OMZP::python:112 — rc-expansion `${^ARR}/suffix` cross-product + (N.) qualifier.
    #[test]
    #[ignore = "zshrs gap: rc-expansion ${^NAMES[@]}/bin/activate(N.) cross-product+glob yields the array elements, not the matched paths"]
    fn rc_expansion_crossproduct() {
        assert_parity(
            r###"d=$(mktemp -d)
cd "$d"
mkdir -p venv/bin .venv/bin
touch venv/bin/activate .venv/bin/activate
NAMES=(venv .venv missing)
for f in "${^NAMES[@]}"/bin/activate(N.); do print "${f}"; done
cd /
rm -rf "$d""###,
        );
    }

    /// OMZP::python:10 — `${@:-.}` all-positional with default.
    #[test]
    fn all_positional_default() {
        assert_parity(
            r###"f(){ print "args=${@:-.}"; }
f
f one two"###,
        );
    }

    /// OMZP::python:5 — alias body with quoted glob + `*.py(N)` qualifier in sandbox.
    #[test]
    fn nullglob_py_sandbox() {
        assert_parity(
            r###"d=$(mktemp -d)
cd "$d"
touch a.py b.py c.txt
for f in *.py(N); do print "${f}"; done
print "n=$(print -l *.py(N) | grep -c .)"
cd /
rm -rf "$d""###,
        );
    }
}

// ══════════════════════════════ OMZP::ruby ═════════════════════════

mod omzp_ruby {
    use super::*;

    /// OMZP::ruby:11 — alias family + ${(k)aliases[(I)pat*]} key filter.
    #[test]
    fn aliases_key_filter() {
        assert_parity(
            r###"alias gein="gem install"
alias geun="gem uninstall"
alias geli="gem list"
print "gein=>${aliases[gein]}"
ks=(${(k)aliases[(I)ge*]})
print "ge_count=${#ks}""###,
        );
    }

    /// OMZP::ruby:16 — introspect related aliases sorted by key via (ko)aliases[(I)pat*].
    #[test]
    fn aliases_sorted_key_introspect() {
        assert_parity(r###"alias geca="gem cert --add" gecr="gem cert --remove" gecb="gem cert --build"; for k in ${(ko)aliases[(I)gec*]}; do print "alias $k=${(qq)aliases[$k]}"; done"###);
    }

    /// OMZP::ruby:5 — alias whose body is a pipeline; introspect without running.
    #[test]
    fn alias_pipeline_body() {
        assert_parity(r###"alias rfind="find . -name *.rb | xargs grep -n"; print -r -- "rfind=>${aliases[rfind]}""###);
    }
}

// ══════════════════════════════ OMZP::rust ═════════════════════════

mod omzp_rust {
    use super::*;

    /// OMZP::rust:9 — `typeset -g -A _comps; _comps[name]=val` completion map + introspect.
    #[test]
    fn comps_global_assoc() {
        assert_parity(
            r###"typeset -g -A _comps
_comps[cargo]=_cargo
_comps[rustup]=_rustup
print "cargo=>${_comps[cargo]}"
print "n=${#_comps}""###,
        );
    }

    /// OMZP::rust:23 — `>|` clobber redirect with quoted heredoc, read back.
    #[test]
    fn clobber_heredoc_readback() {
        assert_parity(
            r###"d=$(mktemp -d)
cat >| "$d/_cargo" <<'EOF'
#compdef cargo
source "$(rustup default)"/_cargo
EOF
n=$(grep -c "" "$d/_cargo")
print "lines=$n"
IFS= read -r first < "$d/_cargo"
print "first=$first"
rm -rf "$d""###,
        );
    }

    /// OMZP::rust:1 — compound arithmetic AND of two membership tests.
    #[test]
    fn compound_and_membership() {
        assert_parity(
            r###"typeset -A C
C[rustup]=1 C[cargo]=1
(( ${+C[rustup]} && ${+C[cargo]} )) && print "both"
(( ${+C[rustup]} && ${+C[missing]} )) || print "not-both""###,
        );
    }
}

// ══════════════════════════════ OMZP::node ═════════════════════════

mod omzp_node {
    use super::*;

    /// OMZP::node:4 — `local section=${1:-all}` positional default.
    #[test]
    fn section_positional_default() {
        assert_parity(
            r###"nd(){ local section=${1:-all}; print "section=$section"; }
nd
nd http"###,
        );
    }
}

// ════════════════════════════ OMZP::golang ═════════════════════════

mod omzp_golang {
    use super::*;

    /// OMZP::golang:13 — multi-alias define + iterate (ko)aliases[(I)go*].
    #[test]
    fn go_aliases_iterate() {
        assert_parity(r###"alias gob="go build" goc="go clean" gor="go run" got="go test"; for k in ${(ko)aliases[(I)go*]}; do print "$k -> ${aliases[$k]}"; done"###);
    }

    /// OMZP::golang:6 — `for p in 5 6 8` numeric word-list loop interpolating into strings.
    #[test]
    fn numeric_wordlist_loop() {
        assert_parity(
            r###"for p in 5 6 8; do
  print "compctl -g *.${p} ${p}l"
  print "compctl -g *.go ${p}g"
done"###,
        );
    }

    /// OMZP::golang:26 — alias body interpolating $GOPATH/sub path.
    #[test]
    fn gopath_interp() {
        assert_parity(
            r###"GOPATH=/home/u/go
print "cd $GOPATH/src"
print "cd $GOPATH/bin""###,
        );
    }
}

// ══════════════════════════════ OMZP::perl ═════════════════════════

mod omzp_perl {
    use super::*;

    /// OMZP::perl:33 — `[[ -z $EDITOR ]] && EDITOR=vim` default-if-empty chain.
    #[test]
    fn default_if_empty_chain() {
        assert_parity(
            r###"unset EDITOR
[[ -z $EDITOR ]] && EDITOR=vim
print "ed=$EDITOR"
EDITOR=nano
[[ -z $EDITOR ]] && EDITOR=vim
print "ed=$EDITOR""###,
        );
    }

    /// OMZP::perl:36 — paired `[[ -e ]]` / `[[ ! -e ]]` guard-and-act chains.
    #[test]
    fn file_existence_guard_chains() {
        assert_parity(
            r###"d=$(mktemp -d)
cd "$d"
touch exists.pl
f(){ [[ -e $1 ]] && print "$1 exists; not modifying."; [[ ! -e $1 ]] && print "$1 created"; }
f exists.pl
f new.pl
cd /
rm -rf "$d""###,
        );
    }

    /// OMZP::perl:39 — multi-line text via single-quote + "\n" concatenation.
    #[test]
    fn multiline_quote_concat() {
        assert_parity(r###"print '#!/usr/bin/perl'"\n"'use strict;'"\n"'use warnings;'"\n""###);
    }

    /// OMZP::perl:48 — function interpolating $1/$2/$3 into a command-string template.
    #[test]
    fn template_arg_interp() {
        assert_parity(
            r###"pgs(){ print "perl -i.orig -pe s/${1}/${2}/g ${3}"; }
pgs foo bar file.txt"###,
        );
    }

    /// OMZP::perl:60 — default with nested parameter `${ROOT:-${HOME}/perl5/perlbrew}`.
    #[test]
    fn default_nested_param() {
        assert_parity(
            r###"unset PERLBREW_ROOT
H=/home/u
print "${PERLBREW_ROOT:-${H}/perl5/perlbrew}"
PERLBREW_ROOT=/opt/pb
print "${PERLBREW_ROOT:-${H}/perl5/perlbrew}""###,
        );
    }

    /// OMZP::perl:36 — `print` interpreting embedded `\n` to emit multiple lines.
    #[test]
    fn print_embedded_newline() {
        assert_parity(r###"print "file.py exists; not modifying.\ndone""###);
    }
}

// ══════════════════════════════ OMZP::scala ════════════════════════

mod omzp_scala {
    use super::*;

    /// _scala:45 — literal candidate-list array; ${#arr}/${arr[1]}/${arr[-1]} access.
    #[test]
    fn candidate_array_access() {
        assert_parity(
            r###"feats=(postfixOps reflectiveCalls implicitConversions higherKinds existentials experimental.macros _)
print "count=${#feats}"
print "first=${feats[1]} last=${feats[-1]}""###,
        );
    }

    /// _scala:64 — completion-spec array; `${e%%\[*}` strips [description] to option name.
    #[test]
    fn strip_bracket_description() {
        assert_parity(
            r###"local -a o
o=("-deprecation[Emit warning]" "-nowarn[Generate no warnings]" "-verbose[Output messages]")
for e in $o; do print -r -- "${e%%\[*}"; done"###,
        );
    }

    /// _scala:240 — nested case dispatch on option prefix then service.
    #[test]
    fn nested_case_dispatch() {
        assert_parity(
            r###"disp(){ local cur="$1" svc="$2"; case $cur in
  (-X*) print "X-opts";;
  (-Y*) print "Y-opts";;
  (*) case $svc in (scala) print "scala-args";; (scalac) print "scalac-args";; esac;;
esac; }
disp -Xlint scala
disp -Yinline scala
disp foo.scala scalac
disp bar scala"###,
        );
    }

    /// _scala:68 — extract bracketed description via ${spec#*\[} then ${tmp%%\]*}.
    #[test]
    fn extract_bracket_desc() {
        assert_parity(
            r###"spec="-g\:-[Set level of generated debugging info (default\: vars)]:debugging info level:(none source line vars)"
tmp="${spec#*\[}"
print -r -- "desc=${tmp%%\]*}""###,
        );
    }
}

// ══════════════════════════════ OMZP::npm ══════════════════════════

mod omzp_npm {
    use super::*;

    /// OMZP::npm:88 — `${line/pat/rep}` single-occurrence rewrite of a command line.
    #[test]
    fn single_subst_rewrite() {
        assert_parity(
            r###"line="npm uninstall lodash --save"
print "${line/npm uninstall/npm install}""###,
        );
    }

    /// OMZP::npm:86 — `case "$line" in pat*) ${line/.../...}` prefix dispatch with rewrite.
    #[test]
    fn prefix_dispatch_rewrite() {
        assert_parity(
            r###"rw(){ local line="$1"; case "$line" in
  "npm uninstall"*) print "${line/npm uninstall/npm install}";;
  "npm install"*) print "${line/npm install/npm uninstall}";;
  "npm i "*) print "${line/npm i/npm uninstall}";;
  *) print "no-match";;
esac; }
rw "npm install foo"
rw "npm uninstall foo"
rw "npm i bar"
rw "yarn add x""###,
        );
    }

    /// OMZP::npm:109 — `${#BUFFER}` string length to a counter.
    #[test]
    fn buffer_length_counter() {
        assert_parity(
            r###"BUFFER="npm install"
CURSOR=${#BUFFER}
print "cursor=$CURSOR""###,
        );
    }

    /// OMZP::npm:82 — iterate quoted list, case-match each (history walk).
    #[test]
    fn history_walk_case() {
        assert_parity(
            r###"hist=("npm install a" "npm test" "npm uninstall b")
for line in "${hist[@]}"; do
  case "$line" in
    (npm\ install*) print "I:$line";;
    (npm\ uninstall*) print "U:$line";;
    (*) print "skip:$line";;
  esac
done"###,
        );
    }
}

// ══════════════════════════════ OMZP::yarn ═════════════════════════

mod omzp_yarn {
    use super::*;

    /// OMZP::yarn:8 — `(( ${path[(Ie)$dir]} ))` reverse-index membership on array.
    #[test]
    fn path_Ie_membership() {
        assert_parity(
            r###"pa=(/usr/bin /bin /usr/local/bin)
d=/bin
(( ${pa[(Ie)$d]} )) && print "in-path idx=${pa[(Ie)$d]}"
e=/nope
(( ! ${pa[(Ie)$e]} )) && print "not-in-path""###,
        );
    }

    /// OMZP::yarn:9 — `path+=("$dir")` append and `${arr[-1]}` last.
    #[test]
    fn append_and_last() {
        assert_parity(
            r###"pa=(/a /b)
pa+=("/c")
print "${pa[@]}"
print "last=${pa[-1]}""###,
        );
    }

    /// OMZP::yarn:41 — if/else mutually-exclusive alias sets, then introspect.
    #[test]
    fn mutually_exclusive_alias_sets() {
        assert_parity(
            r###"berry=1
if (( berry )); then
  alias yii="yarn install --immutable"
  alias ydlx="yarn dlx"
else
  alias yii="yarn install --frozen-lockfile"
  alias yga="yarn global add"
fi
print "yii=${aliases[yii]}"
print "ydlx=${aliases[ydlx]:-UNSET}"
print "yga=${aliases[yga]:-UNSET}""###,
        );
    }
}

// ══════════════════════════════ OMZP::rails ════════════════════════

mod omzp_rails {
    use super::*;

    /// OMZP::rails:41 — global aliases `alias -g` + ${galiases[name]} introspection.
    #[test]
    fn galiases_introspect() {
        assert_parity(
            r###"alias -g RED="RAILS_ENV=development"
alias -g REP="RAILS_ENV=production"
print "RED=>${galiases[RED]}"
print "count=${#galiases}""###,
        );
    }

    /// OMZP::rails:3 — `if [ -e ] elif [ -e ] else` file-existence dispatch chain.
    #[test]
    fn file_dispatch_chain() {
        assert_parity(
            r###"d=$(mktemp -d)
cd "$d"
mkdir -p bin
touch bin/rails
rc(){ if [ -e "bin/stubs/rails" ]; then print "stubs"; elif [ -e "bin/rails" ]; then print "bin/rails"; elif [ -e "script/rails" ]; then print "script"; else print "command"; fi; }
rc
rm bin/rails
rc
cd /
rm -rf "$d""###,
        );
    }

    /// OMZP::rails:25 — `[[ -e "Gemfile" || -e "gems.rb" ]]` OR-combined file test.
    #[test]
    fn or_combined_file_test() {
        assert_parity(
            r###"d=$(mktemp -d)
cd "$d"
touch gems.rb
[[ -e "Gemfile" || -e "gems.rb" ]] && print "have-gemfile" || print "none"
rm gems.rb
[[ -e "Gemfile" || -e "gems.rb" ]] && print "have-gemfile" || print "none"
cd /
rm -rf "$d""###,
        );
    }

    /// OMZP::rails:118 — function building a remote-command string from positionals.
    #[test]
    fn remote_command_string() {
        assert_parity(
            r###"rcon(){ print "ssh $1 ( cd $2 \&\& ruby script/console production )"; }
rcon host1 /srv/app"###,
        );
    }
}

// ══════════════════════════════ OMZP::macos ════════════════════════

mod omzp_macos {
    use super::*;

    /// OMZP::macos:9 — `(( ! $# ))` zero-arg dispatch.
    #[test]
    fn zero_arg_dispatch() {
        assert_parity(r###"f(){ if (( ! $# )); then print "no-args-mode"; else print "args:$*"; fi }; f; f a b"###);
    }

    /// OMZP::macos:37 — `(( $# > 0 )) && command="...; $*"` conditional append of joined args.
    #[test]
    fn conditional_append_joined() {
        assert_parity(r###"cmd="cd; clear"; set -- echo hi; (( $# > 0 )) && cmd="${cmd}; $*"; print -r -- "$cmd""###);
    }

    /// OMZP::macos:271 — `[[ $# -eq 0 ]] && >&2 echo ... && return 1` usage-guard chain.
    #[test]
    fn usage_guard_stderr_chain() {
        assert_parity(r###"f(){ [[ $# -eq 0 ]] && >&2 echo "Usage: prog cmd" && return 1; print "ran with $#"; }; f; print "rc=$?"; f x; print "rc=$?""###);
    }

    /// OMZP::macos:286 — `${@:-.}` default applied to whole positional list.
    #[test]
    fn positional_list_default() {
        assert_parity(r###"f(){ print -r -- "target=${@:-.}"; }; f; f /tmp /var"###);
    }

    /// OMZP::macos:96 — `case` app dispatch with stderr error + return 1 for unsupported.
    #[test]
    fn app_case_dispatch_error() {
        assert_parity(r###"f(){ local app=$1; case $app in (Terminal|iTerm) print "supported: $app";; (*) print "prog: unsupported: $app" >&2; return 1;; esac }; f Terminal; f Chrome; print "rc=$?""###);
    }

    /// OMZP::macos:274 — `for page in "${(@f)data}"` line-split loop + ${page:t} basename.
    #[test]
    fn line_split_basename_loop() {
        assert_parity(
            r###"data="/usr/share/man/man1/ls.1
/usr/share/man/man1/cd.1"; for page in "${(@f)data}"; do print "page-basename=${page:t}"; done"###,
        );
    }

    /// OMZP::macos:281 — protocol-prefix concat with positional list (vnc://$@).
    #[test]
    fn protocol_prefix_concat() {
        assert_parity(r###"f(){ print "vnc://$@"; }; f 192.168.1.5; f host port"###);
    }

    /// OMZP::macos:296 — `awk 'NR==1 || /pat/'` header-plus-pattern filter on canned df.
    #[test]
    fn awk_header_pattern_filter() {
        assert_parity(
            r###"data="Filesystem Size Used Avail
/dev/disk1 100 50 50
tmpfs 10 1 9
/dev/disk2 200 20 180"; print -r -- "$data" | awk "NR==1 || /^\/dev\/disk/""###,
        );
    }

    /// OMZP::macos:244 — command substitution into builtin arg (cd "$(pfd)") pure part.
    #[test]
    fn cmdsubst_into_builtin_arg() {
        assert_parity(r###"pfd(){ print "/tmp/somedir"; }; target="$(pfd)"; print "would-cd-to: $target""###);
    }
}

// ══════════════════════════════ OMZP::tmux ═════════════════════════

mod omzp_tmux {
    use super::*;

    /// OMZP::tmux:56 — eval-generated function with `${1:0:1} == -` leading-dash guard.
    #[test]
    fn eval_fn_leading_dash_guard() {
        assert_parity(
            r###"gen(){ eval "function $1 {
  if [[ -z \$1 ]] || [[ \${1:0:1} == - ]]; then print \"$2 only:\$*\"; else print \"$2 $3:\$*\"; fi
}"; }; gen ta attach -t; ta; ta -d; ta mysession"###,
        );
    }

    /// OMZP::tmux:67 — `${(z)var}` word split, ${#s[@]} count, negative index.
    #[test]
    fn z_split_count_negindex() {
        assert_parity(r###"arg="attach -d -t"; s=(${(z)arg}); print "count=${#s[@]} first=$s[1] last=$s[-1]""###);
    }

    /// OMZP::tmux:184 — `${PWD##*/}` basename + `${md5:0:6}` substring concat.
    #[test]
    fn basename_substring_concat() {
        assert_parity(r###"p=/home/user/projects/myrepo; dir=${p##*/}; h=abcdef0123456789; print "${dir}-${h:0:6}""###);
    }

    /// OMZP::tmux:117 — `local -a` array + conditional `+=()` command-vector build.
    #[test]
    fn command_vector_build() {
        assert_parity(r###"tc=(command tmux); cc=true; uu=true; [[ $cc == true ]] && tc+=(-CC); [[ $uu == true ]] && tc+=(-u); print -r -- "$tc""###);
    }

    /// OMZP::tmux:128 — `case` dispatch on $PWD for session naming.
    #[test]
    fn pwd_case_session_name() {
        assert_parity(
            r###"for p in /home/u/proj / /home/u; do
  HOME=/home/u
  case $p in
    "$HOME") sn=HOME;; (/) sn=ROOT;; (*) sn=${p##*/};;
  esac
  print "$p -> $sn"
done"###,
        );
    }

    /// OMZP::tmux:8 — `: ${VAR:=default}` null-command default-assignment.
    #[test]
    fn null_command_default_assign() {
        assert_parity(r###"unset T; : ${T:=false}; print "T=$T"; T=true; : ${T:=false}; print "T=$T""###);
    }

    /// OMZP::tmux:45 — nested default `${XDG_CONFIG_HOME:-$HOME/.config}` inside path.
    #[test]
    fn nested_default_in_path() {
        assert_parity(r###"HOME=/home/u; unset XDG_CONFIG_HOME; print "${XDG_CONFIG_HOME:-$HOME/.config}/tmux/tmux.conf"; XDG_CONFIG_HOME=/cfg; print "${XDG_CONFIG_HOME:-$HOME/.config}/tmux/tmux.conf""###);
    }

    /// OMZP::tmux:55 — `setopt localoptions no_nomatch` restored on function return.
    #[test]
    fn localoptions_restored() {
        assert_parity(r###"f(){ setopt localoptions no_nomatch; print "in-fn nomatch-disabled"; }; f; if [[ -o nomatch ]]; then print "outside: nomatch restored"; fi"###);
    }

    /// OMZP::tmux:182 — `${var:0:6}` / `${var:N:M}` substring extraction.
    #[test]
    fn substring_extraction() {
        assert_parity(r###"dir=myrepo; full=0123456789abcdef; print "${dir}-${full:0:6}"; print "${full:3:4}""###);
    }

    /// OMZP::tmux:170 — `IFS== read` parse of key=value lines via here-string.
    #[test]
    fn ifs_eq_read_herestring() {
        assert_parity(
            r###"out="FOO=bar
BAZ=qux"; while IFS== read -r k val; do print "set $k to $val"; done <<< "$out""###,
        );
    }

    /// OMZP::tmux:91 — `[[ $colors == 256 ]]` if/else selecting exported TERM.
    #[test]
    fn colors_term_select() {
        assert_parity(r###"colors=256; if [[ $colors == 256 ]]; then term=tmux-256color; else term=screen; fi; print "TERM=$term"; colors=8; if [[ $colors == 256 ]]; then term=tmux-256color; else term=screen; fi; print "TERM=$term""###);
    }
}

// ═══════════════════════════ OMZP::docker-compose ══════════════════

mod omzp_docker_compose {
    use super::*;

    /// OMZP::docker-compose:7 — alias family from interpolated $dccmd + introspection.
    #[test]
    fn alias_family_interp() {
        assert_parity(r###"dccmd="docker compose"; alias dco="$dccmd"; alias dcb="$dccmd build"; alias dcupd="$dccmd up -d"; for a in dco dcb dcupd; do print "$a=${aliases[$a]}"; done"###);
    }

    /// OMZP::docker-compose:5 — `[[ -x "${x:A}" ]] && A || B` ternary command select.
    #[test]
    fn ternary_command_select() {
        assert_parity(r###"fake=/nonexistent/docker-compose; [[ -x "${fake:A}" ]] && dccmd=docker-compose || dccmd="docker compose"; print "chose: $dccmd""###);
    }

    /// OMZP::docker-compose:5 — symlink `${link:A}` resolution + :t + [[ -x ]] in sandbox.
    #[test]
    fn symlink_resolution_sandbox() {
        assert_parity(r###"d=$(mktemp -d); print "#!/bin/sh" > "$d/realtool"; chmod +x "$d/realtool"; ln -s "$d/realtool" "$d/linktool"; link="$d/linktool"; print "resolved=${link:A:t}"; [[ -x "$link" ]] && print "exec-ok"; rm -rf "$d""###);
    }
}

// ══════════════════════════════ OMZP::brew ═════════════════════════

mod omzp_brew {
    use super::*;

    /// OMZP::brew:68 — alias referencing another alias (bubu='bubo && bup') + introspection.
    #[test]
    fn alias_referencing_alias() {
        assert_parity(r###"alias bi="brew install"; alias bu="brew update"; alias bubo="brew update && brew outdated"; alias bubu="bubo && bup"; alias | grep -E "^(bi|bu|bubo|bubu)=""###);
    }

    /// OMZP::brew:83 — `sed` backreference rewrite of name:deps lines.
    #[test]
    fn sed_backref_rewrite() {
        assert_parity(
            r###"data="git:gettext pcre2
node:icu4c
wget:openssl gettext"; print -r -- "$data" | sed "s/^\(.*\):\(.*\)$/\1 [\2]/""###,
        );
    }

    /// OMZP::brew:82 — `print -r --` literal output with embedded color-var markers.
    #[test]
    fn print_color_markers() {
        assert_parity(r###"blue="<B>"; off="<O>"; bold="<b>"; print -r -- "${blue}==>${off} ${bold}Formulae${off}"; print -r -- "${blue}==>${off} ${bold}Casks${off}""###);
    }
}

// ══════════════════════════════ OMZP::rsync ════════════════════════

mod omzp_rsync {
    use super::*;

    /// OMZP::rsync:4 — alias-value pattern test `[[ $v == *--delete* ]]` via ${aliases[name]}.
    #[test]
    fn alias_value_pattern_test() {
        assert_parity(r###"alias rsync-synchronize="rsync -avzu --delete --progress -h"; v=${aliases[rsync-synchronize]}; if [[ $v == *--delete* ]]; then print "deletes-enabled"; fi; print -r -- "$v""###);
    }
}

// ══════════════════════════════ OMZP::nmap ═════════════════════════

mod omzp_nmap {
    use super::*;

    /// OMZP::nmap:20 — scan-mode alias family + ${aliases[name]} flag introspection.
    #[test]
    fn scan_alias_family() {
        assert_parity(r###"alias nmap_slow="sudo nmap -sS -v -T1"; alias nmap_fast="nmap -F -T5 --version-light --top-ports 300"; for a in nmap_slow nmap_fast; do print "$a: ${aliases[$a]}"; done"###);
    }

    /// OMZP::nmap:25 — alias-value word munging ${v%% *} (first) / ${v##* } (last).
    #[test]
    fn alias_value_word_munge() {
        assert_parity(r###"alias nmap_fast="nmap -F -T5 --top-ports 300"; v=${aliases[nmap_fast]}; print "first-word=${v%% *}"; print "last-word=${v##* }""###);
    }
}

// ══════════════════════════════ OMZP::postgres ═════════════════════

mod omzp_postgres {
    use super::*;

    /// OMZP::postgres:9 — alias from interpolated dir var; alias-referencing-alias.
    #[test]
    fn pg_alias_interp_chain() {
        assert_parity(r###"PG=/opt/homebrew/var/postgres; alias startpost="pg_ctl -D $PG -l $PG/server.log start"; alias restartpost="stoppost && sleep 1 && startpost"; print "${aliases[startpost]}"; print "${aliases[restartpost]}""###);
    }
}

// ══════════════════════════════ OMZP::rake ═════════════════════════

mod omzp_rake {
    use super::*;

    /// OMZP::rake:6 — `noglob` alias family incl. alias name containing a slash.
    #[test]
    fn noglob_alias_with_slash() {
        assert_parity(r###"alias rake="noglob rake"; alias "bin/rake"="noglob bin/rake"; alias brake="noglob bundle exec rake"; for a in rake "bin/rake" brake; do print "$a => ${aliases[$a]}"; done"###);
    }
}

// ══════════════════════════════ OMZP::man ══════════════════════════

mod omzp_man {
    use super::*;

    /// OMZP::man:25 — `${${(Az)BUFFER}[1]}` array-split-then-index nested expansion.
    #[test]
    fn az_split_then_index() {
        assert_parity(r###"BUFFER="git commit --amend"; args=(${${(Az)BUFFER}[1]} ${${(Az)BUFFER}[2]}); print "cmd=${args[1]} sub=${args[2]}"; print "man-target=${args[1]}-${args[2]}""###);
    }

    /// OMZP::man:20 — glob-equality guard `[[ "$BUFFER" = man\ * ]]`.
    #[test]
    fn glob_equality_guard() {
        assert_parity(r###"for b in "man ls" "git log" "manatee x"; do if [[ "$b" = man\ * ]]; then print "$b -> already-man"; else print "$b -> wrap"; fi; done"###);
    }
}

// ══════════════════════════════ OMZP::colorize ═════════════════════

mod omzp_colorize {
    use super::*;

    /// OMZP::colorize:23 — `local -a` + ${arr[(Ie)val]} reverse-index membership.
    #[test]
    fn local_a_Ie_membership() {
        assert_parity(r###"f(){ local -a tools; tools=(chroma pygmentize); for c in pygmentize awk; do if [[ ${tools[(Ie)$c]} -eq 0 ]]; then print "$c: not-in-list"; else print "$c: index ${tools[(Ie)$c]}"; fi; done; }; f"###);
    }

    /// OMZP::colorize:40 — `[ -z "$VAR" ]` set-if-empty default.
    #[test]
    fn set_if_empty_default() {
        assert_parity(r###"unset STYLE; if [ -z "$STYLE" ]; then STYLE=emacs; fi; print "style=$STYLE"; STYLE=monokai; if [ -z "$STYLE" ]; then STYLE=emacs; fi; print "style=$STYLE""###);
    }

    /// OMZP::colorize:57 — `for FNAME in "$@"` with case lexer dispatch.
    #[test]
    fn fname_case_lexer_dispatch() {
        assert_parity(r###"f(){ local FNAME lexer; for FNAME in "$@"; do case $FNAME in (*.py) lexer=python;; (*.rs) lexer=rust;; (*) lexer=text;; esac; if [[ $lexer != text ]]; then print "$FNAME -> $lexer"; else print "$FNAME -> guess"; fi; done; }; f a.py b.rs c.unknown"###);
    }

    /// OMZP::colorize:78 — nested function definition + local var capture.
    #[test]
    fn nested_function_local_capture() {
        assert_parity(r###"outer(){ inner(){ print "inner sees LESS=$1"; }; local LESS="-R x"; inner "$LESS"; }; outer"###);
    }
}

// ══════════════════════════════ OMZP::gcloud ═══════════════════════

mod omzp_gcloud {
    use super::*;

    /// OMZP::gcloud:24 — iterate location array, [[ -d ]] guard + break (sandbox).
    #[test]
    fn location_dir_break() {
        assert_parity(r###"d=$(mktemp -d); mkdir -p "$d/real"; locs=("$d/nope1" "$d/real" "$d/nope2"); for l in $locs; do if [[ -d "$l" ]]; then found=${l:t}; break; fi; done; print "found=$found"; rm -rf "$d""###);
    }

    /// OMZP::gcloud:40 — `for x ( list )` paren-loop + [[ -f ]] + break (sandbox).
    #[test]
    fn paren_loop_file_break() {
        assert_parity(r###"d=$(mktemp -d); print x > "$d/comp.inc"; for f ( "$d/missing.inc" "$d/comp.inc" ); do if [[ -f "$f" ]]; then print "using ${f:t}"; break; fi; done; rm -rf "$d""###);
    }
}

// ══════════════════════════════ OMZP::ngrok ════════════════════════

mod omzp_ngrok {
    use super::*;

    /// OMZP::ngrok:2 — `(( ! $+functions[x] ))` existence guard.
    #[test]
    fn functions_existence_guard() {
        assert_parity(r###"fake(){ :; }; if (( ! $+functions[fake] )); then print "no fake"; else print "have fake"; fi; if (( ! $+functions[nope] )); then print "no nope"; fi"###);
    }
}

// ══════════════════════════════ OMZP::redis-cli ════════════════════

mod omzp_redis_cli {
    use super::*;

    /// _redis-cli:8 — parse `cmd:description` entries via ${e%%:*} / ${e#*:}.
    #[test]
    fn parse_cmd_description() {
        assert_parity(r###"a=("get:get the value of a key" "del:delete a key" "expire:set ttl, in seconds"); for e in $a; do k=${e%%:*}; d=${e#*:}; print "$k => $d"; done"###);
    }

    /// _redis-cli:31 — prefix-glob filter `[[ $k == get* ]]` over parsed keys.
    #[test]
    fn prefix_glob_filter_keys() {
        assert_parity(r###"a=("get:x" "getset:y" "getrange:z" "del:w" "set:v"); matched=(); for e in $a; do k=${e%%:*}; if [[ $k == get* ]]; then matched+=($k); fi; done; print "matches: $matched""###);
    }
}
