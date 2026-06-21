//! Behavioural parity corpus mined from the user's zpwr project
//! (`~/.zpwr`) — the most advanced single-author zsh codebase in
//! existence (172k+ LOC, 506+ subcommands). Idioms were harvested from
//! `autoload/common/*` (472 function files), `autoload/comp_utils/*`,
//! `env/*.zsh`, `scripts/*.zsh`, `scripts/lib.sh`, and `env/.p10k.zsh`.
//!
//! Each test replicates a DISTINCTIVE/ADVANCED zsh idiom actually used
//! in the source (cited `file` per test) as a deterministic mini-script,
//! then asserts `zshrs --zsh -fc` matches `/opt/homebrew/bin/zsh -fc`
//! byte-for-byte on stdout and exit code. Both shells run with `-f`
//! (no rc files). Every script was verified byte-identical across two
//! consecutive real-zsh runs before inclusion.
//!
//! This file deliberately targets the engine corners zpwr leans on
//! hardest — nested `${${…}…}`, `(P)/(t)/(tP)` indirection, `(e)`
//! eval-expansion, `(z)/(Az)` lexer splits, glob-qualifier combos,
//! special-hash subscript flags, set ops `:|`/`:*`, tied parameters,
//! named dirs, `(#b)/(#m)` backrefs — i.e. where parity gaps live.

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

// ════════════════════════ autoload/common ══════════════════════════

mod zpwr_common {
    use super::*;

