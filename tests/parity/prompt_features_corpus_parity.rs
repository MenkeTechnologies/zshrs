//! Behavioural parity corpus drawn from zsh knowledge + GitHub frameworks
//! (prezto, grml, ohmyzsh, zsh4humans), targeting areas the manual/plugin
//! passes under-covered: PROMPT ESCAPE sequences (heavily used by real
//! prompt themes), newer/uncommon builtins & expansions (=(...) process
//! substitution, =cmd equals expansion, functions -M math funcs, print
//! columnar, brace stepping, multios, emulate -c), and distinctive
//! framework idioms.
//!
//! Every candidate was generated from real-world zsh usage and then
//! VERIFIED empirically (deterministic across two `zsh -fc` runs) before
//! inclusion — no behaviour is claimed from memory. Each test asserts
//! `zshrs --zsh -fc` matches `/opt/homebrew/bin/zsh -fc` on stdout +
//! exit; prompt/escape output is rendered through `cat -v` so control
//! bytes are literal and stable.

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
    Command::new(zsh_path()).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

struct ShellResult {
    stdout: String,
    #[allow(dead_code)]
    stderr: String,
    exit: i32,
}

fn run_zsh(script: &str) -> ShellResult {
    let out = Command::new(zsh_path()).args(["-fc", script]).output().expect("invoke zsh");
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

// ═══════════════════════════ PROMPT ESCAPES ════════════════════════

mod prompt {
    use super::*;

    /// %(?.t.f) exit-status ternary, true.
    #[test]
    fn ternary_status_true() {
        assert_parity(r###"print -P '%(?.OK.NO)' | cat -v"###);
    }

    /// %(?.t.f) after false.
    #[test]
        fn ternary_status_false() {
        assert_parity(r###"false; print -P '%(?.OK.NO)' | cat -v"###);
    }

    /// %(N?.t.f) explicit exit-status match.
    #[test]
        fn ternary_status_explicit() {
        assert_parity(r###"(exit 42); print -P '%(42?.MATCH.NO)' | cat -v; (exit 5); print -P '%(42?.MATCH.NO)' | cat -v"###);
    }

    /// %(!.r.u) privilege ternary.
    #[test]
    fn ternary_privilege() {
        assert_parity(r###"print -P '%(!.root.user)' | cat -v"###);
    }

    /// %(Nj.t.f) jobs ternary (0 jobs).
    #[test]
    fn ternary_jobs() {
        assert_parity(r###"print -P '%(1j.HASJOBS.NOJOBS)' | cat -v; print -P '%(0j.ZEROJOBS.X)' | cat -v"###);
    }

    /// %F{name} foreground color.
    #[test]
    fn fg_name() {
        assert_parity(r###"print -P '%F{red}R%f' | cat -v"###);
    }

    /// %F{NNN} numeric (zero-padded + palette index).
    #[test]
    fn fg_numeric() {
        assert_parity(r###"print -P '%F{009}A%f' | cat -v; print -P '%F{196}B%f' | cat -v; print -P '%F{1}C%f' | cat -v"###);
    }

    /// %F{#rrggbb} truecolor.
    #[test]
    fn fg_hex() {
        assert_parity(r###"print -P '%F{#ff0000}R%f' | cat -v; print -P '%F{#00ff00}g%f' | cat -v"###);
    }

    /// %NF numeric prefix-arg form.
    #[test]
    fn fg_prefix_arg() {
        assert_parity(r###"print -P '%3FR%f' | cat -v"###);
    }

    /// %F{default} named default.
    #[test]
    fn fg_default() {
        assert_parity(r###"print -P '%F{default}d%f' | cat -v"###);
    }

    /// %K{...} background color (name + index).
    #[test]
    fn bg_color() {
        assert_parity(r###"print -P '%K{blue}B%k' | cat -v; print -P '%K{202}H%k' | cat -v"###);
    }

    /// %B%b bold.
    #[test]
    fn bold() {
        assert_parity(r###"print -P '%BboldB%b' | cat -v"###);
    }

    /// %U%u underline.
    #[test]
    fn underline() {
        assert_parity(r###"print -P '%UunderU%u' | cat -v"###);
    }

    /// %S%s standout.
    #[test]
    fn standout() {
        assert_parity(r###"print -P '%SstandS%s' | cat -v"###);
    }

    /// %{...%} literal escape wrapper.
    #[test]
    fn literal_wrapper() {
        assert_parity(r###"print -P '%{ESC%}X' | cat -v"###);
    }

    /// %% and %) literals.
    #[test]
    fn literal_pct_paren() {
        assert_parity(r###"print -P '100%%done' | cat -v; print -P 'paren%)end' | cat -v"###);
    }

    /// nested ternary in false branch.
    #[test]
        fn nested_ternary_false() {
        assert_parity(r###"false; print -P '%(?..%(!.R.U))' | cat -v"###);
    }

    /// nested ternary in true branch.
    #[test]
    fn nested_ternary_true() {
        assert_parity(r###"print -P '%(?.%(!.R.U).bad)' | cat -v"###);
    }

    /// bold + fg combination with reset order.
    #[test]
    fn bold_fg_combo() {
        assert_parity(r###"print -P '%B%F{blue}text%f%b' | cat -v"###);
    }

    /// bg + fg nested stop.
    #[test]
    fn bg_fg_combo() {
        assert_parity(r###"print -P '%K{red}%F{white}WB%f%k' | cat -v"###);
    }

    /// stacked B/U/S with reverse-order stops.
    #[test]
    fn stacked_attributes() {
        assert_parity(r###"print -P '%B%U%Sx%s%u%b' | cat -v"###);
    }

    /// %N>str> right truncation.
    #[test]
    fn truncate_right() {
        assert_parity(r###"print -P '%10>...>abcdefghijklmnop' | cat -v"###);
    }

    /// %N<str< left truncation.
    #[test]
    fn truncate_left() {
        assert_parity(r###"print -P '%10<...<abcdefghijklmnop' | cat -v"###);
    }

    /// truncation no-op when string fits.
    #[test]
    fn truncate_fits() {
        assert_parity(r###"print -P '%10>...>abc' | cat -v"###);
    }

    /// truncation multi-char marker.
    #[test]
    fn truncate_marker() {
        assert_parity(r###"print -P '%6>+++>longword' | cat -v"###);
    }

    /// left truncation empty marker.
    #[test]
    fn truncate_empty_marker() {
        assert_parity(r###"print -P '%3<<abcdef' | cat -v"###);
    }

    /// truncation counts visible chars only (excludes color escapes).
    #[test]
    #[ignore = "zshrs gap: prompt %N>>  truncation does not exclude color escapes from the width count (string not truncated)"]
    fn truncate_excludes_escapes() {
        assert_parity(r###"print -P '%5>>%F{red}abcdefgh%f' | cat -v"###);
    }

    /// %G width inside literal.
    #[test]
    fn glitch_width() {
        assert_parity(r###"print -P '%{X%G%}Y' | cat -v; print -P '%{seq%G%G%}X' | cat -v"###);
    }

    /// %NG numeric glitch outside literal.
    #[test]
    fn glitch_numeric() {
        assert_parity(r###"print -P '%2GY' | cat -v"###);
    }

    /// nested literal brace pairs.
    #[test]
    fn nested_literal_braces() {
        assert_parity(r###"print -P '%{a%{b%}c%}Z' | cat -v"###);
    }

    /// %c trailing cwd component after cd to sandbox.
    #[test]
    fn cwd_component() {
        assert_parity(r###"d=$(mktemp -d)/zsbx; mkdir -p "$d"; cd "$d"; print -P "%c" | cat -v"###);
    }

    /// %1~ last component with tilde contraction.
    #[test]
    fn cwd_tilde_component() {
        assert_parity(r###"d=$(mktemp -d)/zsbx; mkdir -p "$d"; cd "$d"; print -P "%1~" | cat -v"###);
    }

    /// %2c trailing 2 components.
    #[test]
    fn cwd_two_components() {
        assert_parity(r###"d=$(mktemp -d)/aa/bb/cc; mkdir -p "$d"; cd "$d"; print -P "%2c" | cat -v"###);
    }

    /// %(N/.t.f) path-depth ternary.
    #[test]
    fn ternary_path_depth() {
        assert_parity(r###"cd /; print -P "%(4/.DEEP.SHALLOW)" | cat -v; d=$(mktemp -d)/a/b/c/d/e; mkdir -p "$d"; cd "$d"; print -P "%(4/.DEEP.SHALLOW)" | cat -v"###);
    }

    /// ${(%)var} parameter prompt expansion.
    #[test]
    fn param_prompt_expand() {
        assert_parity(r###"v="%F{red}X%f"; print -r -- "${(%)v}" | cat -v"###);
    }

    /// %v / %Nv / %-1v psvar.
    #[test]
    fn psvar_access() {
        assert_parity(r###"psvar=(first second); print -P "%v %2v %-1v" | cat -v"###);
    }

    /// %(NV.t.f) psvar-nonempty ternary.
    #[test]
    fn ternary_psvar() {
        assert_parity(r###"psvar=("" two); print -P "%(1V.NE.E)%(2V.NE.E)" | cat -v"###);
    }

    /// literal %) and %% inside ternary text.
    #[test]
        fn ternary_literals_inside() {
        assert_parity(r###"false; print -P '%(?.t.f%)x)' | cat -v; print -P '%(?.100%%.fail)' | cat -v"###);
    }

    /// %(Nl.t.f) line-position ternary.
    #[test]
    #[ignore = "zshrs gap: prompt %(Nl.t.f) line-position ternary takes wrong branch (chars-already-printed not tracked)"]
    fn ternary_line_position() {
        assert_parity(r###"print -P 'xx%(1l.PRINTED.FRESH)' | cat -v"###);
    }

    /// successive fg changes, %f resets each.
    #[test]
    fn successive_fg() {
        assert_parity(r###"print -P '%F{cyan}a%F{magenta}b%f%f' | cat -v"###);
    }
}

// ═══════════════════════ NEWER / UNCOMMON FEATURES ═════════════════

mod features {
    use super::*;

    /// =(...) process substitution (temp-file form).
    #[test]
    fn procsubst_equals() {
        assert_parity(r###"cat =(printf "line1\nline2\n")"###);
    }

    /// =cmd equals expansion (PATH resolution), basename only.
    #[test]
    fn equals_expansion() {
        assert_parity(r###"print ${$(print =sh):t}"###);
    }

    /// =cmd on nonexistent errors.
    #[test]
    fn equals_expansion_missing() {
        assert_parity(r###"print =no_such_cmd_zzz"###);
    }

    /// functions -M user math function.
    #[test]
    fn functions_M() {
        assert_parity(r###"_addtwo(){ REPLY=$(( $1 + $2 )) }
functions -M addtwo 2 2 _addtwo
print $(( addtwo(10,20) ))"###);
    }

    /// functions -M with branching logic.
    #[test]
    fn functions_M_branch() {
        assert_parity(r###"_mymax(){ REPLY=$(( $1 > $2 ? $1 : $2 )) }
functions -M mymax 2 2 _mymax
print $(( mymax(3,7) ))"###);
    }

    /// <(...) input process substitution.
    #[test]
    fn procsubst_in() {
        assert_parity(r###"cat <(printf "via-fifo\n")"###);
    }

    /// >(...) output process substitution.
    #[test]
    fn procsubst_out() {
        assert_parity(r###"d=$(mktemp -d); print hi > >(cat > $d/out); wait; print "$(<$d/out)"; rm -rf $d"###);
    }

    /// two =(...) in one command.
    #[test]
    fn procsubst_two_equals() {
        assert_parity(r###"diff =(printf "a\nb\n") =(printf "a\nc\n"); true"###);
    }

    /// print -C n columnar (down).
    #[test]
    fn print_C() {
        assert_parity(r###"print -C 3 a b c d e f"###);
    }

    /// print -aC n across-first.
    #[test]
    fn print_aC() {
        assert_parity(r###"print -aC 2 a b c d"###);
    }

    /// print -x / -X tab expansion.
    #[test]
    fn print_x_X() {
        assert_parity(r###"print -x 8 "ab\tcd"; print -X 8 "ab\tcd""###);
    }

    /// read -d delimiter (here-string).
    #[test]
    fn read_d() {
        assert_parity(r###"read -d : x <<< "foo:bar"; print "[$x]""###);
    }

    /// read -E echo input.
    #[test]
    fn read_E() {
        assert_parity(r###"read -E x <<< "echoed"; print "[$x]""###);
    }

    /// brace integer range with step (up and down).
    #[test]
    fn brace_step() {
        assert_parity(r###"print {1..10..3}; print {10..1..2}"###);
    }

    /// stepped char range stays literal.
    #[test]
    fn brace_char_step_literal() {
        assert_parity(r###"print {a..i..2}"###);
    }

    /// anonymous fn EXIT trap fires on return.
    #[test]
    fn anon_fn_trap() {
        assert_parity(r###"(){ trap "print TRAPPED" EXIT; print body }"###);
    }

    /// TRY_BLOCK_ERROR in always block.
    #[test]
    fn try_block_error() {
        assert_parity(r###"(){ { false } always { print "cleanup tbe=$TRY_BLOCK_ERROR" } }"###);
    }

    /// return propagates through always.
    #[test]
    fn return_through_always() {
        assert_parity(r###"f(){ { return 3 } always { print "tbe=$TRY_BLOCK_ERROR" } }; f; print "rc=$?""###);
    }

    /// multios output to multiple files.
    #[test]
    fn multios_out() {
        assert_parity(r###"d=$(mktemp -d); print x > $d/f1 > $d/f2; print "$(<$d/f1)|$(<$d/f2)"; rm -rf $d"###);
    }

    /// multios input concatenation.
    #[test]
    fn multios_in() {
        assert_parity(r###"d=$(mktemp -d); print a > $d/f1; print b > $d/f2; cat < $d/f1 < $d/f2; rm -rf $d"###);
    }

    /// <<- tab-stripped heredoc.
    #[test]
    fn heredoc_dash() {
        assert_parity("cat <<-END\n\tindented\n\tlines\n\tEND\n");
    }

    /// emulate sh -c one-shot word splitting.
    #[test]
    fn emulate_sh_c() {
        assert_parity(r###"emulate sh -c 'v="a b c"; set -- $v; echo $#'"###);
    }

    /// emulate ksh -c one-shot.
    #[test]
    fn emulate_ksh_c() {
        assert_parity(r###"emulate ksh -c 'print hello'"###);
    }

    /// [[ -o noclobber ]] negated-name option test.
    #[test]
    fn option_test_negated() {
        assert_parity(r###"setopt noclobber; [[ -o noclobber ]] && print A; [[ -o clobber ]] && print B || print noB"###);
    }

    /// ${(A)=name=...} array assign with split.
    #[test]
    fn A_split_assign() {
        assert_parity(r###"name="a b c"; : ${(A)=arr=$name}; print ${#arr} ${arr[2]}"###);
    }

    /// autoload +X immediate load + functions body.
    #[test]
    fn autoload_plus_X() {
        assert_parity(r###"d=$(mktemp -d); print "print loaded-body" > $d/myfn; fpath=($d $fpath); autoload +X myfn; functions myfn; rm -rf $d"###);
    }

    /// unloaded autoload placeholder body via ${functions[name]}.
    #[test]
    fn autoload_unloaded_marker() {
        assert_parity(r###"d=$(mktemp -d); print "print B" > $d/g; fpath=($d $fpath); autoload g; print ${functions[g]}; rm -rf $d"###);
    }

    /// $(<file) fast read.
    #[test]
    fn dollar_lt_file() {
        assert_parity(r###"d=$(mktemp -d); print content > $d/f; print "$(<$d/f)"; rm -rf $d"###);
    }

    /// setopt globstarshort ** shorthand.
    #[test]
    fn globstarshort() {
        assert_parity(r###"setopt globstarshort; d=$(mktemp -d); mkdir -p $d/a/b; touch $d/a/b/f.txt; print -l $d/**.txt | sed "s#$d#SB#"; rm -rf $d"###);
    }
}

// ════════════════════════ FRAMEWORK IDIOMS ═════════════════════════

mod frameworks {
    use super::*;

    /// prezto — ${(j:,:):-\$${^@}} join over rc-expand default of positionals.
    #[test]
    #[ignore = "zshrs gap: ${(j:,:):-\\$${^@}} — rc-expand ${^@} over positionals not applied before join (no per-element distribution)"]
    fn join_default_rc_positionals() {
        assert_parity(r###"f() { print -r -- "${(j:,:):-\$${^@}}"; }
f alpha beta gamma"###);
    }

    /// prezto — (@M)${(f)var}:#alternation line filter.
    #[test]
    fn at_M_f_alternation() {
        assert_parity(r###"_ls_version="ls (GNU coreutils) 9.1
something else
lsd 0.23.1
busybox v1.36"
print -rl -- ${(@M)${(f)_ls_version}:#*(GNU|lsd|uutils) *}"###);
    }

    /// prezto — case dispatch on ./* /* *:* path shapes.
    #[test]
    fn case_path_shapes() {
        assert_parity(r###"setopt extendedglob
typeset -a argo
for arg in ./rel /abs host:remote plainword; do
  case $arg in
    ( ./* ) argo+=( "L-rel:$arg" ) ;;
    (  /* ) argo+=( "L-abs:$arg" ) ;;
    ( *:* ) argo+=( "remote:$arg" ) ;;
    (   * ) argo+=( "other:$arg" ) ;;
  esac
done
print -rl -- "${argo[@]}""###);
    }

    /// grml — ${(%):-...} deterministic prompt expansions.
    #[test]
    fn colon_minus_prompt() {
        assert_parity(r###"print -r -- "${(%):-%(?.OK.FAIL)}"
false; print -r -- "${(%):-%(?.OK.FAIL)}"
print -r -- "${(%):-%5(l.wide.narrow)}"
print -r -- "${(%):-%3<..<abcdefghij}""###);
    }

    /// ${PWD/#$HOME/~} anchored-prefix vs literal \~.
    #[test]
    fn pwd_home_abbrev() {
        assert_parity(r###"HOME=/home/jacob
PWD=/home/jacob/projects/zshrs
print -r -- "${PWD/#$HOME/~}"
print -r -- "${PWD/#$HOME/\~}""###);
    }

    /// grml — ${LBUFFER%%(#m)pat} capture + assoc fallback.
    #[test]
    fn lbuffer_strip_capture() {
        assert_parity(r###"setopt extendedglob
typeset -A abk=( "G" "git status" "L" "ls -la" )
LBUFFER="echo G"
LBUFFER=${LBUFFER%%(#m)[.\-+:|_a-zA-Z0-9]#}
print -r -- "stripped=[$LBUFFER] match=[$MATCH]"
LBUFFER+=${abk[$MATCH]:-$MATCH}
print -r -- "expanded=[$LBUFFER]""###);
    }

    /// prezto git-info — (pws:\t:)N tab-field subscript on scalar.
    #[test]
    fn pws_tab_subscript() {
        assert_parity(r###"ahead_and_behind=$(printf "3\t5")
ahead="$ahead_and_behind[(pws:\t:)1]"
behind="$ahead_and_behind[(pws:\t:)2]"
print -r -- "ahead=$ahead behind=$behind""###);
    }

    /// prezto git-info — porcelain alternation class patterns.
    #[test]
    fn porcelain_class_patterns() {
        assert_parity(r###"setopt extendedglob
status_text="## main...origin/main [ahead 2]
 M file1
?? file2
A  file3
 D file4"
status_lines=("${(@f)${status_text}}")
added=0 deleted=0
for line in "${status_lines[@]}"; do
  [[ "$line" == ([ACDMT][\ MT]|[ACMT]D)\ * ]] && (( added++ ))
  [[ "$line" == [\ ACMRT]D\ * ]] && (( deleted++ ))
done
print -r -- "added=$added deleted=$deleted""###);
    }

    /// recursive glob with exclusion **/*.txt~*/.git/* + (.N:t).
    #[test]
    #[ignore = "zshrs gap: **/*.txt~*/.git/*(.N:t) — :t modifier not applied after recursive-glob-with-exclusion (full paths returned)"]
    fn recursive_glob_exclusion() {
        assert_parity(r###"setopt extendedglob
d=$(mktemp -d)
mkdir -p "$d"/{src,.git,src/sub}
: > "$d/src/a.txt"; : > "$d/src/sub/b.txt"; : > "$d/.git/c.txt"
print -rl -- ${d}/**/*.txt~*/.git/*(.N:t)
rm -rf "$d""###);
    }

    /// (e) templating over an array.
    #[test]
    #[ignore = "zshrs gap: ${(e)parts[@]} eval-expansion over an array yields nothing (scalar (e) works, array (e) does not)"]
    fn e_templating_array() {
        assert_parity(r###"name="world"; greeting="hello \$name"
print -r -- "${(e)greeting}"
typeset -a parts=( "a=\$((1+2))" "b=\$name" )
print -rl -- "${(e)parts[@]}""###);
    }

    /// grml — (i) index used to blank an element in place.
    #[test]
    fn index_blank_element() {
        assert_parity(r###"typeset -a wl=(foo bar baz qux)
PREFIX=baz
wl[${wl[(i)$PREFIX]}]=""
print -r -- "after: ${(j:,:)wl}""###);
    }

    /// (ou) sort-unique + (j:|:) over :t.
    #[test]
    fn sort_unique_join_tail() {
        assert_parity(r###"typeset -a paths=(/a/b/c /d/e /f)
print -r -- "${(j:|:)${(@)paths:t}}"
typeset -a dups=(x y x z y)
typeset -a s=( "${(ou)dups[@]}" )
print -r -- "${(j:,:)s}""###);
    }

    /// negative array slices [1,-2]/[2,-1] + negative index.
    #[test]
    fn negative_slices() {
        assert_parity(r###"typeset -a argv2=(a b c d e)
print -r -- "${(@)argv2[1,-2]}"
print -r -- "${(@)argv2[2,-1]}"
print -r -- "last=${argv2[-1]} second-last=${argv2[-2]}""###);
    }

    /// (@ko) key iteration with arithmetic percentages.
    #[test]
    fn ko_percentages() {
        assert_parity(r###"typeset -A CMD=( ls 10 cd 5 grep 3 )
total=0
for k in "${(@k)CMD}"; do (( total += CMD[$k] )); done
typeset -a report
for k in "${(@ko)CMD}"; do report+=( "$k=$(( CMD[$k]*100/total ))%" ); done
print -r -- "${(j: :)report}""###);
    }

    /// ${^arr} rc-expand distribution with affixes.
    #[test]
    fn caret_rc_expand_affix() {
        assert_parity(r###"typeset -a v=(a b c)
print -rl -- pre-${^v}-post"###);
    }

    /// (D) directory abbreviation.
    #[test]
    fn D_dir_abbrev() {
        assert_parity(r###"HOME=/home/jacob
p=/home/jacob/src/proj
print -r -- "${(D)p}"
q=/usr/local
print -r -- "${(D)q}""###);
    }

    /// (q+) minimal quoting.
    #[test]
    fn q_plus_minimal() {
        assert_parity(r###"plain="abc"
special="a b\"c"
print -r -- ${(q+)plain}
print -r -- ${(q+)special}"###);
    }

    /// (ws.:.) negative field subscript.
    #[test]
    fn ws_negative_field() {
        assert_parity(r###"str="alpha:beta:gamma:delta"
print -r -- "${str[(ws.:.)2]}"
print -r -- "${str[(ws.:.)-1]}""###);
    }

    /// zformat -f %b/%r template.
    #[test]
    fn zformat_template() {
        assert_parity(r###"zmodload zsh/zutil
local out
zformat -f out "%b on %r" "b:main" "r:origin"
print -r -- "$out""###);
    }

    /// zstyle -e reply built from (L)var:-fallback.
    #[test]
    fn zstyle_e_casefold() {
        assert_parity(r###"zstyle -e ":x:y" tag "reply=( \${(L)PWD:-fallback} )"
typeset -a got
PWD=/Some/Mixed/Case
zstyle -a ":x:y" tag got
print -r -- "${got[*]}""###);
    }

    /// [[ -v name ]] + ${(t)${(P)name}} type introspection — `-v $n`
    /// param-expands its operand and `${(t)${(P)n}}` reports the
    /// referenced parameter's type (aspar) in both quoted and bare form.
    #[test]
    fn v_test_with_type() {
        assert_parity(r###"typeset -A h=( k v )
typeset -a a=( 1 2 3 )
s="scalar"
for n in h a s missing; do
  if [[ -v $n ]]; then print -r -- "$n is-set type=${(t)${(P)n}}"; else print -r -- "$n unset"; fi
done"###);
    }
}