    /// cv — modifier ordering: `${(Q)var:h}` applies :h before (Q) strips quotes.
    #[test]
    fn q_then_h_modifier_order() {
        assert_parity(r###"firstArg="\"/tmp/foo/bar.txt\""; print -r -- "${(Q)firstArg:h}""###);
    }

    /// cv — `(Q)` strip then chained `:h :t :e :r`.
    #[test]
    fn q_strip_then_chain() {
        assert_parity(r###"firstArg="\"/tmp/foo/bar.txt\""; v="${(Q)firstArg}"; print -r -- "${v:h} :: ${v:t} :: ${v:e} :: ${v:r}""###);
    }

    /// cv — `${(qqq@)args}` per-element single-quote quoting of array.
    #[test]
    fn qqq_at_per_element() {
        assert_parity(r###"args=( "a b" "c'd" ); printf "%s\n" "${(qqq@)args}""###);
    }

    /// cca — nested `${${(Az)@}[1]//\"/}` lexer-split $@, subscript, strip quotes.
    #[test]
    fn az_at_subscript_strip() {
        assert_parity(r###"set -- "\"/a/b/c.txt\"" two three; firstArg="${${(Az)@}[1]//\"/}"; print -r -- "$firstArg""###);
    }

    /// gcl — `${@: -1}` offset-substring last positional (space before -1 required).
    #[test]
    fn at_offset_last_positional() {
        assert_parity(r###"set -- a b https://github.com/u/repo.git; last_arg=${@: -1}; git_name=${last_arg##*/}; dir_name=${git_name%.*}; print -r -- "$last_arg | $git_name | $dir_name""###);
    }

    /// digs — `${var: -1}` last char and `${var:0:-1}` strip-last-char.
    #[test]
    fn neg_offset_and_strip_last() {
        assert_parity(r###"noproto="example.com."; print -r -- "last=[${noproto: -1}] strip=[${noproto:0:-1}]""###);
    }

    /// hd — `${cmd//$TOKEN/***}` GSUB redaction with a variable as the pattern.
    #[test]
    fn gsub_var_pattern_redact() {
        assert_parity(r###"GITHUB_TOKEN="abc123secret"; cmd="curl -H token:abc123secret https://api"; print -r -- "${cmd//$GITHUB_TOKEN/***}""###);
    }

    /// digs — `${(s.::.)var}` split on a multi-char separator.
    #[test]
    fn multichar_separator_split() {
        assert_parity(r###"p="a::b::c"; print -l -- "${(s.::.)p}""###);
    }

    /// digs — `${(@f)data}` split-on-newline preserving empty fields.
    #[test]
    fn at_f_preserve_empty() {
        assert_parity(r###"data=$'l1\nl2\n\nl4'; arr=( "${(@f)data}" ); print -r -- "n=${#arr}"; printf "<%s>\n" "${arr[@]}""###);
    }

    /// gcl — `${(l:5::0:)n}` left-pad with zeros.
    #[test]
    fn left_pad_zeros() {
        assert_parity(r###"for n in 1 22 333; do print -r -- "${(l:5::0:)n}"; done"###);
    }

    /// ge — base output `$(([##16]n))` hex/`$(([##2]n))` binary + zero-pad.
    #[test]
    fn base_output_zero_pad() {
        assert_parity(r###"n=255; print -r -- "${(l:4::0:)$(([##16]n))}"; print -r -- "$(([##2]10))""###);
    }

    /// ge — char-to-code `$(( ##A ))` literal, `$(( #c ))` var.
    #[test]
    fn char_code_literal_and_var() {
        assert_parity(r###"print -r -- $(( ##A )); c=z; print -r -- $(( #c ))"###);
    }

    /// cv — `${(t)var}` type flag: array / association / scalar.
    #[test]
    fn type_flag_three_kinds() {
        assert_parity(r###"typeset -a arr=(1 2); typeset -A h=(k v); s=str; print -r -- "${(t)arr} | ${(t)h} | ${(t)s}""###);
    }

    /// cv — `${(tP)name}` type of an indirectly-named parameter.
    #[test]
    fn type_of_indirect() {
        assert_parity(r###"typeset -a myarr=(a b); n=myarr; print -r -- "${(tP)n}""###);
    }

    /// hd — `(#b)` backref capture in GSUB using `${match[N]}` (key=val reorder).
    #[test]
    fn backref_gsub_reorder() {
        assert_parity(r###"setopt extendedglob; v="key=val"; print -r -- "${v//(#b)(*)=(*)/${match[2]}:${match[1]}}""###);
    }

    /// digs — `(#m)` whole-match `$MATCH` in GSUB (wrap digit runs).
    #[test]
    fn match_ref_wrap_digits() {
        assert_parity(r###"setopt extendedglob; v="abc123def"; print -r -- "${v//(#m)[0-9]##/<$MATCH>}""###);
    }

    /// ge — `${#h[(I)pat]}` count of assoc keys matching a glob.
    #[test]
    fn count_assoc_keys_glob() {
        assert_parity(r###"typeset -A h=(apple 1 apricot 2 banana 3); print -r -- "count=${#h[(I)a*]}""###);
    }

    /// digs — `${(M)v%%:*}` match-only keeps the removed `%%` portion.
    #[test]
    fn match_only_keep_removed() {
        assert_parity(r###"v="ERROR:bad:stuff"; print -r -- "${(M)v%%:*}""###);
    }

    /// gse — `${(us:,:)v}` split-on-comma + unique combined.
    #[test]
    fn split_unique_combined() {
        assert_parity(r###"v="x,y,x,z,y"; print -r -- "${(us:,:)v}""###);
    }

    /// gcl — nested `${${var:-default}:t}` default-then-modifier.
    #[test]
    fn default_then_modifier() {
        assert_parity(r###"unset foo; print -r -- "${${foo:-/usr/local/bin/zsh}:t}""###);
    }

    /// gse — `${(e)tmpl}` eval-expansion of embedded `$` references.
    #[test]
    fn eval_expansion_template() {
        assert_parity(r###"inner=WORLD; tmpl='hello $inner'; print -r -- "${(e)tmpl}""###);
    }

    /// cv — sort flag in quoted (join) vs unquoted (word) context.
    #[test]
    fn sort_quoted_vs_unquoted() {
        assert_parity(r###"a=(c a b); print -r -- "joined=[${(o)a}]"; print -r -- "split=${(o)a}""###);
    }

    /// gsdc — ZLE buffer arithmetic slice `${BUFFER[1,CURSOR]}` / `${BUFFER[CURSOR+1,-1]}`.
    #[test]
    fn buffer_arith_slice() {
        assert_parity(r###"BUFFER="git commit -m msg"; CURSOR=10; print -r -- "L=[${BUFFER[1,CURSOR]}] R=[${BUFFER[CURSOR+1,-1]}]""###);
    }

    /// ge — glob qualifiers `(.N:t)` plain-files-basename and `(/N:t)` dirs-basename.
    #[test]
    fn glob_plain_dir_basename() {
        assert_parity(r###"d=$(mktemp -d); : > $d/a.txt; : > $d/b.log; mkdir $d/sub; files=( $d/*(.N:t) ); dirs=( $d/*(/N:t) ); print -r -- "files=${(o)files}"; print -r -- "dirs=${(o)dirs}"; rm -rf $d"###);
    }

    /// zpwrTop — count functions matching case-insensitive glob `[(I)(#i)*pat*]`.
    #[test]
    fn count_functions_ci_glob() {
        assert_parity(r###"setopt extendedglob
foo(){ :; }; zpwrBar(){ :; }; ZPWRbaz(){ :; }; qux(){ :; }
print ${#functions[(I)(#i)*zpwr*]}"###);
    }

    /// zpwrTop — `${(@k)options[(R)on]}` reverse-value keys of special hash.
    #[test]
    fn options_reverse_value_keys() {
        assert_parity(r###"setopt extendedglob
local -a on=( "${(@k)options[(R)on]}" )
print $(( ${#on} > 0 ))"###);
    }

    /// zpwrFuncRank — `^_*` negation glob with `(N.:t)` qualifier.
    #[test]
    fn negation_glob_basename() {
        assert_parity(r###"setopt extendedglob
local d=$(mktemp -d)
: > $d/alpha; : > $d/_hidden; : > $d/beta
local -a f=( $d/*(N.:t) )
local -a g=( $d/^_*(N.:t) )
print "${(o)f}"
print "${(o)g}"
command rm -rf $d"###);
    }

    /// zpwrAcceptLine — nested `${(Az)${(s@,@)x}}` split-then-reparse.
    #[test]
    fn az_of_s_comma_split() {
        assert_parity(r###"local panes="1,2,3"
local -a out
local p
for p in ${(Az)${(s@,@)panes}}; do out+=( "pane:$p" ); done
print -l $out"###);
    }

    /// zpwrAcceptLine — `${(QL)x}` combined dequote + lowercase.
    #[test]
    fn combined_dequote_lower() {
        assert_parity(r###"local raw="'HELLO World'"
print "${(QL)raw}""###);
    }

    /// zpwrSnapshot — `parameters[(R)scalar-export]` type-subscript enumerates exported scalars.
    #[test]
    fn parameters_type_subscript_export() {
        assert_parity(r###"export MYEXPORTVAR=1
local plain=2
print $(( ${#parameters[(R)scalar-export]} >= 1 ))
print ${+parameters[MYEXPORTVAR]}"###);
    }

    /// zpwrBufferXtrace — `[[ =~ ]]` with `$match[N]` capture-group buffer rewrite.
    #[test]
    fn regex_capture_buffer_rewrite() {
        assert_parity(r###"local LBUFFER="echo a; set -x; echo b"
if [[ $LBUFFER =~ "^(.*)((set)[[:space:]]+(-x)[[:space:]]*;+[[:space:]]*)+(.*)$" ]]; then
  print "1=[$match[1]] 5=[$match[5]]"
fi"###);
    }

    /// zpwrSnapshot — `${functions[name]}` body capture distinguishes real fn from stub.
    #[test]
    fn functions_body_capture() {
        assert_parity(r###"realfn(){ print hello; print world; }
local body="${functions[realfn]}"
print "lines: $(print -r -- "$body" | wc -l | tr -d " ")"
[[ $body == *print* ]] && print "has-print""###);
    }

    /// zpwrAcceptLine — word-boundary command match `name([^[:alnum:]_]*|)`.
    #[test]
    fn word_boundary_command_match() {
        assert_parity(r###"setopt extendedglob
local cmd
for cmd in "touch" "touchtest" "touch foo"; do
  if [[ ${cmd} == touch([^[:alnum:]_]*|) ]]; then
    print "MATCH: $cmd"
  else
    print "no: $cmd"
  fi
done"###);
    }

    /// zpwrFlame — double-nested strip `${${entries[1]##* }}` / `${${entries[1]#* }%% *}`.
    #[test]
    fn double_nested_field_strip() {
        assert_parity(r###"local -a entries=( "1000 12.5 3.2 1 my_func" )
print "name: ${${entries[1]##* }}"
print "time: ${${entries[1]#* }%% *}""###);
    }

    /// zpwrTop — `${#${(M)${(k)assoc}:#pat}}` count keys matching glob, no loop.
    #[test]
    fn count_matching_keys_nested() {
        assert_parity(r###"local -A comps=( _git 1 _zpwr_foo 1 _ls 1 _zpwr_bar 1 )
print ${#${(M)${(k)comps}:#_zpwr*}}"###);
    }

    /// zpwrTop — numeric validation glob `<->(.<->)#`.
    #[test]
    fn numeric_validation_glob() {
        assert_parity(r###"setopt extendedglob
local -a vals=( "12.345" "abc" "99" "1.2.3" )
local v
for v in $vals; do
  [[ "$v" == <->(.<->)# ]] && print "num: $v" || print "skip: $v"
done"###);
    }

    /// zpwrTop — `10#$x` force-decimal (defeats leading-zero octal).
    #[test]
    fn force_decimal_base() {
        assert_parity(r###"local frac="007"
print $(( 10#$frac ))
local oct="010"
print $(( 10#$oct ))"###);
    }

    /// zpwrStale — glob qualifier types `(.)` plain `(/)` dir `(@)` symlink.
    #[test]
    fn glob_type_qualifiers() {
        assert_parity(r###"setopt extendedglob nullglob
local d=$(mktemp -d)
: > $d/file1; mkdir $d/dir1; ln -s file1 $d/link1
local -a plain=( $d/*(.:t) )
local -a dirs=( $d/*(/:t) )
local -a links=( $d/*(@:t) )
print "plain: ${(o)plain}"
print "dirs: ${(o)dirs}"
print "links: ${(o)links}"
command rm -rf $d"###);
    }

    /// zpwrChangeQuotes — ZLE buffer arithmetic slice `${BUFFER:2:-1}` / `${x[3,-2]}`.
    #[test]
    fn buffer_substr_negative() {
        assert_parity(r###"local BUFFER="\$(echo hi)"
print "[${BUFFER[1,2]}]"
print "[${BUFFER:2:-1}]"
local x="abcdefgh"
print "[${x[3,-2]}]""###);
    }

    /// zpwrFuncRank — dynamic associative increment `(( assoc[$key]++ ))` histogram.
    #[test]
    fn assoc_increment_histogram() {
        assert_parity(r###"local -A c
local w
for w in git git ls git ls vim; do (( c[$w]++ )); done
local k
for k in ${(ok)c}; do print "$k=$c[$k]"; done"###);
    }

    /// zpwrTrace — array slice `${a[2,4]}`, neg index, slice-then-join `${(j:,:)a[2,-1]}`.
    #[test]
    fn array_slice_join() {
        assert_parity(r###"local -a a=( one two three four five )
print "${a[2,4]}"
print "${a[-1]}"
print "${(j:,:)a[2,-1]}""###);
    }

    /// zpwrExecGlobParallel — `${(f)"$(<file)"}` read-file-and-split-lines.
    #[test]
    fn read_file_split_lines() {
        assert_parity(r###"local tmp=$(mktemp)
print -l "alpha" "beta" "gamma" > $tmp
local -a lines=( ${(f)"$(<$tmp)"} )
print "count=${#lines} second=$lines[2]"
command rm -f $tmp"###);
    }

    /// zpwrExecGlob — `${(e)command}` eval-expansion of stored template (`$f` substituted).
    #[test]
    fn eval_expand_stored_command() {
        assert_parity(r###"local f="report.txt"
local command="process \$f now"
print "${(e)command}""###);
    }
}

// ════════════════════ autoload/comp_utils ══════════════════════════

mod zpwr_comp {
    use super::*;

    /// _parameters — `${(M)${(f)x}:#pat}` keep only lines matching glob.
    #[test]
    fn keep_lines_matching() {
        assert_parity(r###"local data="apple
banana
apricot
cherry"
local -a hits=( ${(M)${(f)data}:#a*} )
print -l $hits"###);
    }

    /// _parameters — `${${(f)x}:#pat}` drop lines matching glob.
    #[test]
    fn drop_lines_matching() {
        assert_parity(r###"local data="keep1
DROP
keep2
DROP"
local -a kept=( ${${(f)data}:#DROP} )
print -l $kept"###);
    }

    /// _megacomplete — `${arr[(I)pat]}` index-of-last-match membership (0 if none).
    #[test]
    fn index_last_match_membership() {
        assert_parity(r###"local -a w=( ping curl git nmap )
print "I-curl: ${w[(I)curl]}"
print "I-none: ${w[(I)zzz]}"
print "membership-test: $(( ${w[(I)nmap]} > 0 ))""###);
    }

    /// _megacomplete — `${(u)arr}` unique preserving first-seen order.
    #[test]
    fn unique_first_seen() {
        assert_parity(r###"local -a a=( foo bar foo baz bar foo )
print -l ${(u)a}"###);
    }

    /// _files — `:gs/a/b/` global string-substitution modifier.
    #[test]
    fn gs_modifier() {
        assert_parity(r###"local s="a.b.c.d"
print "${s:gs/./-/}"
local fc="foo_bar_baz"
print "${fc:gs/_/ /}""###);
    }

    /// _files — `${tmp//\%p/repl}` percent-token templating across array elements.
    #[test]
    fn percent_token_template() {
        assert_parity(r###"local glob="*.log"
local -a tmp=( "%p:logfiles" "static:other" )
local -a out=( ${tmp//\%p/${glob}} )
print -l $out"###);
    }

    /// _parameters — `${(@M)arr:#pat}` matched-keep filter with @ preservation.
    #[test]
    fn at_M_matched_keep() {
        assert_parity(r###"local -a items=( apple_pie banana cherry_cake apricot )
local -a matched=( "${(@M)items:#a*}" )
print -l $matched"###);
    }

    /// _parameters — `${(P)name:0:N}` indirect deref then substring slice.
    #[test]
    fn indirect_then_substr() {
        assert_parity(r###"local LONGVAR="0123456789abcdef"
local name=LONGVAR
print "${(P)name:0:5}""###);
    }

    /// _parameters — `zparseopts -D -K -E -A` value-taking capture into assoc.
    #[test]
    fn zparseopts_assoc_capture() {
        assert_parity(r###"local -A opts
set -- -g "*.txt" -q
zparseopts -D -K -E -A opts g: q
print "g=${opts[-g]}"
print "q_set=${+opts[-q]}"
print "rest=$*""###);
    }

    /// _command_names — `zstyle` set + `-s` get + `-t` boolean-test roundtrip.
    #[test]
    fn zstyle_roundtrip() {
        assert_parity(r###"zstyle ":completion:*:foo" group-name "mygroup"
local val
zstyle -s ":completion:*:foo" group-name val
print "$val"
zstyle -t ":completion:*:foo" nonexistent && print "yes" || print "no""###);
    }
}

// ═══════════════════════════ env/*.zsh ═════════════════════════════

mod zpwr_env {
    use super::*;

    /// zpwr.zsh — `cmd=${v%%=*}` then split on custom sep `${(s%;%)cmd}`.
    #[test]
    fn strip_then_semicolon_split() {
        assert_parity(r###"v="forgit::add;forgit::warn=desc"; cmd=${v%%=*}; for exp in ${(s%;%)cmd}; do print -- "$exp"; done"###);
    }

    /// .zpwr_env.sh — tied scalar/array `typeset -T` with custom join char.
    #[test]
    fn tied_typeset_T() {
        assert_parity(r###"typeset -T FOO foo ":"; foo=(a b c); print -r -- "$FOO"; print -r -- "${foo[2]}""###);
    }

    /// .zpwr_env.sh — tied array append rebuilds joined scalar.
    #[test]
    fn tied_append_rebuild() {
        assert_parity(r###"typeset -T MYPATH mypath ":"; mypath=(/a /b /c); print -r -- "$MYPATH"; mypath+=(/d); print -r -- "$MYPATH""###);
    }

    /// zpwr.zsh — named directory `hash -d`, introspect `${nameddirs}` + `~name`.
    #[test]
    fn named_directory() {
        assert_parity(r###"hash -d zp=/tmp/zpwr/repo; print -r -- ${nameddirs[zp]}; print -r -- ~zp/scripts"###);
    }

    /// .zpwr_env.sh — array difference `${a:|b}` and intersection `${a:*b}`.
    #[test]
    fn array_diff_intersect() {
        assert_parity(r###"a=(rm mv cp ln chmod); b=(cp ln); d=(${a:|b}); i=(${a:*b}); print -r -- "diff=$d"; print -r -- "int=$i""###);
    }

    /// .zpwr_env.sh — empty-separator split into characters `${(s::)s}`.
    #[test]
    fn split_into_chars() {
        assert_parity(r###"s="abc"; for c in ${(s::)s}; do print -- "$c"; done"###);
    }

    /// .zpwr_env.sh — preserve-empty split `${(ps:\t:)data}`.
    #[test]
    fn ps_preserve_empty_tab() {
        assert_parity(r###"data=$'a\tb\tc'; arr=(${(ps:\t:)data}); print "n=$#arr"; print -- "${arr[2]}""###);
    }

    /// .zpwr_env.sh — unique array `typeset -aU` auto-dedup on append.
    #[test]
    fn typeset_aU_dedup_append() {
        assert_parity(r###"typeset -aU u=(rm mv rm cp mv); print -r -- "$u"; u+=(rm); print "after add n=$#u""###);
    }

    /// local/.tokens.sh — global aliases `alias -g` via `${galiases}`.
    #[test]
    fn global_aliases_hash() {
        assert_parity(r###"alias -g G="| grep" PIPE="2>&1"; for k in ${(ko)galiases}; do print -- "$k -> ${galiases[$k]}"; done"###);
    }

    /// local/.tokens.sh — suffix aliases `alias -s` via `${saliases}`.
    #[test]
    fn suffix_aliases_hash() {
        assert_parity(r###"alias -s txt=cat pdf=open; for k in ${(ko)saliases}; do print -- "$k=${saliases[$k]}"; done"###);
    }

    /// zpwr.zsh — separate sorted keys `${(ko)H}` and sorted-by-key values `${(vo)H}`.
    #[test]
    fn sorted_keys_and_values() {
        assert_parity(r###"typeset -gA H=(z 26 a 1 m 13); print -r -- ${(ko)H}; print -r -- ${(vo)H}"###);
    }

    /// .zpwr_re_env.sh — parameter-type introspection via `${parameters[name]}`.
    #[test]
    fn parameters_type_strings() {
        assert_parity(r###"typeset -gA AA=(x 1); typeset -ax LL=(a b); typeset -gix N=5; print "AA=${parameters[AA]}"; print "LL=${parameters[LL]}"; print "N=${parameters[N]}""###);
    }

    /// zpwr.zsh — sorted hash iteration with per-value custom-sep split.
    #[test]
    fn hash_iter_value_split() {
        assert_parity(r###"typeset -gA H; H[a]="f1;f2"; H[b]="g1"; for k in ${(ko)H}; do for e in ${(s%;%)H[$k]}; do print "$k:$e"; done; done"###);
    }

    /// .zpwr_env.sh — `(#b)` backref capture in `[[ == ]]` into `$match`.
    #[test]
    fn backref_in_test() {
        assert_parity(r###"setopt extendedglob; s="=37;45"; if [[ $s == (#b)=(*) ]]; then print "cap=$match[1]"; fi"###);
    }
}

// ══════════════════════════ scripts/lib.sh ═════════════════════════

mod zpwr_lib {
    use super::*;

    /// lib.sh — index-exact array membership `arr[(Ie)$x]`.
    #[test]
    fn index_exact_membership() {
        assert_parity(r###"typeset -a bl=(idna six certifi); pkg=six; if (( bl[(Ie)$pkg] )); then print "found at ${bl[(Ie)$pkg]}"; fi; pkg=nope; print "missing=${bl[(Ie)$pkg]}""###);
    }

    /// lib.sh — `${(tP)arg}` type of named param (array vs scalar).
    #[test]
    fn tP_array_vs_scalar() {
        assert_parity(r###"typeset -ax arr=(1 2); typeset -gx str=hi; n=arr; print -rl -- ${(tP)n}; n=str; print -rl -- ${(tP)n}"###);
    }

    /// lib.sh — indirect arithmetic via `${(P)name}` in `$(( ))`.
    #[test]
    fn indirect_in_arith() {
        assert_parity(r###"typeset -i n=5; name=n; print $(( ${(P)name} * 2 ))"###);
    }

    /// lib.sh — `${p:gs#a#X#}` perl-style replace with custom delimiter.
    #[test]
    fn gs_custom_delim() {
        assert_parity(r###"p="a/b/c/a"; print -r -- "${p:gs#a#X#}""###);
    }

    /// lib.sh — indirect array expansion `${(P)name}` and subscript of it.
    #[test]
    fn indirect_array_subscript() {
        assert_parity(r###"typeset -a colors=(red green blue); name=colors; print -r -- ${(P)name}; print -r -- ${${(P)name}[2]}"###);
    }

    /// lib.sh — keep-matched-portion `${(M)s#*=}` / `${(M)s%%[0-9]*}`.
    #[test]
    fn keep_matched_portion() {
        assert_parity(r###"s="foo=bar=baz"; print -r -- "${(M)s#*=}"; print -r -- "${(M)s%%[0-9]*}""###);
    }

    /// lib.sh — substitution forms `${s//pat/r}`, anchored `${s/#a/X}`, `${s/%c/Z}`.
    #[test]
    fn anchored_substitutions() {
        assert_parity(r###"s="a.b.c"; print -- "${s//./_}"; print -- "${s/#a/X}"; print -- "${s/%c/Z}""###);
    }

    /// lib.sh — newline-split `"${(f)data}"` with count and negative index.
    #[test]
    fn f_split_count_negindex() {
        assert_parity(r###"data=$(printf "l1\nl2\nl3"); lines=("${(f)data}"); print "count=$#lines last=${lines[-1]}""###);
    }
}

// ═════════════════════════ scripts/*.zsh ═══════════════════════════

mod zpwr_scripts {
    use super::*;

    /// zshRegenSearchableEnv.zsh — last-write-wins assoc + numeric-sorted keys `${(on)${(k)assoc}}`.
    #[test]
    fn lastwrite_assoc_sorted_keys() {
        assert_parity(r###"data=("param foo" "alias bar" "func baz")
typeset -A key_by_name
for line in $data; do
  type=${line%% *}; name=${line#* }
  key_by_name[$name]="$type $name"
done
sorted=(${(on)${(k)key_by_name}})
for n in $sorted; do print -r -- "${key_by_name[$n]}"; done"###);
    }

    /// zshRegenSearchableEnv.zsh — sort flags `(on)` `(oin)` `(On)`.
    #[test]
    fn sort_flag_variants() {
        assert_parity(r###"arr=(zebra apple Mango banana)
print -r -- ${(on)arr}
print -r -- ${(oin)arr}
print -r -- ${(On)arr}"###);
    }

    /// zshRegenSearchableEnv.zsh — `unsetopt multibyte` so `${#str}` counts bytes.
    #[test]
    fn multibyte_byte_count() {
        assert_parity(r###"unsetopt multibyte
b2="café"; print -r -- ${#b2}
setopt multibyte
print -r -- ${#b2}"###);
    }

    /// zshRegenSearchableEnv.zsh — `(qqqq)` ANSI-C `$'...'` quoting.
    #[test]
    fn qqqq_ansi_c_quote() {
        assert_parity(r###"name=$'a\tb'
print -rn -- "${(qqqq)name}"
print"###);
    }

    /// forDirZipRar.zsh — `$0` history modifiers `:h :t :r :e`.
    #[test]
    fn zero_history_modifiers() {
        assert_parity(r###"0="/some/path/to/script.zsh"
print -r -- ${0:h}
print -r -- ${0:t}
print -r -- ${0:r}
print -r -- ${0:e}"###);
    }

    /// forDirZipRar.zsh — chained `:t:r` and `:e` on multi-dot filename.
    #[test]
    fn chained_t_r_e() {
        assert_parity(r###"p="/foo/bar/baz.tar.gz"
print -r -- ${p:t}
print -r -- ${p:t:r}
print -r -- ${p:e}"###);
    }

    /// zshcompgen.zsh — exact-index membership `(( arr[(Ie)x] ))` blacklist.
    #[test]
    fn ie_blacklist_membership() {
        assert_parity(r###"BLACKLIST=(shutdown halt reboot)
for c in halt running poweroff; do
  if (( BLACKLIST[(Ie)$c] )); then print "$c blacklisted"; else print "$c ok"; fi
done"###);
    }

    /// zshcompgen.zsh — subscript flags `(i)` `(I)` index, `(r)` `(R)` value-by-pattern.
    #[test]
    fn subscript_i_I_r_R() {
        assert_parity(r###"a=(foo bar baz bar)
print -r -- ${a[(i)bar]}
print -r -- ${a[(I)bar]}
print -r -- ${a[(r)ba*]}
print -r -- ${a[(R)ba*]}"###);
    }

    /// forDirZipRar.zsh — assoc values only `${(v)hash}` then numeric sort.
    #[test]
    fn values_numeric_sort() {
        assert_parity(r###"typeset -A h=(a 1 b 2 c 3)
print -r -- ${(on)${(v)h}}"###);
    }

    /// zshRegenSearchableEnv.zsh — double-indirect `${(P)${(P)name}}`.
    #[test]
    fn double_indirect() {
        assert_parity(r###"foo=bar; bar=value
name=foo
print -r -- ${(P)name}
print -r -- ${(P)${(P)name}}"###);
    }

    /// forDirZipRar.zsh — `0=` resolution `${${0:#$ZSH_ARGZERO}:-...}` then `${(M)0:#/*}`.
    #[test]
    fn zero_resolution_chain() {
        assert_parity(r###"ZSH_ARGZERO=/bin/zsh
zero="./relative/script.zsh"
zero="${${zero:#$ZSH_ARGZERO}:-fallback}"
print -r -- "$zero"
zero="${${(M)zero:#/*}:-/abs/$zero}"
print -r -- "$zero""###);
    }

    /// .p10k.zsh — `(g::)` process backslash escapes in a string.
    #[test]
    fn g_process_escapes() {
        assert_parity(r###"icon='→'
print -r -- "${(g::)icon}""###);
    }

    /// zshRegenSearchableEnv.zsh — `[##16]` (no prefix) vs `[#16]` (with prefix).
    #[test]
    fn base_prefix_vs_noprefix() {
        assert_parity(r###"n=255
print -r -- $(( [##16] n ))
print -r -- $(( [##2] 10 ))
print -r -- $(( [#16] 255 ))"###);
    }

    /// allPanesSwap.zsh — `(ps:\n:)` split keeping empties on literal newline.
    #[test]
    fn ps_newline_keep_empty() {
        assert_parity(r###"data=$'x\ny\nz'
arr=(${(ps:\n:)data})
print -r -- "${#arr}""###);
    }

    /// forDirZipRar.zsh — glob qualifier combos `(.N:t)` `(/N:t)` `(L0N:t)` empty.
    #[test]
    fn glob_qualifier_combos() {
        assert_parity(r###"d=$(mktemp -d)
mkdir -p $d/sub
print foo > $d/a.txt; print bar > $d/b.txt
: > $d/empty.txt
cd $d
print -r -- ${(on)$(print -l *.txt(.N:t))}
print -r -- ${(on)$(print -l *(/N:t))}
print -r -- ${(on)$(print -l *.txt(L0N:t))}
cd /; rm -rf $d"###);
    }

    /// manzshcompgen.zsh — sort-by-size `(OLN:t)` largest-first basenames.
    #[test]
    fn sort_by_size_glob() {
        assert_parity(r###"d=$(mktemp -d)
print x > $d/one.log; print yy > $d/two.log
cd $d
print -rl -- ${(o)$(print -l *.log(OLN:t))}
cd /; rm -rf $d"###);
    }

    /// forDirZipRar.zsh — stack: delete element via `arr[$idx]=()`, push via `+=`.
    #[test]
    fn stack_delete_push() {
        assert_parity(r###"stack=(a b c)
idx=-1
print -r -- "${stack[$idx]}"
stack[$idx]=()
print -r -- "len=${#stack} -> $stack"
stack+=(x y)
print -r -- "$stack""###);
    }

    /// allPanesSwap.zsh — string char-slice `${buf[i,j]}` incl negative + arithmetic.
    #[test]
    fn string_char_slices() {
        assert_parity(r###"buf="hello world foo"
print -r -- "${buf[1,5]}"
print -r -- "${buf[-3,-1]}"
i=7
print -r -- "${buf[$((i)),$((i+4))]}""###);
    }

    /// allPanesSwap.zsh — dotted-key assoc namespace `info[$id.w]` + sorted keys.
    #[test]
    fn dotted_key_assoc() {
        assert_parity(r###"typeset -A info
for id in p1 p2; do
  info[$id.w]=80; info[$id.h]=24
done
print -r -- "${info[p1.w]} ${info[p2.h]}"
print -r -- ${(on)${(k)info}}"###);
    }

    /// zshcompgen.zsh — pattern-match count `${(M)#arr:#pat}` vs `${#a[(I)p]}`.
    #[test]
    fn match_count_two_forms() {
        assert_parity(r###"a=(apple apricot banana avocado)
print -r -- "${#a[(I)a*]}"
print -r -- "${(M)#a:#a*}""###);
    }

    /// zshRegenSearchableEnv.zsh — print-join `${(pj:\n:)lines}` (p enables `\n`).
    #[test]
    fn pj_newline_join() {
        assert_parity(r###"lines=(one two three)
body="${(pj:\n:)lines}"
print -r -- "$body" | wc -l | tr -d " ""###);
    }

    /// delete_dups.zsh — strip leading char via substring `${f:1}`.
    #[test]
    fn strip_leading_char() {
        assert_parity(r###"data=$'_git\n_docker\n_kubectl'
while read f; do
  cmd="${f:1}"
  print -r -- "$cmd"
done <<< "$data""###);
    }

    /// fzfEnvPreview.zsh — `read -r a b c <<< "$entry"` field destructuring.
    #[test]
    fn read_field_destructure() {
        assert_parity(r###"entry="param 128 64"
read -r type offset length <<< "$entry"
print -r -- "type=$type off=$offset len=$length""###);
    }
}

// ═════════════════════════ env/.p10k.zsh ═══════════════════════════

mod zpwr_p10k {
    use super::*;

    /// four-deep nested strip `${${${${X#\(}% }%\)}:-fallback}`.
    #[test]
    fn four_deep_strip() {
        assert_parity(r###"CONDA_PROMPT_MODIFIER="(base) "
CONDA_PREFIX="/opt/anaconda/envs/myenv"
print -r -- "${${${${CONDA_PROMPT_MODIFIER#\(}% }%\)}:-${CONDA_PREFIX:t}}"
CONDA_PROMPT_MODIFIER=""
print -r -- "${${${${CONDA_PROMPT_MODIFIER#\(}% }%\)}:-${CONDA_PREFIX:t}}""###);
    }

    /// substring `${X:0:24}` + conditional ellipsis `${${X:24}:+…}`.
    #[test]
    fn substr_conditional_ellipsis() {
        assert_parity(r###"P9K_CONTENT="this is a very long string over twenty four chars"
print -r -- "${P9K_CONTENT:0:24}${${P9K_CONTENT:24}:+…}"
short="brief"
print -r -- "${short:0:24}${${short:24}:+…}""###);
    }

    /// negated-match `:#` inside `:+` to append only when differing.
    #[test]
    fn negmatch_in_plus() {
        assert_parity(r###"P9K_CONTENT="3.11.0"; P9K_PYENV_PYTHON_VERSION="3.11.0"
print -r -- "${P9K_CONTENT}${${P9K_PYENV_PYTHON_VERSION:#$P9K_CONTENT}:+ $P9K_PYENV_PYTHON_VERSION}"
P9K_PYENV_PYTHON_VERSION="3.12.1"
print -r -- "${P9K_CONTENT}${${P9K_PYENV_PYTHON_VERSION:#$P9K_CONTENT}:+ $P9K_PYENV_PYTHON_VERSION}""###);
    }

    /// empty-name expansion `${${:-/$X}:#/default}` synthesize-then-filter.
    #[test]
    fn synthesize_then_filter() {
        assert_parity(r###"P9K_KUBECONTEXT_NAMESPACE=default
print -r -- "x${${:-/$P9K_KUBECONTEXT_NAMESPACE}:#/default}y"
P9K_KUBECONTEXT_NAMESPACE=prod
print -r -- "x${${:-/$P9K_KUBECONTEXT_NAMESPACE}:#/default}y""###);
    }

    /// `(V)` visible representation of control chars.
    #[test]
    fn visible_control_chars() {
        assert_parity(r###"x=$'a\tb'
print -r -- "${(V)x}""###);
    }

    /// char-to-code `$(( #\A ))` and `$(( #var ))`.
    #[test]
    fn char_code_backslash() {
        assert_parity(r###"print -r -- $(( #\A ))
c=z
print -r -- $(( #c ))"###);
    }

    /// percent-escape doubling `${branch//\%/%%}`.
    #[test]
    fn percent_double() {
        assert_parity(r###"branch="feat%50"
print -r -- "${branch//\%/%%}""###);
    }

    /// right-pad with fill char `${(r:6::-:)s}`.
    #[test]
    fn right_pad_fill() {
        assert_parity(r###"for s in a bb ccc; do print -r -- "${(r:6::-:)s}|"; done"###);
    }

    /// left-pad with Unicode box-drawing fill `${(l:5::─:)}`.
    #[test]
    fn left_pad_boxdraw() {
        assert_parity(r###"print -r -- "${(l:5::─:)}x""###);
    }

    /// modulo-match in array-from-arith `${${(M)$((i%6)):#3}:+!}`.
    #[test]
    fn modulo_match_marker() {
        assert_parity(r###"for i in 1 2 3 4 5 6; do
  printf "%s%s" "$i" "${${(M)$((i%6)):#3}:+!}"
done
print"###);
    }

    /// prompt-expansion in parameter `${(%):-%(?.yes.no)}`.
    #[test]
    fn prompt_ternary_in_param() {
        assert_parity(r###"print -r -- "${(%):-%(?.yes.no)}""###);
    }

    /// `unset -m 'PAT*'` pattern-based mass unset.
    #[test]
    fn unset_m_pattern() {
        assert_parity(r###"typeset -g FOO_A=1 FOO_B=2 BAR=3
unset -m "FOO_*"
print -r -- "${+FOO_A} ${+FOO_B} ${+BAR}""###);
    }

    /// `print -P` prompt-escape expansion `%F{}%B%b%f`.
    #[test]
    fn print_P_prompt_escapes() {
        assert_parity(r###"print -P "%F{red}%Bbold%b%f plain" | cat -v"###);
    }
}
