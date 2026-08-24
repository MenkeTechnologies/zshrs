//! Nine-way emulation parity harness (`PARITY_CASES`): each "way" runs a
//! zshrs emulation mode against its *correct* reference and requires
//! byte-identical stdout + exit-code sign. Seven ways are real-shell-faithful
//! — `zshrs --X` vs the real shell X: `zsh`, `bash`, `ksh`, `/bin/sh`,
//! `/bin/dash` required, plus `mksh` and `ash` best-effort. Two ways are
//! zsh-STYLE cross-emulation legs — `zshrs --sh --zsh` / `--ksh --zsh` (which
//! deliberately keep zsh semantics) vs real zsh doing `emulate sh` /
//! `emulate ksh`, because the correct reference for "zsh's approximation of
//! sh" is zsh itself, not `/bin/sh`.
//!
//! This is the curated-corpus differential the way `parity-fuzz.rs` is for
//! `--zsh` at scale: a hand-picked set of *portable* scripts that MUST be
//! byte-identical (stdout + exit-code sign) between zshrs-in-mode-X and the
//! real shell X. The corpus deliberately avoids constructs whose behavior
//! legitimately differs across these shells — unquoted word-splitting,
//! `echo` escape handling, arrays, `[[ ]]` — because a differential on
//! those would flag intentional language differences as noise. Mode-specific
//! rejections (e.g. dash's) are pinned in `tests/dash_mode.rs`.
//!
//! Missing reference shells are reported (never silently passed). Set
//! `ZSHRS_REQUIRE_REF_SHELLS=1` (CI does) to turn a missing shell into a
//! failure instead of a skip, so the parity contract is enforced rather
//! than aspirational.

use std::path::Path;
use std::process::Command;

fn zshrs_bin() -> String {
    env!("CARGO_BIN_EXE_zshrs").to_string()
}

/// A reference shell and the zshrs flag that emulates it.
/// One parity "way": a zshrs invocation paired with the reference invocation
/// it must match. Most cases are `zshrs --X` vs the real shell X. Two are
/// CROSS-EMULATION cases — zshrs's zsh-STYLE emulation of a POSIX shell
/// (`--sh --zsh` / `--ksh --zsh`, which deliberately keeps zsh semantics)
/// vs the real zsh doing the same `emulate` — because the correct reference
/// for "zsh's approximation of sh" is zsh itself, not /bin/sh.
struct ParityCase {
    /// Human label / the way's name.
    name: &'static str,
    /// zshrs emulation flags (e.g. `["--sh", "--zsh"]`).
    zshrs_flags: &'static [&'static str],
    /// Candidate paths / PATH names for the reference binary, first wins.
    candidates: &'static [&'static str],
    /// When set, prepend `emulate <this>\n` to the reference script so the
    /// reference (zsh) runs in the matching emulation — the cross-emulation
    /// legs. `None` runs the reference shell natively.
    ref_emulate: Option<&'static str>,
    /// Run the EXTENDED_CORPUS too (arrays / `[[` / `(( ))` / braces).
    extended: bool,
    /// Best-effort case: absence of its reference is a skip, never fatal even
    /// under ZSHRS_REQUIRE_REF_SHELLS. The core ways are required.
    optional: bool,
}

const ZSH: &[&str] = &["zsh", "/bin/zsh", "/usr/bin/zsh", "/opt/homebrew/bin/zsh"];

const PARITY_CASES: &[ParityCase] = &[
    // ── zshrs --X vs the real shell X (real-shell-faithful) ──────────────
    ParityCase {
        name: "zsh",
        zshrs_flags: &["--zsh"],
        candidates: ZSH,
        ref_emulate: None,
        extended: true,
        optional: false,
    },
    ParityCase {
        name: "bash",
        zshrs_flags: &["--bash"],
        candidates: &[
            "bash",
            "/bin/bash",
            "/usr/bin/bash",
            "/opt/homebrew/bin/bash",
        ],
        ref_emulate: None,
        extended: true,
        optional: false,
    },
    ParityCase {
        name: "ksh",
        zshrs_flags: &["--ksh"],
        candidates: &["ksh", "/bin/ksh", "/usr/bin/ksh"],
        ref_emulate: None,
        extended: true,
        optional: false,
    },
    ParityCase {
        name: "sh",
        zshrs_flags: &["--sh"],
        candidates: &["/bin/sh"],
        ref_emulate: None,
        extended: false,
        optional: false,
    },
    ParityCase {
        name: "dash",
        zshrs_flags: &["--dash"],
        candidates: &["/bin/dash", "/usr/bin/dash"],
        ref_emulate: None,
        extended: false,
        optional: false,
    },
    // ── zshrs --X --zsh (zsh-STYLE) vs real zsh doing `emulate X` ────────
    ParityCase {
        name: "sh/zsh-style",
        zshrs_flags: &["--sh", "--zsh"],
        candidates: ZSH,
        ref_emulate: Some("sh"),
        extended: false,
        optional: false,
    },
    ParityCase {
        name: "ksh/zsh-style",
        zshrs_flags: &["--ksh", "--zsh"],
        candidates: ZSH,
        ref_emulate: Some("ksh"),
        extended: true,
        optional: false,
    },
    // ── best-effort variants: ash ≈ dash, mksh ≈ ksh (POSIX base only) ───
    ParityCase {
        name: "mksh",
        zshrs_flags: &["--mksh"],
        candidates: &["mksh", "/bin/mksh", "/usr/bin/mksh"],
        ref_emulate: None,
        extended: false,
        optional: true,
    },
    ParityCase {
        name: "pdksh",
        zshrs_flags: &["--pdksh"],
        candidates: &["pdksh", "/bin/pdksh", "/usr/bin/pdksh"],
        ref_emulate: None,
        extended: false,
        optional: true,
    },
    ParityCase {
        name: "ash",
        zshrs_flags: &["--ash"],
        candidates: &["ash", "/bin/ash", "/usr/bin/ash"],
        ref_emulate: None,
        extended: false,
        optional: true,
    },
];

/// Portable scripts that every one of {zsh, ksh, sh, dash} executes
/// identically. Only `printf` is used for output (no `echo` escape
/// divergence); all expansions are quoted (no word-split divergence); no
/// arrays / `[[`. Each entry is `(script, why)` — the `why` documents the
/// POSIX feature exercised so a future edit knows what it protects.
const PORTABLE_CORPUS: &[&str] = &[
    "printf '%s\\n' hello",                                        // literal
    "x=5; printf '%s\\n' \"$x\"",                                  // scalar assign + expand
    "for i in 1 2 3; do printf '%s' \"$i\"; done; printf '\\n'",   // for loop
    "i=0; while [ \"$i\" -lt 3 ]; do i=$((i+1)); done; printf '%s\\n' \"$i\"", // while + arith
    "case foo in f*) printf match;; esac; printf '\\n'",           // case glob
    "printf '%s\\n' \"${undef:-def}\"",                            // default-value expansion
    "f() { printf '%s\\n' \"$1\"; }; f hi",                        // function + positional
    "printf '%d\\n' \"$((6/2+1))\"",                               // arithmetic
    "printf '%s\\n' \"$(printf sub)\"",                            // command substitution
    "set -- a b c; printf '%s\\n' \"$#\"",                         // positional count
    "v=abc; printf '%s\\n' \"${v#a}\"",                            // prefix strip
    "v=abc; printf '%s\\n' \"${v%c}\"",                            // suffix strip
    "v=aXbXc; printf '%s\\n' \"${v%X*}\"",                         // greedy-vs-lazy suffix
    "true && printf yes; printf '\\n'",                            // && short-circuit
    "false || printf recover; printf '\\n'",                       // || short-circuit
    "printf '%s\\n' \"${#abc}\" 2>/dev/null || printf '%s\\n' 0",  // length (abc undefined → 0)
    "x=1; y=2; printf '%s\\n' \"$((x<y))\"",                       // arith comparison
    "if [ a = a ]; then printf eq; fi; printf '\\n'",              // test builtin
    "n=5; printf '%s\\n' \"$((n*n))\"",                            // arith mult
    "printf '%s ' one two three; printf '\\n'",                    // printf reuse
    // Field splitting on a non-whitespace IFS — the trailing-empty-field
    // rule where zsh diverges from the POSIX shells. In a bare drop-in
    // mode these MUST match the real shell (posix-faithful): a trailing
    // separator drops the empty field, a leading/middle one keeps it.
    "IFS=:; v=a:b:; set -- $v; printf '%s\\n' \"$#\"",             // trailing → drop empty
    "IFS=:; v=:a:b; set -- $v; printf '%s\\n' \"$#\"",             // leading → keep empty
    "IFS=:; v=a::b; set -- $v; printf '%s\\n' \"$#\"",             // middle → keep empty
    "IFS=:; v=:; set -- $v; printf '%s\\n' \"$#\"",                // lone separator
    "IFS=:; v=:a::b:; set -- $v; printf '%s\\n' \"$#\"",           // combined
    "IFS=:; v=a:b:c; set -- $v; printf '%s\\n' \"$#\"",            // no trailing
    // `read` (no -r): a backslash-escaped IFS char is literal — one field.
    // Identical in dash/ksh/sh AND zsh, so it belongs in the shared corpus.
    "printf 'a\\\\ b\\n' | { read x y; printf '[%s][%s]\\n' \"$x\" \"$y\"; }",
    "printf 'a\\\\ b\\n' | { read x; printf '[%s]\\n' \"$x\"; }",
    // Harder POSIX torture (all 7 ways agree).
    "printf '%05d\\n' 5",                                          // zero-pad width
    "printf '%d\\n' \"$((5 - - 3))\"",                             // unary-minus chain → 8
    "printf '%d\\n' \"$((7 & 3 | 4))\"",                           // bitwise precedence → 7
    "printf '%d\\n' \"$((1 ? 2 ? 3 : 4 : 5))\"",                   // nested ternary → 3
    "v=aaa/bbb; printf '%s\\n' \"${v#*/}\"",                       // shortest prefix strip
    "v=x.tar.gz; printf '%s\\n' \"${v%.gz}\"",                     // suffix strip
    "unset x; printf '%s\\n' \"${x:-${y:-deep}}\"",               // nested default
    "printf '%s\\n' \"$(echo \"$(echo deep)\")\"",               // nested command sub
    "case abc in a|b|abc) printf alt;; esac; printf '\\n'",        // case alternation
    "[ 5 -eq 05 ] && printf eq; printf '\\n'",                     // leading-zero numeric test
    "set -- -a -b -c; while getopts abc o; do printf '%s' \"$o\"; done; printf '\\n'", // getopts
    "set -- 1 2 3 4 5; shift 3; printf '%s\\n' \"$*\"",            // shift N
    "printf '%b\\n' 'a\\tb'",                                      // %b escape interpretation
    // Expansion batch — every one verified byte-identical across
    // zsh/bash/ksh/sh/dash (scratchpad/harness_classify.sh); guards a
    // regression in any single mode against ALL real shells at once.
    "x=5; y=3; printf '%s\\n' \"$((x*y+x-y))\"",                   // arith mixed ops → 17
    "printf '%s\\n' \"$((17/4))\" \"$((17%4))\"",                  // int div + mod
    "printf '%s\\n' \"$((1<<4))\" \"$((256>>2))\"",               // shifts
    "printf '%s\\n' \"$((5&3))\" \"$((5|2))\" \"$((5^1))\"",       // bitwise and/or/xor
    "printf '%s\\n' \"$((3>2))\" \"$((2>3))\" \"$((2==2))\"",      // arith comparisons
    "x=10; printf '%s\\n' \"$((x>5?x:0))\"",                       // arith ternary
    "printf '%s\\n' \"$(( (2+3) * 4 ))\"",                         // parenthesized arith
    "printf '%s\\n' \"$((0x1f))\"",                                // hex literal → 31
    "v=a.b.c; printf '%s\\n' \"${v%%.*}\" \"${v##*.}\"",           // greedy strip both ends
    "v=a.b.c; printf '%s\\n' \"${v%.*}\" \"${v#*.}\"",             // lazy strip both ends
    "v=aXbXc; printf '%s\\n' \"${v%X*}\"",                         // suffix strip with glob
    "x=; printf '%s\\n' \"${x:-def}\" \"${x-def}\"",               // :- vs - on empty
    "x=set; printf '%s\\n' \"${x:+yes}\"",                         // :+ alt on set
    "unset x; printf '%s\\n' \"${x:=assigned}\" \"$x\"",           // := assign-default
    "printf '%s\\n' \"${x:-$(printf sub)}\"",                      // default = cmd-sub
    "set -- a b c; printf '%s\\n' \"$#\" \"$1\" \"$3\"",           // positional count/index
    "set -- a b c d e; shift 2; printf '%s\\n' \"$1\" \"$#\"",     // shift then count
    "f() { return 7; }; f; printf '%s\\n' \"$?\"",                 // function return status
    "cmd() { echo \"$@\"; }; cmd a b c",                          // \"$@\" forwarding
    "i=0; until [ $i -ge 3 ]; do i=$((i+1)); done; printf '%s\\n' \"$i\"", // until loop
    "s=0; for x in 1 2 3 4; do s=$((s+x)); done; printf '%s\\n' \"$s\"",   // for-in accumulate
    "case xyz in a*|x*) printf hit;; esac; printf '\\n'",          // case alternation glob
    "case foo in ???) printf three;; esac; printf '\\n'",          // case fixed-length glob
    "x=$(printf 'a b c'); printf '%s\\n' \"$x\"",                  // cmd-sub with spaces (quoted)
    "printf '%03d\\n' 7",                                          // width+zero pad
    "printf '%x %o\\n' 255 8",                                     // hex + octal output
    "v=hello; printf '%s\\n' \"${v}world\"",                       // braced-var concat
    "readonly r=5; printf '%s\\n' \"$r\"",                         // readonly then read
    "if [ 3 -gt 2 ] && [ 1 -lt 2 ]; then printf both; fi; printf '\\n'", // chained test
    // Second expansion batch (harness_classify2.sh — all ref shells agree).
    "printf '%s\\n' \"${#@}\"",                                    // count of \"$@\"
    "x=abc; printf '%s\\n' \"${x#?}\" \"${x%?}\"",                 // single-char strip both
    "v=aaabbb; printf '%s\\n' \"${v##a}\"",                        // greedy prefix (only first a)
    "printf '%s\\n' \"$(( -5 ))\" \"$(( - 5 ))\"",                 // unary minus, spaced
    "printf '%s\\n' \"$(( !0 ))\" \"$(( !5 ))\"",                  // logical not
    "printf '%s\\n' \"$(( ~0 ))\"",                                // bitwise not → -1
    "x=3; printf '%s\\n' \"$(( x += 2 ))\" \"$x\"",                // arith += side effect
    "printf '%d\\n' \"'A\"",                                       // char → code point 65
    "printf '%s\\n' \"$(( 1 && 0 ))\" \"$(( 1 || 0 ))\"",          // logical and/or
    "printf '%s\\n' \"$(( 3 == 3 && 4 != 5 ))\"",                  // compound comparison
    "a=5; printf '%s\\n' \"$(( a > 0 ? (a > 3 ? 2 : 1) : 0 ))\"",  // nested ternary
    "printf '%s\\n' \"$(( 100 / 3 * 3 ))\"",                       // left-assoc div/mul → 99
    // `+alt` on a variable the script OWNS. This read `${TERM+set}`, whose
    // answer is the reference's identity rather than the operator: bash
    // defaults TERM to `dumb` when the environment carries none, while zsh and
    // dash leave it unset. `/bin/sh` is bash on macOS and dash on Linux, so the
    // `sh` leg could not agree with both — it passed on the ubuntu runner and
    // failed on the macOS one. Both branches of the operator are covered now.
    "unset u; s=1; printf '%s|%s\\n' \"${u+set}\" \"${s+set}\"",         // +alt, unset and set
    "a=hello; b=$a; a=world; printf '%s\\n' \"$b\"",               // value copy, not alias
    "v=$(printf 'x\\ny\\n'); printf '[%s]\\n' \"$v\"",             // multiline cmd-sub trims trailing NL
    "x=5; { x=10; }; printf '%s\\n' \"$x\"",                       // brace group shares scope
    "x=5; (x=10); printf '%s\\n' \"$x\"",                          // subshell isolates
    "f() { g() { printf inner; }; g; }; f; printf '\\n'",          // nested function
    "v=abc; case $v in *b*) printf mid;; esac; printf '\\n'",      // case substring glob
    "n=0; for x in; do n=$((n+1)); done; printf '%s\\n' \"$n\"",   // empty for list
    "printf '%s\\n' \"${x-}${y-}\"",                               // unset - default (empty)
    "printf '%.2f\\n' 3",                                          // float format of int
    "printf '%5.2f\\n' 3.14159",                                   // width.precision float
    "printf '%-5s|\\n' hi",                                        // left-justified string
    "printf '%+d\\n' 5",                                           // forced-sign int
    // Third expansion batch (harness_classify3.sh — all ref shells agree).
    "printf '%s\\n' \"${x:=a}${x:=b}\"",                           // repeated := (2nd no-op) → ab? aa
    "x=abcdef; printf '%s\\n' \"${x#a?c}\"",                       // glob '?' inside prefix strip
    "v=aXbXc; printf '%s\\n' \"${v#*X}\" \"${v##*X}\"",            // shortest vs longest glob strip
    "printf '%s\\n' \"$(( ${x:-4} + 1 ))\"",                       // default expansion inside arith
    "x=5; printf '%s\\n' \"$(( x ? x : -1 ))\"",                   // bare-var arith ternary
    "printf '%s\\n' \"$(( 07 + 1 ))\"",                            // octal literal in arith → 8
    "set -- 1 2 3; printf '%s\\n' \"$@\"",                         // \"$@\" separate words
    "set -- 1 2 3; printf '[%s]\\n' \"$*\"",                       // \"$*\" IFS-joined
    "set --; printf '%s\\n' \"$#\"",                               // zero positionals
    "x=$(exit 3); printf '%s\\n' \"$?\"",                          // cmd-sub propagates exit
    "true; false; printf '%s\\n' \"$?\"",                          // status of last command
    "x=5; unset x; printf '%s\\n' \"${x:-gone}\"",                 // unset then default
    "printf '%s\\n' \"${#}\"",                                     // positional count via ${#}
    "set -- aa bbb c; printf '%s\\n' \"${#1}\" \"${#2}\"",         // length of positionals
    "v=hello world; printf '%s\\n' \"${v% *}\"",                   // strip last word
    "v=a; v=${v}${v}${v}; printf '%s\\n' \"$v\"",                  // self-concatenation
    "printf '%s\\n' \"$(( 5 % 3 % 2 ))\"",                         // left-assoc modulo → 0
    "printf '%s\\n' \"$(( (1+2)*(3+4) ))\"",                       // grouped arith → 21
    "x=3; case $x in 1) ;; 2) ;; *) printf other;; esac; printf '\\n'", // empty arms + default
    "r=$(printf line); printf '[%s]\\n' \"$r\"",                   // cmd-sub no trailing NL
    // Fourth expansion batch (harness_classify4.sh — deep nesting / dynamic).
    "v=abc; printf '%s\\n' \"${v#${v}}\"",                         // strip self → empty
    "x=b; v=abc; printf '%s\\n' \"${v#*$x}\"",                     // dynamic glob strip pattern
    "p=ab; v=abcd; printf '%s\\n' \"${v#$p}\"",                    // variable-valued prefix strip
    "printf '%s\\n' \"${x:-${y:-${z:-deep}}}\"",                   // triple-nested default
    "x=1; printf '%s\\n' \"$(( x + $(printf 2) ))\"",             // cmd-sub inside arithmetic
    "printf '%s\\n' \"$(printf '%s' \"$(printf '%s' innermost)\")\"", // nested command sub
    "v=a1b2c3; printf '%s\\n' \"${v#[a-z][0-9]}\"",                // bracket-class prefix strip
    "a=3; b=4; printf '%s\\n' \"$(( a*a + b*b ))\"",               // arith with repeated vars → 25
    "printf '%s\\n' \"$(( 1 + 2 * 3 - 4 / 2 ))\"",                 // full precedence → 5
    "f() { echo \"$1\"; }; f \"$(printf 'with space')\"",         // cmd-sub arg keeps spaces
    "set -- a b c; for x; do printf '%s' \"$x\"; done; printf '\\n'", // for over \"$@\" implicitly
    "set -- a b c; n=$#; while [ $# -gt 0 ]; do shift; done; printf '%s\\n' \"$n\"", // shift-drain
    "a=''; b=x; printf '%s\\n' \"${a:-$b}\"",                      // default sourced from another var
    "printf '%s\\n' \"$(( 10 - 2 - 3 ))\"",                        // left-assoc subtraction → 5
    "x=$((3)); y=$((x*x)); printf '%s\\n' \"$y\"",                 // chained arithmetic assigns
    "printf '%s\\n' \"$(false || echo recovered)\"",              // cmd-sub with || recovery
    "v=a:b:c; while [ -n \"$v\" ]; do printf '%s ' \"${v%%:*}\"; case $v in *:*) v=\"${v#*:}\";; *) v='';; esac; done; printf '\\n'", // split-loop idiom
    // Fifth expansion batch (harness_classify5.sh) — heredocs, redirection,
    // pipelines, printf specifiers, arith bases, getopts. NOTE: heredoc entries
    // use REAL newlines (\n in the Rust literal), not the printf \\n escape.
    "cat <<EOF\nline1\nline2\nEOF",                               // plain heredoc
    "cat <<-EOF\n\tindented\n\ttabbed\nEOF",                      // <<- strips leading tabs
    "x=world; cat <<EOF\nhello $x\nEOF",                          // heredoc expands vars
    "cat <<\"EOF\"\nno $expand here\nEOF",                        // quoted delimiter → literal
    "read a b <<EOF\none two three\nEOF\nprintf '%s\\n' \"$a\"",  // read from heredoc
    "printf '%s\\n' foo > /dev/null; printf '%s\\n' bar",         // redirect stdout to /dev/null
    "printf 'err\\n' >&2 2>/dev/null; printf 'out\\n'",           // stderr redirect
    "exec 3>&1; printf '%s\\n' via3 >&3; exec 3>&-",              // custom fd dup + close
    "{ printf a; printf b; } | tr a-z A-Z",                       // brace group piped
    "printf '%s\\n' one two three | wc -l | tr -d ' '",           // multi-stage pipeline
    "echo hi | cat | cat | cat",                                  // long pipeline
    "x=$(printf a; printf b; printf c); printf '%s\\n' \"$x\"",   // cmd-sub multi-statement
    "true | false; printf '%s\\n' \"$?\"",                        // pipeline exit = last stage
    "false | true; printf '%s\\n' \"$?\"",                        // pipeline exit = last (0)
    "false && printf a; printf '%s\\n' \"$?\"",                   // && short-circuit status
    "( exit 5 ); printf '%s\\n' \"$?\"",                          // subshell exit status
    "printf '%5.3f\\n' 3.14159",                                  // float width.precision
    "printf '%e\\n' 1000",                                        // scientific notation
    "printf '%g\\n' 0.0001",                                      // general float format
    "printf '%i\\n' 42",                                          // %i alias for %d
    "printf '%%\\n'",                                             // literal percent
    "printf 'a%sb%sc\\n' X Y",                                    // interleaved %s
    "printf '%o\\n' 64",                                          // octal output → 100
    // NB: `$(( base#num ))` is NOT portable — bash/ksh/zsh accept it but real
    // dash/ash reject it (POSIX arithmetic has no base# syntax), so it lives in
    // EXTENDED, and dash-strict mode rejects it to match the real shell.
    "set -- -a -b arg; getopts 'ab:' o; printf '%s\\n' \"$o\"",   // getopts first flag
    "printf '[%10s]\\n' hi",                                      // right-justified width
    "printf '[%-10s]\\n' hi",                                     // left-justified width
    "x=$(echo a b c); set -- $x; printf '%s\\n' \"$2\"",          // cmd-sub then word-split
    // Sixth expansion batch (harness_classify6.sh) — test/[ operators, external
    // command pipelines, expr, exit-status chains.
    "[ 5 -gt 3 ] && [ 3 -lt 5 ] && echo y",                       // numeric -gt/-lt
    "[ -n \"a\" -a -z \"\" ] && echo y",                          // -n/-z with -a
    "[ abc = abc ] && echo eq",                                   // string equality
    "[ abc != xyz ] && echo ne",                                  // string inequality
    "[ 5 -eq 5 -o 3 -eq 4 ] && echo or",                          // -eq with -o
    "[ ! -z x ] && echo notempty",                                // negated -z
    "test 3 -lt 4 && echo lt",                                    // `test` builtin form
    "[ -e /dev/null ] && echo exists",                            // -e file test
    "[ -d /tmp ] && echo isdir",                                  // -d directory test
    "v=foo; case $v in foo|bar) echo m;; esac",                   // case alternation
    "echo 'one two three' | cut -d' ' -f2",                       // cut field
    "echo abcABC | tr 'a-z' 'A-Z'",                               // tr translit
    "echo hello | wc -c | tr -d ' '",                             // wc -c piped
    "x=$(echo a; echo b); echo \"$x\" | wc -l | tr -d ' '",       // multiline cmd-sub count
    "for x in $(seq 1 3); do printf '%s' \"$x\"; done; echo",     // for over $(seq)
    "x=5; expr $x + 3",                                           // expr arithmetic
    "y=$(expr 10 / 2); echo \"$y\"",                              // expr in cmd-sub
    "true; echo $?; false; echo $?",                              // exit-status of true/false
    "x=hi; export x; sh -c 'echo $x'",                            // export crosses to child sh
    "a=1; b=$a; unset a; echo \"${b}\"",                          // unset original keeps copy
    "s='  trim  '; echo \"$s\" | sed 's/^ *//;s/ *$//'",          // sed trim
    "printf '%s|' a b; echo",                                     // printf format reuse
    // Seventh expansion batch (harness_classify7.sh) — quoting + nesting.
    "echo \"a\\\"b\"",                                            // escaped quote inside DQ
    "echo \"it's\"",                                             // apostrophe inside DQ
    "echo one\"two\"three",                                      // adjacent quoted/unquoted
    "echo \"$( echo nested )\"",                                 // cmd-sub inside DQ
    "echo \"\\$escaped\"",                                       // literal $ via backslash in DQ
    "set -- one two; echo \"$1,$2\"",                            // positional in DQ
    "set -- a b c; shift; echo \"$@\"",                          // shift then \"$@\"
    "f() { echo \"$#\"; }; f a b c d",                           // arg count in function
    "x=; echo \"${x:+has}${x:-none}\"",                          // adjacent :+ / :- on empty
    "echo \"$(echo \"$(echo deep)\")\"",                         // nested cmd-sub with inner DQ
    "a=$((1+1)); b=$((a+1)); c=$((b+1)); echo \"$a$b$c\"",       // chained arith assigns → 234
    "echo \"$(( 1+1 ))$(( 2+2 ))\"",                             // adjacent arith in DQ
    "x=5; echo \"result=$(( x * x ))\"",                         // arith interpolated in DQ
    "x=abc; y=\"pre-${x}-post\"; echo \"$y\"",                   // braced var in DQ string
    "a=x; b=y; echo \"$a$b\" \"$a-$b\"",                         // concat vs separated
    "v=$(false && echo yes || echo no); echo \"$v\"",           // cmd-sub with && ||
    "unset x; echo \"${x:=set}\"; echo \"$x\"",                  // := assigns then reads
    // Eighth expansion batch (harness_classify8.sh) — grouping, subshells,
    // nested loops, case globs, heredoc-with-cmd-sub.
    "{ echo a; echo b; }",                                       // brace group
    "( echo x; echo y )",                                        // subshell group
    "{ echo grouped; } | cat",                                  // brace group piped
    "x=5; ( x=10 ); echo $x",                                    // subshell var isolation
    "echo before; { false; } || echo recovered",                // group in || chain
    "v=$( { echo one; echo two; } ); echo \"$v\" | wc -l | tr -d ' '", // group in cmd-sub
    "for i in 1 2; do for j in a b; do printf '%s%s ' \"$i\" \"$j\"; done; done; echo", // nested for
    "n=0; for x in a b c; do case $x in b) continue;; esac; n=$((n+1)); done; echo $n", // continue in case
    "for x in 1 2 3 4 5; do [ $x -eq 3 ] && break; printf '%s' \"$x\"; done; echo", // break in for
    "cat <<END\n$(echo cmd-in-heredoc)\nEND",                    // cmd-sub inside heredoc
    "x=hi; cat <<END\nvalue=$x\nEND",                            // var inside heredoc
    "x=$(exit 2) || echo \"failed:$?\"",                         // cmd-sub failure status in ||
    "case \"\" in \"\") echo empty;; esac",                     // empty-string case
    "case 5 in [0-9]) echo digit;; esac",                       // case digit class
    "case abc in ?b?) echo mid;; esac",                          // case ? globs
    "x=file.txt; case $x in *.txt) echo text;; *.log) echo log;; esac", // case extension match
    "r=0; for x in 1 2 3; do r=$((r+x)); done; [ $r -eq 6 ] && echo sum6", // loop sum + test
    "printf '%s\\n' \"$( (echo a; echo b) | tail -1)\"",         // subshell piped in cmd-sub
    "seq 1 3 | while read n; do printf '%s' \"$n\"; done; echo",  // pipe into while-read
    "{ read a; read b; } <<END\nfirst\nsecond\nEND\necho \"$a-$b\"", // group read from heredoc
    // Ninth expansion batch (harness_classify9.sh) — arithmetic edge cases +
    // parameter-expansion operator combos.
    "echo $(( 3 + 4 * 2 - 1 ))",                                  // precedence → 10
    "echo $(( 2 * (3 + 4) ))",                                    // parens override → 14
    "echo $(( -10 / 3 ))",                                        // negative int div → -3
    "echo $(( -10 % 3 ))",                                        // negative modulo → -1
    "echo $(( 3 * -2 ))",                                         // multiply by negative → -6
    "echo $(( 1 + 2 == 3 ))",                                     // arith then compare → 1
    "echo $(( 3 <= 3 ))\" \"$(( 4 >= 5 ))",                       // <= and >=
    "echo $(( 1 && 1 && 0 ))\" \"$(( 0 || 0 || 1 ))",            // chained &&/||
    "echo $(( 5 & 6 | 1 ))",                                      // bitwise and-then-or → 5
    "echo $(( 15 >> 1 << 1 ))",                                   // chained shifts → 14
    "x=10; echo $(( x % 3 + x / 3 ))",                           // mod + div → 4
    "a=2; b=3; echo $(( a < b ? b - a : a - b ))",              // ternary with subtraction → 1
    "echo $(( 1 < 2 && 2 < 3 && 3 < 4 ))",                       // fully chained comparisons → 1
    "v=a/b/c; echo \"${v##*/}\" \"${v#*/}\"",                    // longest vs shortest prefix
    "v=file.tar.gz; echo \"${v%%.*}\" \"${v%.*}\"",             // longest vs shortest suffix
    "unset v; echo \"[${v:+set}][${v:-def}]\"",                  // :+ and :- on unset
    "v=; echo \"[${v:+set}][${v-keep}]\"",                       // :+ vs - on empty
    "echo \"${#unset_variable}\"",                               // length of unset → 0
    "v=a=b=c; echo \"${v#*=}\" \"${v##*=}\"",                    // strip through first vs last =
    "x=5; case $(( x > 3 )) in 1) echo big;; 0) echo small;; esac", // arith result in case
    "v=UPPER; case $v in [A-Z]*) echo caps;; esac",              // case uppercase class
    "v=123abc; case $v in [0-9]*) echo startsdigit;; esac",      // case starts-with-digit
    // Tenth expansion batch (harness_classify10.sh) — printf number formats,
    // arith assignment, IFS-with-explicit-count, redirection.
    "printf '%d\\n' 007",                                        // leading-zero decimal → 7
    "printf '%d\\n' +5",                                         // explicit-plus decimal → 5
    "printf '%.0f\\n' 2.7",                                      // float round to int → 3
    "printf '%x\\n' 4095",                                       // hex output → fff
    "printf '%08x\\n' 255",                                      // zero-padded hex
    "printf '%+.1f\\n' 1.5",                                     // signed one-decimal float
    "printf '%3d|%-3d\\n' 5 5",                                  // right vs left numeric width
    "printf '%s=%s\\n' key value",                              // key=value format
    "x=5; x=$((x+1)); echo \"$x\"",                             // arith reassign → 6
    "x=5; : $((x=x*2)); echo \"$x\"",                           // arith side-effect via : → 10
    "IFS=; v='a b c'; set -- $v; echo \"$#\"",                  // empty IFS → 1 field
    "IFS=:; for p in /a:/b:/c; do echo \"$p\"; done",           // IFS split in for-list
    "v='key=value'; k=\"${v%%=*}\"; val=\"${v#*=}\"; echo \"$k:$val\"", // split on first =
    "cat /dev/null; echo empty-cat-ok",                         // cat empty file
    "> /dev/null echo prefixed-redirect",                       // leading redirect
    "x=3.5; echo \"${x%.*}\"",                                  // strip fractional part
    "echo $(( 5 > 3 ? 5 : 3 ))\" \"$(( 2 > 8 ? 2 : 8 ))",       // two ternaries → 5 8
    "for i in $(seq 3 -1 1); do printf '%d' \"$i\"; done; echo", // descending seq
    // A `#`/`%`/`##`/`%%` pattern taken from an UNQUOTED $var must NOT be
    // word-split even under SH_WORD_SPLIT (on by default in bash/ksh/dash/sh) —
    // the whole var value is the pattern, spaces included. Regression for a
    // singsub-path split bug (missing PREFORK_SINGLE gate). Found by fuzzer.
    "v='a b c'; w='a b'; printf '[%s]' \"${v#$w}\"",            // spaced prefix pattern → [ c]
    "v='x y z'; w='y z'; printf '[%s]' \"${v%$w}\"",            // spaced suffix pattern → [x ]
    "v='hello world'; w='hello world'; printf '[%s]' \"${v#$w}\"", // whole-value pattern → []
    "p='/a b/c'; printf '[%s]' \"${p##*/}\"",                   // spaced path basename → [c]
];

/// Extended-feature corpus — indexed arrays, `[[`, `(( ))`, brace expansion,
/// substring/replace expansion, here-strings. Runs ONLY against `extended`
/// reference shells (zsh/bash/ksh), each vs the matching zshrs mode. Index
/// base differs by shell (zsh 1-based, bash/ksh 0-based) but the differential
/// compares each mode against its own reference, so base-agnostic and
/// per-mode-correct scripts both pass. Known dense-vs-sparse array
/// divergences (`a[5]=q` count, `unset a[i]`) are deliberately excluded.
const EXTENDED_CORPUS: &[&str] = &[
    "a=(x y z); printf '%s\\n' \"${#a[@]}\"",              // element count
    "a=(x y z); printf '[%s]' \"${a[@]}\"; printf '\\n'",  // splat
    "[[ abc == a* ]] && printf y; printf '\\n'",           // [[ glob match
    "[[ abc =~ ^a.c$ ]] && printf y; printf '\\n'",        // [[ regex
    "[[ x == x && y == y ]] && printf y; printf '\\n'",    // [[ &&
    "(( 3 > 2 )) && printf y; printf '\\n'",               // (( )) truth
    "x=0; (( x++ )); printf '%s\\n' \"$x\"",               // (( )) post-inc
    "(( v = 3 + 4 )); printf '%s\\n' \"$v\"",              // (( )) assign
    "for ((i=0;i<3;i++)); do printf '%s' \"$i\"; done; printf '\\n'", // C-for
    "v=abcdef; printf '%s\\n' \"${v:2:3}\"",               // substring
    "v=abcdef; printf '%s\\n' \"${v: -2}\"",               // negative offset
    "v=aXbXc; printf '%s\\n' \"${v//X/-}\"",               // global replace
    "v=aXbXc; printf '%s\\n' \"${v/X/-}\"",                // first replace
    "v=path/to/file; printf '%s\\n' \"${v##*/}\"",         // greedy prefix strip
    "printf '%s ' {a,b,c}; printf '\\n'",                  // brace list
    "printf '%s ' {1..4}; printf '\\n'",                   // brace range
    "printf '%s ' a{1,2}b; printf '\\n'",                  // brace with affixes
    "cat <<< hi",                                          // here-string
    "read x <<< 'in here'; printf '%s\\n' \"$x\"",         // here-string into read
    // Harder extended torture (bash/ksh/zsh agree, each vs its own shell).
    "a=(a b c d e); printf '%s\\n' \"${a[@]:1:2}\"",       // array slice
    "a=(x y z); a+=(q); printf '%s\\n' \"${#a[@]}\"",      // array append
    "a=(1 2 3); printf '%s\\n' \"${a[@]: -1}\"",           // last element (neg offset)
    "[[ hello == h?llo ]] && printf y; printf '\\n'",      // [[ ? glob
    "[[ \"a b\" == \"a b\" ]] && printf y; printf '\\n'",  // [[ quoted equal
    "(( x = 2 ** 8 )); printf '%s\\n' \"$x\"",             // (( )) power
    "x=5; printf '%s\\n' \"$(( x > 3 ? x : 0 ))\"",        // arith ternary
    "v=abcdef; printf '%s\\n' \"${v:(-3):2}\"",            // substring paren-neg offset
    "[[ abcXYZ =~ [A-Z]+ ]] && printf y; printf '\\n'",    // [[ regex char-class
    "v=aXbXcX; printf '%s\\n' \"${v//X}\"",                // replace-with-nothing
    "echo {1..3}{a,b}",                                    // brace cross-product
    "(( n=10, n%=3 )); printf '%s\\n' \"$n\"",             // comma + mod-assign
    "a=(one two three); printf '%s\\n' \"${a[@]#t}\"",     // per-element prefix strip
    // Expansion batch — verified identical across bash/ksh/zsh vs each mode
    // (scratchpad/harness_classify.sh). Base-agnostic (splat/count/slice) or
    // per-mode-correct against the matching reference.
    "a=(1 2 3); a+=(4 5); printf '%s\\n' \"${#a[@]}\"",     // append then count → 5
    "[[ abcdef == a*f ]] && printf y; printf '\\n'",        // [[ leading+trailing glob
    "[[ hello == h[aeiou]llo ]] && printf y; printf '\\n'", // [[ bracket class
    "[[ abc != xyz ]] && printf y; printf '\\n'",           // [[ inequality
    "[[ 5 -gt 3 ]] && [[ 2 -lt 4 ]] && printf y; printf '\\n'", // [[ arith tests chained
    "[[ -n foo && -z '' ]] && printf y; printf '\\n'",      // [[ -n/-z with &&
    "[[ abc == ?bc ]] && printf y; printf '\\n'",           // [[ single-char glob
    "[[ 'a b' == 'a b' ]] && printf y; printf '\\n'",       // [[ quoted-space equality
    "x=5; (( x *= 2 )); printf '%s\\n' \"$x\"",             // (( )) compound *=
    "(( a = 10 )); (( a-- )); printf '%s\\n' \"$a\"",       // (( )) post-decrement
    "(( r = 17 % 5 )); printf '%s\\n' \"$r\"",              // (( )) modulo
    "n=8; printf '%s\\n' \"$(( n & (n-1) ))\"",             // clear-lowest-bit idiom
    "for ((i=0; i<4; i++)); do printf '%s' \"$i\"; done; printf '\\n'", // C-for concat
    "v=abcdefgh; printf '%s\\n' \"${v:2:4}\"",              // substring offset+len
    "v=abcdef; printf '%s\\n' \"${v: -3:2}\"",              // negative-offset substring+len
    "v=aXbXcXd; printf '%s\\n' \"${v//X/_}\"",              // global replace
    "s=\"a b c\"; printf '%s\\n' \"${s// /-}\"",            // global replace spaces
    "printf '%s ' {2..8..2}; printf '\\n'",                 // numeric brace step
    "printf '%s ' {a..e}; printf '\\n'",                    // alpha brace range
    "echo x{1,2,3}y",                                       // brace list with affixes
    "x=3; case $x in [1-5]) printf lo;; esac; printf '\\n'", // case numeric class
    "a=(x y z); printf '%s\\n' \"${a[@]}\"",                // whole-array splat
    // Second expansion batch (harness_classify2.sh — bash/ksh/zsh agree).
    "a=(1 2 3); printf '%s\\n' \"${a[@]: -2}\"",            // last-2 slice (neg offset)
    "a=(a b c); printf '%s\\n' \"${#a[*]}\"",               // count via [*]
    "a=(one two three); printf '%s\\n' \"${a[*]}\"",        // [*] join with space
    "a=(x y z); IFS=,; printf '%s\\n' \"${a[*]}\"",         // [*] join with custom IFS
    "a=(1 2 3 4); printf '%s\\n' \"${a[@]:2}\"",            // slice from offset to end
    "a=(a b); printf '%s\\n' \"${a[@]+set}\"",              // +alt on set array
    "[[ abc =~ b ]] && printf y; printf '\\n'",             // =~ substring regex
    "[[ 2024 =~ ^[0-9]+$ ]] && printf num; printf '\\n'",   // =~ anchored digit regex
    "[[ abcABC == *ABC ]] && printf y; printf '\\n'",       // trailing glob
    "[[ \"\" == \"\" ]] && printf empty; printf '\\n'",     // empty == empty
    "[[ ab < ac ]] && printf lt; printf '\\n'",             // string less-than
    "(( 0 )) || printf zero; printf '\\n'",                 // (( )) false → ||
    "(( 5 )) && printf nonzero; printf '\\n'",              // (( )) true → &&
    "(( a = 5, b = a * 2 )); printf '%s\\n' \"$b\"",        // (( )) comma sequence
    "v=abcdef; printf '%s\\n' \"${v:0:3}\"",                // substring from 0
    "v=abcdef; printf '%s\\n' \"${v:3}\"",                  // substring to end
    "v=aaa; printf '%s\\n' \"${v/#a/X}\"",                  // anchored-prefix replace
    "v=aaa; printf '%s\\n' \"${v/%a/X}\"",                  // anchored-suffix replace
    "printf '%s ' {A..C}{1..2}; printf '\\n'",              // alpha×numeric cross-product
    "a=(1 2 3); s=0; for x in \"${a[@]}\"; do s=$((s+x)); done; printf '%s\\n' \"$s\"", // iterate+sum
    "v=Hello; [[ $v == H* ]] && printf y; printf '\\n'",    // var glob match
    "a=(1 2 3); printf '%s\\n' \"${a[@]/2/X}\"",            // per-element replace across array
    // Third expansion batch (harness_classify3.sh — bash/ksh/zsh agree).
    "a=(1 2 3); printf '%s\\n' \"${a[@]:0:2}\"",            // slice offset 0 len 2
    "a=(x); printf '%s\\n' \"${a[@]}\"",                    // single-element splat
    "a=(1 2 3); b=(\"${a[@]}\"); printf '%s\\n' \"${#b[@]}\"", // array copy preserves count
    "[[ abc == \"abc\" ]] && printf y; printf '\\n'",       // quoted RHS = literal match
    "[[ 5 == 5 ]] && printf y; printf '\\n'",               // numeric-looking string equal
    "[[ -z ${x} ]] && printf empty; printf '\\n'",          // -z on unset braced var
    "[[ abc == a?? ]] && printf y; printf '\\n'",           // two single-char globs
    "(( 3 < 2 )) && printf lt || printf ge; printf '\\n'",  // false (( )) → || branch
    "x=10; while (( x > 7 )); do x=$((x-1)); done; printf '%s\\n' \"$x\"", // (( )) while cond
    "v=Hello_World; printf '%s\\n' \"${v//_/ }\"",          // replace underscore with space
    "v=abc; printf '%s\\n' \"${v/b}\"",                     // replace-with-nothing (delete)
    "a=(1 2 3 4 5); printf '%s\\n' \"${a[@]:1}\"",          // slice offset to end
    "printf '%s ' {5..1}; printf '\\n'",                    // descending numeric range
    "[[ 100 -gt 99 ]] && printf y; printf '\\n'",           // [[ numeric -gt
    "x=5; (( y = x++ )); printf '%s %s\\n' \"$x\" \"$y\"",  // post-increment in arith assign
    "[[ foobar == foo* && foobar == *bar ]] && printf y; printf '\\n'", // compound glob AND
    "(( x = 1 << 10 )); printf '%s\\n' \"$x\"",             // (( )) shift-assign → 1024
    // Fourth expansion batch (harness_classify4.sh — per-element / iteration).
    "a=(x y z); printf '%s\\n' \"${a[@]#?}\"",              // strip first char of each
    "a=(foo bar); printf '%s\\n' \"${a[@]%o*}\"",           // strip suffix glob of each
    "a=(1 2 3 4); printf '%s\\n' \"${#a[@]}\" \"${a[@]: -1}\"", // count + last element
    "v=aaabbb; printf '%s\\n' \"${v//a/}\"",                // delete-all replace
    "v=a.b.c.d; printf '%s\\n' \"${v//./ }\"",              // replace all dots with space
    "(( n = 5 )); (( n > 3 && n < 10 )) && printf mid; printf '\\n'", // arith range test
    "a=(1 2 3); c=0; for x in \"${a[@]}\"; do (( c++ )); done; printf '%s\\n' \"$c\"", // iterate + (( c++ ))
    "[[ \"\" ]] || printf empty; printf '\\n'",             // [[ empty-string ]] is false
    "[[ x ]] && printf nonempty; printf '\\n'",             // [[ nonempty-string ]] is true
    "a=(one); a+=(two three); printf '%s\\n' \"${a[*]}\"",  // multi-element append
    // Fifth expansion batch (harness_classify5.sh — anchored/class/nested).
    "a=(1 2 3); printf '%s\\n' \"${a[@]/#/x}\"",            // prepend to each element
    "a=(1 2 3); printf '%s\\n' \"${a[@]/%/y}\"",            // append to each element
    "x=5; [[ $x -eq 5 ]] && printf y; printf '\\n'",        // [[ arithmetic -eq
    "[[ -e /dev/null ]] && printf exists; printf '\\n'",    // [[ file-exists test
    "a=(a b c); printf '%s\\n' \"${a[@]:0:0}\"; printf end", // zero-length slice
    "v=abcdef; printf '%s\\n' \"${v:(-2)}\"",               // paren negative offset
    "v=hello; printf '%s\\n' \"${v/[aeiou]/_}\"",           // bracket-class first replace
    "(( x = 5 )); printf '%s\\n' \"$(( x > 3 ? x * 2 : 0 ))\"", // arith ternary with mul
    "[[ 3.14 == 3.* ]] && printf y; printf '\\n'",          // glob on dotted string
    "[[ $(printf abc) == a* ]] && printf y; printf '\\n'",  // cmd-sub in [[ operand
    "for i in {1..3}; do for j in {1..2}; do printf '%s' \"$i$j\"; done; done; printf '\\n'", // nested brace-for
    "(( total = 0 )); for n in 1 2 3 4 5; do (( total += n )); done; printf '%s\\n' \"$total\"", // (( += )) accumulate
    // Non-portable `base#num` arithmetic (bash/ksh/zsh accept; dash/ash reject).
    "printf '%s\\n' \"$(( 16#ff ))\"",                     // hex base → 255
    "printf '%s\\n' \"$(( 8#17 ))\"",                      // octal base → 15
    "printf '%s\\n' \"$(( 2#11111111 ))\"",                // binary base → 255
    // Sixth expansion batch (harness_classify6.sh).
    "a=(1 2 3); echo \"${a[@]}\" | tr ' ' '+'",            // splat piped to tr
    "[[ abc =~ ^abc$ ]] && echo m",                        // =~ fully anchored
    "[[ x != y ]] && echo ne",                             // [[ string !=
    "(( 2 + 2 == 4 )) && echo math",                       // (( )) equality truth
    "a=(a b c d e); echo \"${a[@]:1}\"",                    // slice offset to end
    "a=(x y); a=(\"${a[@]}\" z); echo \"${#a[@]}\"",        // splat-append copy → 3
    "v=Hello; [[ ${v:0:1} == H ]] && echo cap",            // substring in [[ ]]
    "a=(1 2 3); echo \"$(( ${#a[@]} * 2 ))\"",             // array count in arith → 6
    "a=(3 1 2); n=${#a[@]}; echo \"$n\"",                  // count into scalar
    // Seventh expansion batch (harness_classify7.sh).
    "a=(\"a b\" c); echo \"${#a[@]}\"",                    // quoted-space element → count 2
    "a=(1 2 3); printf '%s,' \"${a[@]}\"; echo",           // per-element printf
    "[[ \"a b\" == \"a b\" ]] && echo eq",                 // quoted-space equality
    "[[ \"\" != x ]] && echo ne",                          // empty != nonempty
    "x=42; [[ $x == 4* ]] && echo m",                      // var glob prefix
    "(( a = 2, b = 3, c = a + b )); echo \"$c\"",          // comma-sequence assigns → 5
    "v=aXbXc; echo \"${v%%X*}|${v##*X}\"",                 // greedy strip both ends
    "(( x = 10 % 3 )); [[ $x -eq 1 ]] && echo mod",        // (( )) result in [[ -eq ]]
    // Eighth expansion batch (harness_classify8.sh).
    "a=(1 2 3); ( a=(x y); echo \"${#a[@]}\" ); echo \"${#a[@]}\"", // subshell array isolation
    // (empty-array `${#a[@]}` count omitted — legitimately differs on mksh)
    "a=(a b c); i=0; while [ $i -lt \"${#a[@]}\" ]; do printf '%s' \"${a[$i]}\"; i=$((i+1)); done; echo", // index while over array
    "a=(3 1 2); b=(\"${a[@]}\"); echo \"${b[0]}${b[1]}${b[2]}\"", // array copy by splat
    "a=(1 2 3); (( sum=0 )); for x in \"${a[@]}\"; do (( sum+=x )); done; echo $sum", // (( += )) over array
    "[[ abc == a?c ]] && [[ abc == ??? ]] && echo both", // two [[ globs chained
    "x=5; case $x in [1-3]) echo lo;; [4-6]) echo mid;; esac", // case numeric ranges
    // Ninth expansion batch (harness_classify9.sh).
    // (bare `a[N]` arith subscript omitted — 0-based vs 1-based index differs
    //  across shells / emulation legs, flagged by the full-matrix run.)
    "a=(10 20 30); s=0; for x in \"${a[@]}\"; do (( s += x )); done; echo $s", // sum via (( ))
    "[[ 5 -gt 3 && 3 -gt 1 ]] && echo chain",             // [[ arith && chain
    "[[ abc == a[bc]c ]] && echo cls",                    // [[ bracket-class glob
    "[[ abcdef == a*e? ]] && echo m",                     // [[ mixed */? glob
    "a=(x y z); echo \"${a[@]:1:1}\"",                    // single-element slice
    "v=Hello; [[ $v == [A-Z]* ]] && echo caps",           // [[ leading-uppercase class
    "(( x = 2**10 )); echo $x",                           // (( )) power → 1024
    "a=(a b c); [[ ${#a[@]} -eq 3 ]] && echo three",      // array count in [[ -eq ]]
    "x=42; [[ $x =~ ^[0-9]+$ ]] && echo num",             // =~ digit regex
    "[[ \"foo bar\" == *\" \"* ]] && echo hasspace",      // [[ glob for embedded space
    "a=(1 2 3); b=$(( ${a[0]} + ${a[2]} )); echo $b",     // ${a[i]} in arith cmd-sub
    // Tenth expansion batch (harness_classify10.sh) — arith compound-assign ops.
    "x=5; (( x <<= 2 )); echo $x",                        // left-shift-assign → 20
    "x=20; (( x >>= 2 )); echo $x",                       // right-shift-assign → 5
    "x=6; (( x &= 3 )); echo $x",                         // and-assign → 2
    "x=5; (( x |= 2 )); echo $x",                         // or-assign → 7
    "x=5; (( x ^= 1 )); echo $x",                         // xor-assign → 4
    "[[ $((3+4)) -eq 7 ]] && echo arith-in-test",         // arith cmd-sub in [[ -eq ]]
    "a=(1 2 3 4); s=0; for x in \"${a[@]}\"; do s=$((s+x)); done; echo $s", // iterate + sum → 10
    // NB: `local` is intentionally NOT here — ksh93 has no `local` builtin
    // (it uses `typeset`), so it is a legitimate ksh divergence, not a bug.
    // Bare `${a[N]}` single-index is also excluded — 1-based (zsh) vs 0-based
    // (bash/ksh) legitimately differs and would flag a non-bug.
    // A `${v/PAT/r}` replace whose PAT comes from an unquoted $var must use the
    // WHOLE var (spaces included) as the pattern, never word-split it, even
    // under SH_WORD_SPLIT. `/`-replace is bash/ksh/zsh-only (not POSIX), so it
    // lives in EXTENDED. Regression for the singsub PREFORK_SINGLE split bug.
    "v='a b c'; w='a b'; printf '[%s]' \"${v/$w/X}\"",     // spaced replace pattern → [X c]
    "v='a b c'; w='b c'; printf '[%s]' \"${v//$w/Y}\"",    // spaced global replace → [a Y]
];

fn find_shell(candidates: &[&str]) -> Option<String> {
    for c in candidates {
        if c.starts_with('/') {
            if Path::new(c).exists() {
                return Some((*c).to_string());
            }
        } else if let Ok(out) = Command::new("sh")
            .args(["-c", &format!("command -v {c}")])
            .output()
        {
            if out.status.success() {
                let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !p.is_empty() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// (stdout, success). stderr is intentionally dropped — its text
/// legitimately differs across shells; only stdout + exit-sign are compared.
fn run(bin: &str, args: &[&str], script: &str) -> (String, bool) {
    let mut full: Vec<&str> = args.to_vec();
    full.push("-c");
    full.push(script);
    let out = Command::new(bin)
        .args(&full)
        .output()
        .unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

/// Run one corpus script through a parity case: zshrs with the case flags,
/// and the reference binary (optionally prefixed with `emulate X` for the
/// cross-emulation legs, and `-f` when the reference is zsh).
fn run_case(case: &ParityCase, refbin: &str, script: &str) -> ((String, bool), (String, bool)) {
    // zshrs side: the case's flags + `-f`.
    let mut zargs: Vec<&str> = case.zshrs_flags.to_vec();
    zargs.push("-f");
    let z = run(&zshrs_bin(), &zargs, script);

    // Reference side. Cross-emulation legs run zsh with `emulate X` prepended;
    // a bare zsh reference also takes `-f` (no rc) for determinism.
    let ref_is_zsh = case.ref_emulate.is_some() || case.name == "zsh";
    let ref_args: &[&str] = if ref_is_zsh { &["-f"] } else { &[] };
    let r = match case.ref_emulate {
        Some(emu) => run(refbin, ref_args, &format!("emulate {emu}\n{script}")),
        None => run(refbin, ref_args, script),
    };
    (r, z)
}

/// The enforcement decision, extracted so it is unit-testable without
/// depending on which reference shells happen to be installed: when
/// `ZSHRS_REQUIRE_REF_SHELLS` is set, any absent reference shell is fatal.
fn missing_is_fatal(require: bool, missing: &[&str]) -> bool {
    require && !missing.is_empty()
}

#[test]
fn enforcement_gate_logic() {
    // Not requiring → a miss is a skip, never fatal.
    assert!(!missing_is_fatal(false, &["ksh"]));
    assert!(!missing_is_fatal(false, &[]));
    // Requiring + all present → not fatal.
    assert!(!missing_is_fatal(true, &[]));
    // Requiring + a miss → fatal. This is the CI contract: a missing
    // reference shell fails the build instead of silently passing.
    assert!(missing_is_fatal(true, &["ksh"]));
    assert!(missing_is_fatal(true, &["ksh", "dash"]));
}

#[test]
fn shell_aliases_map_to_base_modes() {
    // `--ash` is the Almquist family (== `--dash` strict-POSIX) and `--mksh`
    // is a Korn variant (== `--ksh` base). Verify the aliases produce the
    // same observable behavior as their base modes — no reference binary
    // needed.
    let probe = |flag: &str, script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args([flag, "-f", "-c", script])
            .output()
            .expect("spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    };
    // ash ≡ dash: strict-POSIX rejections + posix-faithful splitting.
    for script in [
        "echo $((2**10))",                            // dash arith: `**` rejected
        "[[ 1 = 1 ]] && echo y",                      // `[[` not reserved
        "IFS=:; v=a:b:; set -- $v; printf %s \"$#\"", // trailing-empty drop → 2
        "printf '%d' A",                              // strtoimax printf → exit 1
    ] {
        assert_eq!(
            probe("--ash", script),
            probe("--dash", script),
            "--ash vs --dash: {script}"
        );
    }
    // mksh ≡ ksh and pdksh ≡ ksh: same emulation base (ksharrays etc.).
    for script in [
        "a=(x y z); printf '%s' \"${a[0]}\"", // 0-indexed arrays
        "print -r -- ${options[ksharrays]}",
        "print -r -- ${options[shwordsplit]}",
    ] {
        assert_eq!(
            probe("--mksh", script),
            probe("--ksh", script),
            "--mksh vs --ksh: {script}"
        );
        assert_eq!(
            probe("--pdksh", script),
            probe("--ksh", script),
            "--pdksh vs --ksh: {script}"
        );
    }
}

#[test]
fn dash_strict_rejects_substring_expansion() {
    // `${var:OFFSET[:LEN]}` substring is a ksh/bash/zsh extension — real dash
    // and ash reject it as "Bad substitution" (non-zero exit). The POSIX
    // `:-`/`:+`/`:=`/`:?` operators and `#`/`##`/`%`/`%%`/`${#v}` are NOT
    // substring and MUST keep working. macOS `/bin/sh` is bash (accepts
    // substring), so this is gated on dash_strict (--dash/--ash), not the
    // broader posix-faithful set. Found by the per-mode param fuzzer.
    let probe = |flag: &str, script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args([flag, "-f", "-c", script])
            .output()
            .expect("spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    };
    for strict in ["--dash", "--ash"] {
        for sub in [
            "v=abcdef; echo \"${v:2}\"",
            "v=abcdef; echo \"${v:2:3}\"",
            "v=abcdef; echo \"${v: -2}\"",
        ] {
            let (out, ok) = probe(strict, sub);
            assert!(
                !ok,
                "{strict}: substring `{sub}` must be a bad substitution"
            );
            assert!(
                out.trim().is_empty(),
                "{strict}: substring `{sub}` prints nothing"
            );
        }
        // `${(flags)name}` parameter-flag blocks are also a bad substitution in
        // dash (POSIX `${` never starts with `(`).
        for flag in ["x=hi; echo \"${(U)x}\"", "x=a; echo \"${(w)x}\""] {
            let (_o, ok) = probe(strict, flag);
            assert!(
                !ok,
                "{strict}: param-flag `{flag}` must be a bad substitution"
            );
        }
        // POSIX operators + length must still work under strict mode.
        for (script, want) in [
            ("v=; echo \"${v:-def}\"", "def"),
            ("v=x; echo \"${v:+set}\"", "set"),
            ("unset v; echo \"${v:=asg}\"", "asg"),
            ("v=abc; echo \"${v#a}\"", "bc"),
            ("v=abc; echo \"${v%c}\"", "ab"),
            ("v=abc; echo \"${#v}\"", "3"),
        ] {
            let (out, ok) = probe(strict, script);
            assert!(ok, "{strict}: POSIX `{script}` must succeed");
            assert_eq!(out.trim(), want, "{strict}: {script}");
        }
    }
    // --sh (bash-backed on macOS), --bash, --ksh, --zsh DO support substring.
    for m in ["--sh", "--bash", "--ksh", "--zsh"] {
        let (out, ok) = probe(m, "v=abcdef; echo \"${v:2:3}\"");
        assert!(ok && out.trim() == "cde", "{m}: substring must yield cde");
    }
}

#[test]
fn dash_strict_rejects_arith_command() {
    // dash/ash have no `(( ))` arithmetic command — `((` is two nested
    // subshells `( (`, so `(( 1 + 1 ))` runs the command `1` (→ not found,
    // non-zero) rather than arith-evaluating. zsh/bash/ksh DO have the arith
    // command (exit reflects the truth value). Found by the per-mode
    // dash-strictness sweep. `for ((…))` and `$(( ))` use different lexer
    // paths and are covered by the EXTENDED corpus / regression cases below.
    let probe = |flag: &str, script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args([flag, "-f", "-c", script])
            .output()
            .expect("spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    };
    for strict in ["--dash", "--ash"] {
        // `(( 1 + 1 ))` → runs command `1` in a subshell → non-zero exit.
        let (_o, ok) = probe(strict, "(( 1 + 1 ))");
        assert!(
            !ok,
            "{strict}: `(( 1 + 1 ))` must run as a subshell command (non-zero)"
        );
        // A genuine nested subshell still works: `(( echo hi ))` → prints hi.
        let (out, ok2) = probe(strict, "(( echo hi ))");
        assert!(
            ok2 && out.trim() == "hi",
            "{strict}: nested subshell `(( echo hi ))` → hi"
        );
        // `$(( ))` arithmetic expansion is unaffected (POSIX).
        let (out, ok3) = probe(strict, "echo $((2+3))");
        assert!(ok3 && out.trim() == "5", "{strict}: $((2+3)) still works");
    }
    // zsh/bash/ksh keep the arith command: `(( 1 ))` true → exit 0.
    for m in ["--zsh", "--bash", "--ksh"] {
        let (_o, ok) = probe(m, "(( 1 ))");
        assert!(ok, "{m}: `(( 1 ))` arith command truthy → exit 0");
        let (_o2, ok_false) = probe(m, "(( 0 ))");
        assert!(!ok_false, "{m}: `(( 0 ))` arith command falsy → exit 1");
    }
}

#[test]
fn dash_strict_rejects_braced_array_subscript() {
    // dash/ash have no array subscripts — braced `${name[...]}` is a "Bad
    // substitution" there. The array LITERAL `a=(…)` is already rejected at
    // parse time (so arrays can't even be created under --dash); this covers
    // the subscript-expansion forms. The unbraced `$name[...]` form is left as
    // `$name` + literal `[...]`, matching dash exactly. Found by the per-mode
    // dash-strictness sweep.
    //
    // KNOWN RESIDUAL: the `[@]` splat compiles to a fusevm array opcode
    // (JOIN_STAR / ARRAY_ALL) that bypasses the ported paramsubst, so
    // `${a[@]}` is not yet gated here. Tracked as backlog — completing it needs
    // dash_strict checks at the several compile_zsh.rs splat-emit sites. The
    // other four subscript shapes (`[N]`/`[key]`/`[*]`/negative) ARE gated.
    let probe = |flag: &str, script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args([flag, "-f", "-c", script])
            .output()
            .expect("spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    };
    for strict in ["--dash", "--ash"] {
        for sub in [
            "a=hi; echo \"${a[0]}\"",
            "echo \"${a[1]}\"",
            "a=hi; echo \"${a[*]}\"",
            "echo \"${x[key]}\"",
        ] {
            let (_o, ok) = probe(strict, sub);
            assert!(
                !ok,
                "{strict}: braced subscript `{sub}` must be a bad substitution"
            );
        }
        // The UNBRACED form stays literal (matches dash): `$a[0]` → value + `[0]`.
        let (out, ok) = probe(strict, "a=hi; echo \"$a[0]\"");
        assert!(
            ok && out.trim() == "hi[0]",
            "{strict}: unbraced $a[0] stays literal → hi[0]"
        );
        // Normal ${x} forms still work.
        let (out2, ok2) = probe(strict, "x=hi; echo \"${x}:${#x}\"");
        assert!(
            ok2 && out2.trim() == "hi:2",
            "{strict}: plain ${{x}}/${{#x}} still work"
        );
    }
    // --zsh/--bash keep array subscripts.
    for m in ["--zsh", "--bash"] {
        let (out, ok) = probe(m, "a=(x y z); echo \"${a[1]}\"");
        assert!(ok, "{m}: array subscript must work (got exit failure)");
        assert!(!out.is_empty(), "{m}: array subscript must produce output");
    }
}

#[test]
fn dash_strict_rejects_nonposix_reserved_words() {
    // dash/ash have none of the zsh/bash/ksh reserved words `[[` / `function`
    // / `coproc` — each is an ordinary command word there (`[[`/`coproc` → not
    // found; `function` → the following `{` is a syntax error). The POSIX
    // `name()` function form and every POSIX reserved word must keep working.
    // Found by the per-mode dash-strictness sweep.
    let probe = |flag: &str, script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args([flag, "-f", "-c", script])
            .output()
            .expect("spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    };
    for strict in ["--dash", "--ash"] {
        for script in ["function f { echo hi; }; f", "coproc cat", "[[ a == a ]]"] {
            let (_o, ok) = probe(strict, script);
            assert!(
                !ok,
                "{strict}: `{script}` must not run (non-POSIX reserved word)"
            );
        }
        // POSIX function form + reserved words still work.
        let (out, ok) = probe(strict, "f() { echo hi; }; f");
        assert!(
            ok && out.trim() == "hi",
            "{strict}: POSIX `name()` function must work"
        );
        let (out2, ok2) = probe(strict, "if true; then echo y; fi");
        assert!(ok2 && out2.trim() == "y", "{strict}: POSIX `if` must work");
    }
    // zsh/bash/ksh keep `function` and `coproc`.
    for m in ["--zsh", "--bash", "--ksh"] {
        let (out, ok) = probe(m, "function g { echo fn; }; g");
        assert!(
            ok && out.trim() == "fn",
            "{m}: `function` keyword must work"
        );
    }
}

#[test]
fn dash_strict_rejects_process_substitution() {
    // dash/ash have no `<(...)` / `>(...)` process substitution — the `<`/`>`
    // is a plain redirection and the `(` is an unexpected target, so the parser
    // rejects it (non-zero). bash/zsh support it. macOS `/bin/sh` is bash, so
    // this is gated on dash_strict (--dash/--ash), not posix-faithful. Found by
    // the per-mode dash-strictness sweep.
    let probe = |flag: &str, script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args([flag, "-f", "-c", script])
            .output()
            .expect("spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    };
    for strict in ["--dash", "--ash"] {
        for ps in [
            "cat <(echo hi)",
            "diff <(echo a) <(echo a)",
            "echo x > >(cat)",
        ] {
            let (_o, ok) = probe(strict, ps);
            assert!(
                !ok,
                "{strict}: process substitution `{ps}` must be a syntax error"
            );
        }
        // Plain redirections must still work under strict mode.
        let (out, ok) = probe(strict, "printf 'y\\n' | cat");
        assert!(
            ok && out.trim() == "y",
            "{strict}: plain pipe/redirect still works"
        );
    }
    // bash/zsh support process substitution.
    for m in ["--bash", "--zsh"] {
        let (out, ok) = probe(m, "cat <(echo works)");
        assert!(
            ok && out.trim() == "works",
            "{m}: process substitution → works"
        );
    }
}

#[test]
fn bash_mode_self_contained() {
    // Self-contained bash-mode checks (no /bin/bash needed): bash is a
    // superset of POSIX sh — brace expansion is ON (unlike `emulate sh`),
    // and it inherits the posix-faithful fixes (trailing-empty split,
    // strtoimax printf %d) since bash drops trailing empties and errors on
    // non-numeric %d like dash.
    let cases: &[(&str, &str)] = &[
        ("printf '%s ' {a,b,c}", "a b c "),  // brace expansion on
        ("printf '%s ' {1..4}", "1 2 3 4 "), // brace range
        ("IFS=:; v=a:b:; set -- $v; printf %s \"$#\"", "2"), // trailing-empty drop
    ];
    for (script, want) in cases {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            *want,
            "--bash: {script}"
        );
    }
    // printf %d numeric contract (bash errors on non-numeric, like dash).
    let out = Command::new(zshrs_bin())
        .args(["--bash", "-f", "-c", "printf '%d' A"])
        .output()
        .expect("spawn");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "0");
    assert!(
        !out.status.success(),
        "--bash printf %d A should exit non-zero"
    );

    // bash (unlike zsh/ksh/dash/sh) also errors on an explicitly-supplied EMPTY
    // numeric operand — prints 0 but exits 1 — for every numeric conversion. A
    // MISSING operand is NOT an error. Divergence point (bash-only), so it lives
    // here rather than in the shared corpus. Found by gen_printf_permode fuzzer.
    let run = |flags: &[&str], script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args(flags)
            .args(["-f", "-c", script])
            .output()
            .expect("spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    };
    for conv in ["%d", "%i", "%o", "%u", "%x", "%X"] {
        let (out, ok) = run(&["--bash"], &format!("printf '{conv}' ''"));
        assert_eq!(out, "0", "--bash printf {conv} '' prints 0");
        assert!(
            !ok,
            "--bash printf {conv} '' (empty arg) must exit non-zero"
        );
        // Missing operand is not an error.
        let (_o, ok_missing) = run(&["--bash"], &format!("printf '{conv}\\n'"));
        assert!(
            ok_missing,
            "--bash printf {conv} (missing arg) must exit zero"
        );
        // zsh/ksh/dash/sh accept an empty operand as a clean 0 (exit zero).
        for m in ["--zsh", "--ksh", "--dash", "--sh"] {
            let (_o2, ok2) = run(&[m], &format!("printf '{conv}' ''"));
            assert!(ok2, "{m} printf {conv} '' must exit zero (clean 0)");
        }
    }
    // Recycling: empty then valid — bash prints both, still exits 1.
    let (out, ok) = run(&["--bash"], "printf '%d\\n' '' 5");
    assert_eq!(out, "0\n5\n", "--bash recycling prints 0 then 5");
    assert!(
        !ok,
        "--bash recycling with an empty operand still exits non-zero"
    );

    // POSIX sh must NOT brace-expand (regression guard for the gate).
    let out = Command::new(zshrs_bin())
        .args(["--sh", "-f", "-c", "printf '%s ' {a,b,c}"])
        .output()
        .expect("spawn");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "{a,b,c} ",
        "--sh must not brace-expand"
    );
}

#[test]
fn bash_param_expansion_indirect_and_casemod() {
    // bash-only param syntax that zsh/ksh reject (so it can't live in the
    // shared corpus): `${!name}` indirect and `${v^^}`/`${v,,}`/`${v^}`/
    // `${v,}` case modification. Values are fixed, so no reference binary
    // is needed.
    let bash = |script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    };
    let cases: &[(&str, &str)] = &[
        // indirect (B)
        ("x=5; y=x; printf '%s' \"${!y}\"", "5"),
        ("foo=bar; ref=foo; printf '%s' \"${!ref}\"", "bar"),
        ("unset q; r=q; printf '[%s]' \"${!r}\"", "[]"),
        // case modification (C)
        ("v=Hello; printf '%s' \"${v^^}\"", "HELLO"),
        ("v=Hello; printf '%s' \"${v,,}\"", "hello"),
        ("v=hello; printf '%s' \"${v^}\"", "Hello"),
        ("v=HELLO; printf '%s' \"${v,}\"", "hELLO"),
        (
            "v=abcDEF; printf '%s-%s' \"${v^^}\" \"${v,,}\"",
            "ABCDEF-abcdef",
        ),
    ];
    for (script, want) in cases {
        assert_eq!(bash(script).0, *want, "--bash: {script}");
    }
    // These must remain a "bad substitution" error under --zsh / --dash
    // (the syntax is gated to bash mode only).
    for mode in ["--zsh", "--dash"] {
        let out = Command::new(zshrs_bin())
            .args([mode, "-f", "-c", "x=5; y=x; printf '%s' \"${!y}\""])
            .output()
            .expect("spawn");
        assert!(
            !out.status.success(),
            "{mode}: ${{!y}} must not do indirect"
        );
    }
}

#[test]
fn bash_regex_rematch_read_a_indices() {
    // Bash features surfaced by harder fuzzing, gated to --bash:
    //   * `[[ x =~ (a)(b) ]]` — regex with adjacent capture groups (parsed
    //     wrong under bash/ksh emulation before the lexer fix).
    //   * `$BASH_REMATCH` — array of the whole match + capture groups.
    //   * `read -a arr` — bash array-read flag (zsh/ksh use -A).
    //   * `${!arr[@]}` — array indices.
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // =~ regex with adjacent groups + BASH_REMATCH.
    assert_eq!(
        bash("[[ abcdef =~ (b)(c) ]] && printf '%s-%s-%s' \"${BASH_REMATCH[0]}\" \"${BASH_REMATCH[1]}\" \"${BASH_REMATCH[2]}\""),
        "bc-b-c"
    );
    assert_eq!(
        bash("[[ 2024-01-15 =~ ([0-9]+)-([0-9]+)-([0-9]+) ]] && printf '%s/%s/%s' \"${BASH_REMATCH[1]}\" \"${BASH_REMATCH[2]}\" \"${BASH_REMATCH[3]}\""),
        "2024/01/15"
    );
    // read -a array read.
    assert_eq!(
        bash("read -a arr <<< 'x y z'; printf '%s' \"${arr[1]}\""),
        "y"
    );
    assert_eq!(
        bash("read -a arr <<< 'one two three'; printf '%s' \"${#arr[@]}\""),
        "3"
    );
    // ${!arr[@]} indices (3 separate args → joined with a space here).
    assert_eq!(bash("a=(x y z); printf '%s ' \"${!a[@]}\""), "0 1 2 ");
    assert_eq!(
        bash("a=(p q r); for i in \"${!a[@]}\"; do printf '%s:%s ' \"$i\" \"${a[$i]}\"; done"),
        "0:p 1:q 2:r "
    );

    // BASH_REMATCH must stay unset under --zsh (uses $match instead).
    let out = Command::new(zshrs_bin())
        .args([
            "--zsh",
            "-f",
            "-c",
            "[[ ab =~ (a) ]]; printf '[%s]' \"${BASH_REMATCH:-unset}\"",
        ])
        .output()
        .expect("spawn");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "[unset]");
}

#[test]
fn bash_mapfile_readarray() {
    // `mapfile` / `readarray` read lines from stdin into an array (bash).
    // Values are fixed via here-strings, so no reference binary is needed.
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // -t strips the trailing newline; count is 3.
    assert_eq!(
        bash("mapfile -t L <<< $'a\\nb\\nc'; printf '%s' \"${#L[@]}\""),
        "3"
    );
    assert_eq!(
        bash("mapfile -t L <<< $'a\\nb\\nc'; printf '%s' \"${L[1]}\""),
        "b"
    );
    // readarray is an alias.
    assert_eq!(
        bash("readarray -t L <<< $'x\\ny'; printf '%s' \"${#L[@]}\""),
        "2"
    );
    // Without -t the trailing delimiter is kept in each element.
    assert_eq!(
        bash("mapfile L <<< $'x\\ny'; printf '[%s]' \"${L[@]}\""),
        "[x\n][y\n]"
    );
    // -s skip + -n count.
    assert_eq!(
        bash("mapfile -t -s 1 -n 2 L <<< $'a\\nb\\nc\\nd'; printf '%s' \"${L[*]}\""),
        "b c"
    );
    // -d custom delimiter.
    assert_eq!(
        bash("mapfile -d : -t L <<< 'a:b:c'; printf '%s' \"${L[1]}\""),
        "b"
    );
    // Default array name is MAPFILE.
    assert_eq!(
        bash("mapfile -t <<< $'p\\nq'; printf '%s' \"${MAPFILE[0]}\""),
        "p"
    );

    // Gated to non-zsh: `mapfile` is "command not found" under --zsh.
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "mapfile x"])
        .output()
        .expect("spawn");
    assert!(
        !out.status.success(),
        "--zsh: mapfile must be command-not-found"
    );
}

#[test]
fn bash_sparse_arrays() {
    // bash indexed arrays are SPARSE: `a[5]=q` on a 3-element array leaves
    // indices {0,1,2,5} (NOT dense 0..5 with padding), and `unset a[i]`
    // removes an index leaving a gap. zsh/zshrs arrays are dense `Vec`, so
    // this is emulated under --bash via a side "holes" table consulted by
    // `${a[@]}`/`${a[*]}`/`${#a[@]}`/`${!a[@]}`. Values are fully fixed, so
    // no reference binary is needed (ground truth is bash 5.x, verified).
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // Padding-on-assign creates holes, not dense empties. (Ground truth is
    // bash 5.x; `printf '%s'` on a `[@]` splat cycles the format per arg, so
    // the live elements concatenate; `[*]` is one joined arg and keeps IFS.)
    assert_eq!(bash(r#"a=(x y z); a[5]=q; printf '%s' "${#a[@]}""#), "4");
    assert_eq!(bash(r#"a=(x y z); a[5]=q; printf '%s' "${a[@]}""#), "xyzq");
    assert_eq!(
        bash(r#"a=(x y z); a[5]=q; printf '%s' "${a[*]}""#),
        "x y z q"
    );
    assert_eq!(
        bash(r#"a=(x y z); a[5]=q; printf '%s' "${!a[*]}""#),
        "0 1 2 5"
    );
    // Custom IFS applies to the star-join over live elements only.
    assert_eq!(
        bash(r#"a=(x y z); a[5]=q; IFS=,; printf '%s' "${a[*]}""#),
        "x,y,z,q"
    );
    // `unset a[i]` is 0-based and leaves a hole.
    assert_eq!(
        bash(r#"a=(x y z); unset a[1]; printf '%s' "${a[@]}""#),
        "xz"
    );
    assert_eq!(
        bash(r#"a=(x y z); unset a[1]; printf '%s' "${!a[@]}|${#a[@]}""#),
        "02|2"
    );
    // Fully sparse from an empty array.
    assert_eq!(
        bash(r#"a=(); a[3]=d; a[7]=h; printf '%s' "${a[@]}|${!a[@]}|${#a[@]}""#),
        "dh|37|2"
    );
    // Re-assigning a hole makes it live again (count returns to dense).
    assert_eq!(
        bash(r#"a=(x y z); unset a[1]; a[1]=Y; printf '%s' "${a[@]}|${#a[@]}""#),
        "xYz|3"
    );
    // A full `a=(...)` reassign clears all holes.
    assert_eq!(
        bash(r#"a=(x y z); a[5]=q; a=(m n); printf '%s' "${#a[@]}|${!a[@]}""#),
        "2|01"
    );
    // LEGIT empty elements are NOT holes — a quoted splat keeps them.
    assert_eq!(bash(r#"a=(x "" z); printf '<%s>' "${a[@]}""#), "<x><><z>");
    assert_eq!(bash(r#"a=(x "" z); printf '%s' "${#a[@]}""#), "3");

    // --zsh must be UNAFFECTED: dense semantics, `a[6]=q` on a 3-elem array
    // (1-based) pads to length 6.
    let zsh = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--zsh", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert_eq!(zsh(r#"a=(x y z); a[6]=q; printf '%s' "${#a[@]}""#), "6");
    // The padding empties survive as real (dense) elements under --zsh.
    assert_eq!(
        zsh(r#"a=(x y z); a[6]=q; printf '<%s>' "${a[@]}""#),
        "<x><y><z><><><q>"
    );

    // Explicit-index array literal is sparse: `a=([2]=x [5]=y)` → {2,5}.
    // ("${a[*]}" is one joined arg, so IFS separates cleanly; likewise the
    // quoted "${!a[@]}" is a single space-joined index string.)
    assert_eq!(
        bash(r#"a=([2]=two [5]=five); printf '%s' "${a[*]}""#),
        "two five"
    );
    assert_eq!(
        bash(r#"a=([2]=two [5]=five); printf '%s' "${!a[*]}""#),
        "2 5"
    );
    assert_eq!(bash(r#"a=([2]=two [5]=five); printf '%s' "${#a[@]}""#), "2");
    // Mixing a literal with a later subscript-assign keeps both live.
    assert_eq!(bash(r#"a=([3]=x); a[1]=y; printf '%s' "${!a[*]}""#), "1 3");
    assert_eq!(bash(r#"a=([3]=x); a[1]=y; printf '%s' "${a[*]}""#), "y x");
    // `declare -a` / `typeset -a` with explicit indices is sparse too.
    assert_eq!(
        bash(r#"declare -a a=([5]=x [10]=y); printf '%s|%s|%s' "${a[*]}" "${!a[*]}" "${#a[@]}""#),
        "x y|5 10|2"
    );
    assert_eq!(bash(r#"typeset -a a=([2]=q); printf '%s' "${a[2]}""#), "q");
    assert_eq!(
        bash(r#"declare -a a=([5]=x [10]=y); declare -p a"#).trim_end(),
        r#"declare -a a=([5]="x" [10]="y")"#
    );
}

#[test]
fn bash_case_and_at_transforms() {
    // bash string transforms with no zsh equivalent, gated to --bash. Ground
    // truth is bash 5.x. `${v~~}`/`${v~}` toggle case; `${v@U/@L/@u}` upper-all/
    // lower-all/upper-first; `${v@Q}` shell-quotes (always single-quoted).
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert_eq!(bash(r#"s=HeLLo; printf '%s' "${s~~}""#), "hEllO");
    assert_eq!(bash(r#"s=HeLLo; printf '%s' "${s~}""#), "heLLo");
    assert_eq!(bash(r#"v=Hello; printf '%s' "${v@U}""#), "HELLO");
    assert_eq!(bash(r#"v=Hello; printf '%s' "${v@L}""#), "hello");
    assert_eq!(bash(r#"v=hello; printf '%s' "${v@u}""#), "Hello");
    assert_eq!(bash(r#"v=abc; printf '%s' "${v@Q}""#), "'abc'");
    assert_eq!(bash(r#"v="a b"; printf '%s' "${v@Q}""#), "'a b'");
    assert_eq!(bash(r#"v="it's"; printf '%s' "${v@Q}""#), r#"'it'\''s'"#);
    assert_eq!(bash(r#"v=; printf '%s' "${v@Q}""#), "''");
    // Case-mod with a single-char PATTERN: only matching chars transform.
    assert_eq!(
        bash(r#"v=hello_world; printf '%s' "${v^^[hw]}""#),
        "Hello_World"
    );
    assert_eq!(bash(r#"v=HELLO; printf '%s' "${v,,[HE]}""#), "heLLO");
    assert_eq!(bash(r#"v=abcABC; printf '%s' "${v^^[a-c]}""#), "ABCABC");
    // `${v^PAT}` upper-cases the first char only if it matches the pattern.
    assert_eq!(bash(r#"v=hello; printf '%s' "${v^h}""#), "Hello");
    assert_eq!(bash(r#"v=hello; printf '%s' "${v^l}""#), "hello");
    // `${v@a}` — attribute-flag letters (empty for a plain var), bash order.
    assert_eq!(bash(r#"x=5; printf '[%s]' "${x@a}""#), "[]");
    assert_eq!(bash(r#"declare -r r=1; printf '%s' "${r@a}""#), "r");
    assert_eq!(bash(r#"declare -i n=2; printf '%s' "${n@a}""#), "i");
    assert_eq!(bash(r#"declare -a a=(1); printf '%s' "${a@a}""#), "a");
    assert_eq!(bash(r#"declare -A m=([k]=v); printf '%s' "${m@a}""#), "A");
    assert_eq!(bash(r#"declare -rix v=5; printf '%s' "${v@a}""#), "irx");
    // `${v@E}` expands ANSI-C backslash escapes like $'…'.
    assert_eq!(bash(r#"v="x\ty"; printf '%s' "${v@E}""#), "x\ty");
    assert_eq!(bash(r#"v="a\x41b"; printf '%s' "${v@E}""#), "aAb");

    // `@`/`~` transforms are a bad substitution under --zsh (rc 1, no output).
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", r#"v=Hi; echo "${v@U}""#])
        .output()
        .expect("spawn");
    assert!(
        !out.status.success(),
        "--zsh: ${{v@U}} must be bad substitution"
    );
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
}

#[test]
fn bash_substring_negative_offset_underflow() {
    // Divergence point (not corpus-eligible: reference shells DISAGREE).
    // bash empties the substring when a negative offset underflows past the
    // start of the value; zsh AND ksh93 clamp the offset to 0 and return the
    // whole value. `${v: -10}` on "hello" → "" in bash, "hello" in zsh/ksh93.
    // Ground truth is real bash 5.x / real ksh93. Found by gen_param_fuzz.
    let mode = |m: &str, script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args([m, "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // --bash: underflow → empty.
    assert_eq!(
        mode("--bash", r#"v=hello; printf '[%s]' "${v: -10}""#),
        "[]"
    );
    assert_eq!(mode("--bash", r#"v=x; printf '[%s]' "${v: -3}""#), "[]");
    assert_eq!(mode("--bash", r#"v=abc; printf '[%s]' "${v: -5:2}""#), "[]");
    assert_eq!(
        mode("--bash", r#"a=(1 2 3); printf '[%s]' "${a[@]: -5}""#),
        "[]"
    );
    assert_eq!(
        mode("--bash", r#"a=(1 2 3); printf '[%s]' "${a[@]: -5:2}""#),
        "[]"
    );
    // --bash: in-range negative offset still works normally.
    assert_eq!(
        mode("--bash", r#"v=hello; printf '[%s]' "${v: -5}""#),
        "[hello]"
    );
    assert_eq!(
        mode("--bash", r#"v=hello; printf '[%s]' "${v: -3}""#),
        "[llo]"
    );
    // printf recycles its format per word, so a 2-word slice → [2][3].
    assert_eq!(
        mode("--bash", r#"a=(1 2 3); printf '[%s]' "${a[@]: -2}""#),
        "[2][3]"
    );
    // --zsh AND --ksh: clamp to 0 (whole value / array), no underflow-emptying.
    for m in ["--zsh", "--ksh"] {
        assert_eq!(
            mode(m, r#"v=hello; printf '[%s]' "${v: -10}""#),
            "[hello]",
            "{m}"
        );
        assert_eq!(
            mode(m, r#"a=(1 2 3); printf '[%s]' "${a[@]: -5}""#),
            "[1][2][3]",
            "{m}"
        );
    }
}

#[test]
fn pattern_operand_not_word_split_under_shwordsplit() {
    // A `#`/`%`/`/` pattern taken from an unquoted $var must be used whole —
    // never IFS-word-split — even when SH_WORD_SPLIT is active (bash/ksh
    // default, and zsh under `setopt shwordsplit`). The singsub path (C's
    // PREFORK_SINGLE) suppresses word splitting; a missing SINGLE gate made the
    // pattern collapse to its first word under --bash/--ksh. Ground truth: all
    // reference shells agree. Found by gen_param_fuzz2 (multi-mode).
    let mode = |m: &str, script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args([m, "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    for m in ["--zsh", "--bash", "--ksh"] {
        // strip with spaced pattern from $var
        assert_eq!(
            mode(m, r#"v='a b c'; w='a b'; printf '[%s]' "${v#$w}""#),
            "[ c]",
            "{m}"
        );
        assert_eq!(
            mode(m, r#"v='x y z'; w='y z'; printf '[%s]' "${v%$w}""#),
            "[x ]",
            "{m}"
        );
        assert_eq!(
            mode(m, r#"v='hi there'; w='hi there'; printf '[%s]' "${v#$w}""#),
            "[]",
            "{m}"
        );
        // replace with spaced pattern from $var
        assert_eq!(
            mode(m, r#"v='a b c'; w='a b'; printf '[%s]' "${v/$w/X}""#),
            "[X c]",
            "{m}"
        );
        assert_eq!(
            mode(m, r#"v='a b c'; w='b c'; printf '[%s]' "${v//$w/Y}""#),
            "[a Y]",
            "{m}"
        );
    }
    // The fix must NOT disable ordinary word-splitting of a bare unquoted $var
    // in bash/ksh (SH_WORD_SPLIT on): `$w` in command/arg position still splits.
    assert_eq!(
        mode("--bash", r#"w='a b c'; printf '<%s>' $w"#),
        "<a><b><c>"
    );
    assert_eq!(mode("--ksh", r#"w='a b c'; printf '<%s>' $w"#), "<a><b><c>");
    // ...and a scalar-assignment RHS (also singsub) must keep the value intact.
    assert_eq!(
        mode("--bash", r#"w='a b c'; v=$w; printf '[%s]' "$v""#),
        "[a b c]"
    );
}

#[test]
fn bash_nocasematch_and_read_n() {
    // Two more --bash-only behaviors:
    //  * `shopt -s nocasematch` → case-insensitive `[[ == ]]` / `[[ != ]]`.
    //  * `read -n N` reads at most N chars from stdin (zsh's -n is a no-op).
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    // nocasematch
    assert_eq!(
        bash("shopt -s nocasematch; [[ HELLO == hello ]] && echo ci"),
        "ci"
    );
    assert_eq!(
        bash("shopt -s nocasematch; [[ Hello == h* ]] && echo m"),
        "m"
    );
    assert_eq!(
        bash("shopt -s nocasematch; [[ ABC != abc ]] && echo ne || echo eq"),
        "eq"
    );
    // Still case-sensitive without the shopt, and after unsetting it.
    assert_eq!(bash("[[ HELLO == hello ]] && echo ci || echo cs"), "cs");
    assert_eq!(
        bash("shopt -s nocasematch; shopt -u nocasematch; [[ AB == ab ]] && echo ci || echo cs"),
        "cs"
    );
    // read -n
    assert_eq!(bash(r#"read -n 3 x <<< "abcdef"; echo "$x""#), "abc");
    assert_eq!(bash(r#"read -n 10 x <<< "short"; echo "$x""#), "short");
    assert_eq!(bash(r#"read -n 2 a b <<< "xy"; echo "$a-$b""#), "xy-");

    // --zsh: nocasematch is inert (case-sensitive), `read -n` is a boolean
    // no-op reading the whole line (matching real zsh).
    let zsh = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--zsh", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    assert_eq!(zsh("[[ HELLO == hello ]] && echo ci || echo cs"), "cs");
    assert_eq!(
        zsh(r#"read -n foo <<< "hi there"; echo "[$foo]""#),
        "[hi there]"
    );
}

#[test]
fn bash_type_t_query() {
    // bash `type -t NAME` prints a single word: alias / keyword / function /
    // builtin / file, or nothing (exit 1) if unknown. zsh's `type` has no -t.
    let bash = |script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        (
            String::from_utf8_lossy(&out.stdout).trim_end().to_owned(),
            out.status.success(),
        )
    };
    assert_eq!(bash("f() { :; }; type -t f").0, "function");
    assert_eq!(bash("type -t echo").0, "builtin");
    assert_eq!(bash("type -t if").0, "keyword");
    assert_eq!(bash("type -t while").0, "keyword");
    assert_eq!(bash("type -t ls").0, "file");
    // Unknown name → empty output, exit 1.
    let (out, ok) = bash("type -t definitely_not_a_command_xyz");
    assert_eq!(out, "");
    assert!(!ok);
    // Precedence: a function shadowing a builtin name reports "function".
    assert_eq!(bash("true() { :; }; type -t true").0, "function");

    // --zsh: `-t` is not a zsh `type` flag — the query path stays off, so the
    // normal verbose `type` output is produced (not a one-word type).
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "f(){ :; }; type f"])
        .output()
        .expect("spawn");
    assert!(String::from_utf8_lossy(&out.stdout).contains("function"));
}

#[test]
fn bash_arith_subscript_and_assoc_keys() {
    // bash indexed arrays are 0-based in ARITHMETIC too (`$(( a[1] ))` is the
    // second element), and `${!m[@]}` on an associative array yields its KEYS.
    // Both were zsh-shaped before (1-based arith; empty assoc-key indirect).
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    // 0-based arithmetic subscript, read + compound.
    assert_eq!(bash("a=(10 20 30); echo $(( a[1] ))"), "20");
    assert_eq!(bash("a=(10 20 30); echo $(( a[0] + a[2] ))"), "40");
    assert_eq!(bash("a=(10 20 30); echo $(( a[-1] ))"), "30");
    assert_eq!(bash("a=(1 2 3); i=1; echo $(( a[i] ))"), "2");
    assert_eq!(bash(r#"a=(10 20 30); (( a[0]++ )); echo "${a[0]}""#), "11");
    // `${!m[@]}` = assoc keys; order is hash-dependent, so exercise it
    // functionally (sum every value via its key) — order-independent.
    assert_eq!(
        bash(
            r#"declare -A m=([a]=1 [b]=2 [c]=3); s=0; for k in "${!m[@]}"; do s=$((s+m[$k])); done; echo $s"#
        ),
        "6"
    );
    // The key SET is correct (sorted for determinism).
    assert_eq!(
        bash(
            r#"declare -A m=([x]=1 [y]=2 [z]=3); for k in "${!m[@]}"; do echo "$k"; done | sort | tr -d '\n'"#
        ),
        "xyz"
    );

    // --zsh keeps 1-based arithmetic subscripts (matching real zsh).
    let zsh = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--zsh", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    assert_eq!(zsh("a=(10 20 30); echo $(( a[1] ))"), "10");
}

#[test]
fn bash_set_o_listing() {
    // bash `set -o` lists a fixed ~27 named options as `name<TAB>on/off`;
    // `set +o` uses the reusable `set -o name` / `set +o name` form. zsh lists
    // its full ~180-option set with different naming, so --bash gets the bash
    // table instead.
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // Parse `set -o` into (name, on/off) pairs (name is padded, then a TAB).
    let state_of = |listing: &str, opt: &str| -> Option<String> {
        listing.lines().find_map(|l| {
            let (n, v) = l.split_once('\t')?;
            (n.trim() == opt).then(|| v.trim().to_string())
        })
    };
    let listing = bash("set -o");
    assert_eq!(state_of(&listing, "braceexpand").as_deref(), Some("on"));
    assert_eq!(state_of(&listing, "errexit").as_deref(), Some("off"));
    assert_eq!(state_of(&listing, "pipefail").as_deref(), Some("off"));
    // Exactly the 27 bash options (each line has a tab-separated on/off).
    let count = listing.lines().filter(|l| l.contains('\t')).count();
    assert_eq!(count, 27, "bash set -o should list 27 options");
    // Enabling flips the state.
    assert_eq!(
        state_of(&bash("set -o errexit; set -o"), "errexit").as_deref(),
        Some("on")
    );
    // `set +o` reusable form.
    assert!(bash("set +o").contains("set +o allexport"));
    assert!(bash("set -o pipefail; set +o").contains("set -o pipefail"));

    // --zsh keeps the full zsh option listing (uses `no`-prefixed names).
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "set -o"])
        .output()
        .expect("spawn");
    let zout = String::from_utf8_lossy(&out.stdout);
    assert!(zout.lines().count() > 50, "zsh set -o lists many options");
}

#[test]
fn bash_declare_p_format() {
    // bash `declare -p` uses the reusable `declare -FLAGS name="value"` form
    // (scalar) / `declare -a name=([i]="v" …)` / `declare -A name=([k]="v" …)`,
    // not zsh's `typeset`. Values are backslash-escaped inside the quotes.
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    assert_eq!(bash("x=5; declare -p x"), r#"declare -- x="5""#);
    assert_eq!(bash("declare -i n=7; declare -p n"), r#"declare -i n="7""#);
    assert_eq!(
        bash("declare -rx e=hi; declare -p e"),
        r#"declare -rx e="hi""#
    );
    assert_eq!(
        bash("a=(one two three); declare -p a"),
        r#"declare -a a=([0]="one" [1]="two" [2]="three")"#
    );
    assert_eq!(
        bash("declare -A m=([k1]=v1); declare -p m"),
        r#"declare -A m=([k1]="v1" )"#
    );
    // Sparse indexed array lists only the live indices.
    assert_eq!(
        bash("a=(x y z); a[5]=q; declare -p a"),
        r#"declare -a a=([0]="x" [1]="y" [2]="z" [5]="q")"#
    );
    // Special chars inside "…" are backslash-escaped.
    assert_eq!(
        bash(r#"v="a\"b\$c"; declare -p v"#),
        r#"declare -- v="a\"b\$c""#
    );

    // --zsh keeps the zsh `typeset` form.
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "x=5; typeset -p x"])
        .output()
        .expect("spawn");
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("typeset"));
}

#[test]
fn bash_shopt_q() {
    // bash `shopt -q OPT` is quiet — no output, exit 0 iff every named option
    // is set, else 1. Previously `-q` was mistaken for an option name.
    let run = |script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        (
            String::from_utf8_lossy(&out.stdout).trim_end().to_owned(),
            out.status.success(),
        )
    };
    // Unset option: quiet, non-zero, no output.
    let (out, ok) = run("shopt -q nullglob");
    assert_eq!(out, "");
    assert!(!ok);
    // After enabling, quiet + success.
    let (out, ok) = run("shopt -s nullglob; shopt -q nullglob");
    assert_eq!(out, "");
    assert!(ok);
    // Multiple options: success only if ALL are set.
    assert!(!run("shopt -s nullglob; shopt -q nullglob extglob").1);
    assert!(run("shopt -s nullglob extglob; shopt -q nullglob extglob").1);
}

#[test]
fn bash_extglob() {
    // bash `shopt -s extglob` enables the ksh-style extended patterns
    // `@()`/`*()/+()/?()/!()` — mapped to zsh's `kshglob`, which supports the
    // identical syntax. Works in `[[ ]]`, `case`, and parameter expansion.
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    assert_eq!(
        bash("shopt -s extglob; [[ abc == @(abc|xyz) ]] && echo m"),
        "m"
    );
    assert_eq!(bash("shopt -s extglob; [[ aaa == +(a) ]] && echo p"), "p");
    assert_eq!(
        bash("shopt -s extglob; [[ color == colo?(u)r ]] && echo o"),
        "o"
    );
    assert_eq!(bash("shopt -s extglob; [[ foo == !(bar) ]] && echo n"), "n");
    assert_eq!(
        bash(r#"shopt -s extglob; v="  trim  "; echo "[${v##+([[:space:]])}]""#),
        "[trim  ]"
    );
    // shopt -q tracks it.
    assert_eq!(bash("shopt -q extglob && echo on || echo off"), "off");
    assert_eq!(bash("shopt -s extglob; shopt -q extglob && echo on"), "on");
}

#[test]
fn bash_printf_time_format() {
    // bash/ksh `printf '%(FMT)T' TS` renders TS (epoch seconds) via strftime;
    // a negative or missing TS means "now". zsh's printf lacks this directive.
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    // Fixed timestamps in UTC are deterministic across machines.
    assert_eq!(bash(r#"TZ=UTC printf '%(%Y-%m-%d)T\n' 0"#), "1970-01-01");
    assert_eq!(
        bash(r#"TZ=UTC printf '%(%Y-%m-%dT%H:%M:%S)T\n' 1000000000"#),
        "2001-09-09T01:46:40"
    );
    // Field width applies to the rendered string.
    assert_eq!(bash(r#"TZ=UTC printf '[%10(%Y)T]\n' 0"#), "[      1970]");
    // A missing timestamp uses the current time — assert only its shape.
    let now_year = bash(r#"printf '%(%Y)T\n'"#);
    assert!(now_year.len() == 4 && now_year.chars().all(|c| c.is_ascii_digit()));

    // --zsh has no `%(...)T` directive — it must stay an invalid directive.
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", r#"printf '%(%Y)T\n' 0"#])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
}

#[test]
fn bash_prefix_name_matching() {
    // bash `${!prefix@}` / `${!prefix*}` list the NAMES of set variables whose
    // name starts with `prefix` (sorted), excluding zsh-internal magic params
    // (aliases / argv / functions / …) that bash has no equivalent for.
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    // `@` splats separate words → sort them for a determinstic assertion.
    assert_eq!(
        bash(r#"aa=1; ab=2; ba=3; for v in "${!a@}"; do echo "$v"; done | sort | tr '\n' ' '"#),
        "aa ab"
    );
    assert_eq!(bash(r#"zqx1=1; zqx2=2; echo "${!zqx@}""#), "zqx1 zqx2");
    assert_eq!(bash(r#"myvar=9; echo "${!myv*}""#), "myvar");
    // No zsh magic params leak in (aliases/argv start with 'a').
    assert_eq!(
        bash(r#"for v in "${!a@}"; do echo "$v"; done | grep -c -E '^(aliases|argv)$'"#),
        "0"
    );

    // Indirect (`${!x}`) and array indices (`${!a[@]}`) — same `!` syntax —
    // must keep working.
    assert_eq!(bash(r#"x=y; y=hi; echo "${!x}""#), "hi");
    assert_eq!(bash(r#"a=(p q r); echo "${!a[@]}""#), "0 1 2");
}

#[test]
fn bash_replacement_backslash_strip() {
    // bash strips a source-literal backslash before ANY char in a
    // `${var/pat/repl}` replacement (`\~`→`~`, `\&`→`&`, `\\`→`\`) — but NOT
    // from an EXPANDED value, and `\$` still defangs expansion. zsh keeps the
    // literal backslash, so this is gated to --bash.
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    assert_eq!(bash(r#"v=abc; echo "${v/b/\~}""#), "a~c");
    assert_eq!(bash(r#"v=abc; echo "${v//b/\x}""#), "axc");
    assert_eq!(bash(r#"v=abc; echo "${v/b/\&}""#), "a&c");
    assert_eq!(bash(r#"v=abc; echo "${v/b/x\\y}""#), r#"ax\yc"#); // \\ → one \
                                                                  // Not stripped from a spliced value; \$ defangs expansion.
    assert_eq!(bash(r#"v=abc; x="\~"; echo "${v/b/$x}""#), r#"a\~c"#);
    assert_eq!(bash(r#"v=abc; echo "${v/b/\$x}""#), "a$xc");
    // Ordinary replacements are unaffected.
    assert_eq!(bash(r#"v=aXbXcX; echo "${v//X/-}""#), "a-b-c-");

    // --zsh keeps the literal backslash (matching real zsh).
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", r#"v=abc; echo "${v/b/\~}""#])
        .output()
        .expect("spawn");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), r#"a\~c"#);
}

#[test]
fn bash_special_variables() {
    // bash special vars that alias zsh natives (or are synthesized) under
    // --bash: PIPESTATUS≈pipestatus, FUNCNAME≈funcstack, BASH_VERSINFO/
    // BASH_VERSION synthesized. Values are deterministic given the script, so
    // no reference binary is needed (ground truth verified against bash 5.x).
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    // PIPESTATUS — per-stage exit codes, 0-indexed, all subscript forms.
    assert_eq!(
        bash(r#"true | false | true; echo "${PIPESTATUS[@]}""#),
        "0 1 0"
    );
    assert_eq!(
        bash(r#"true | false | true; echo "${PIPESTATUS[*]}""#),
        "0 1 0"
    );
    assert_eq!(
        bash(r#"true | false; echo "${PIPESTATUS[0]}${PIPESTATUS[1]}""#),
        "01"
    );
    assert_eq!(
        bash(r#"true | false | true; echo "${#PIPESTATUS[@]}""#),
        "3"
    );
    // FUNCNAME — call stack, innermost (current) first; nested frames.
    assert_eq!(bash(r#"f() { echo "${FUNCNAME[0]}"; }; f"#), "f");
    assert_eq!(
        bash(r#"g(){ f(){ echo "${FUNCNAME[@]}"; }; f; }; g"#),
        "f g"
    );
    // BASH_VERSINFO — 6-element array, first element numeric & >= 4.
    assert_eq!(bash(r#"echo "${#BASH_VERSINFO[@]}""#), "6");
    assert_eq!(
        bash(r#"[[ ${BASH_VERSINFO[0]} =~ ^[0-9]+$ ]] && echo num"#),
        "num"
    );
    assert_eq!(
        bash(r#"[[ ${BASH_VERSINFO[0]} -ge 4 ]] && echo modern"#),
        "modern"
    );
    // BASH_VERSION — non-empty `X.Y.Z(...)-release` shape.
    assert!(bash(r#"echo "$BASH_VERSION""#).contains("-release"));
    // `declare -p` of the synthesized specials produces the bash array form.
    assert_eq!(
        bash(r#"true|false; declare -p PIPESTATUS"#),
        r#"declare -a PIPESTATUS=([0]="0" [1]="1")"#
    );
    assert_eq!(
        bash(r#"f(){ declare -p FUNCNAME; }; f"#),
        r#"declare -a FUNCNAME=([0]="f")"#
    );
    assert!(bash(r#"declare -p BASH_VERSINFO"#).starts_with("declare -ar BASH_VERSINFO=(["));

    // --zsh must NOT expose the bash names (empty), but zsh natives still work.
    let zsh = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--zsh", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    assert_eq!(
        zsh(r#"true|false; echo "[${PIPESTATUS[0]}][$BASH_VERSION]""#),
        "[][]"
    );
    assert_eq!(zsh(r#"true|false|true; echo "${pipestatus[@]}""#), "0 1 0");
}

#[test]
fn bash_alpha_brace_step() {
    // bash supports an alphabetic brace-range STEP (`{a..e..2}` → a c e); zsh
    // does not (emits the literal), so it is gated to --bash. Direction is
    // taken from the endpoints; the step's sign is ignored for the alpha form.
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    assert_eq!(bash("echo {a..e..2}"), "a c e");
    assert_eq!(bash("echo {e..a..2}"), "e c a");
    assert_eq!(bash("echo {a..e..-2}"), "a c e");
    assert_eq!(bash("echo {z..a..5}"), "z u p k f a");
    assert_eq!(bash("echo {A..E..2}"), "A C E");
    // Numeric step is shared with zsh and stays correct.
    assert_eq!(bash("echo {1..10..3}"), "1 4 7 10");

    // --zsh keeps the alpha-step literal (matching real zsh), numeric works.
    let zsh = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--zsh", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    assert_eq!(zsh("echo {a..e..2}"), "{a..e..2}");
    assert_eq!(zsh("echo {1..10..3}"), "1 4 7 10");
}

#[test]
fn test_bracket_posix_three_arg_binary_rule() {
    // POSIX `test`/`[` 3-argument rule: with exactly three operands and a
    // BINARY operator in the middle, it is a binary test of $1 and $3 — even
    // when $1 looks like an operator (`!`). `[ "!" = "x" ]` compares the
    // strings, it is NOT a negation. Universal across zsh/bash/ksh/dash/sh, so
    // every zshrs mode must agree. A prior port errored (rc 2) on all of these.
    // Found by the test/[ operator fuzzer.
    let sig = |flag: &str, script: &str| -> bool {
        Command::new(zshrs_bin())
            .args([flag, "-f", "-c", script])
            .output()
            .expect("spawn")
            .status
            .success()
    };
    // (script, expected-success) — true == exit 0.
    let cases: &[(&str, bool)] = &[
        ("[ \"!\" = \"=\" ]", false), // "!" != "="  → false
        ("[ \"!\" != \"=\" ]", true), // "!" != "="  → true
        ("[ \"!\" = \"!\" ]", true),  // "!" == "!"  → true
        ("[ \"!\" != \"a\" ]", true), // "!" != "a"  → true
        // negation forms still work (4-arg ! strips; 3-arg unary-middle negates)
        ("[ ! -z foo ]", true), // ! (-z foo) → ! false → true
        ("[ ! a = b ]", true),  // ! (a = b)  → ! false → true
        ("[ ! a ]", false),     // ! (-n a)   → ! true  → false
        // ordinary binary + unary unaffected
        ("[ a = b ]", false),
        ("[ 5 -eq 5 ]", true),
        ("[ -n x ]", true),
    ];
    for m in ["--zsh", "--bash", "--ksh", "--dash", "--sh"] {
        for (script, want) in cases {
            assert_eq!(
                sig(m, script),
                *want,
                "{m}: {script} expected success={want}"
            );
        }
    }
}

#[test]
fn emulation_parity_matrix() {
    let require = std::env::var("ZSHRS_REQUIRE_REF_SHELLS").is_ok();
    let mut tested = 0usize;
    let mut missing: Vec<&str> = Vec::new();
    let mut mismatches: Vec<String> = Vec::new();

    for case in PARITY_CASES {
        let Some(refbin) = find_shell(case.candidates) else {
            // Optional cases (ash/mksh) are best-effort: absence never counts
            // toward the enforced-missing list.
            if !case.optional {
                missing.push(case.name);
            }
            eprintln!(
                "skip: `{}` reference not found{}",
                case.name,
                if case.optional { " (optional)" } else { "" }
            );
            continue;
        };
        tested += 1;
        let emu = case
            .ref_emulate
            .map(|e| format!(" [emulate {e}]"))
            .unwrap_or_default();
        eprintln!(
            "testing {} : zshrs {} vs {}{} (extended={})",
            case.name,
            case.zshrs_flags.join(" "),
            refbin,
            emu,
            case.extended
        );

        // The portable corpus runs for every case; the extended corpus only
        // for cases whose reference has arrays / [[ / (( )) / brace expansion.
        let corpus =
            PORTABLE_CORPUS
                .iter()
                .chain(if case.extended { EXTENDED_CORPUS } else { &[] });
        for script in corpus {
            let ((r_out, r_ok), (z_out, z_ok)) = run_case(case, &refbin, script);
            if r_out != z_out || r_ok != z_ok {
                mismatches.push(format!(
                    "  [{}] {script:?}\n    ref: ok={r_ok} out={r_out:?}\n    zrs: ok={z_ok} out={z_out:?}",
                    case.name
                ));
            }
        }
    }

    eprintln!(
        "emulation parity: tested {tested} way(s), {} missing",
        missing.len()
    );

    assert!(
        mismatches.is_empty(),
        "emulation parity diverged on {} case(s):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );

    if missing_is_fatal(require, &missing) {
        panic!(
            "ZSHRS_REQUIRE_REF_SHELLS is set but these reference ways were absent: {missing:?}. \
             Install them so the parity contract is enforced, not skipped."
        );
    }
    assert!(
        tested > 0,
        "no reference shells available at all — cannot verify parity"
    );
}

#[test]
fn bash_set_o_accepts_bash_only_option_names() {
    // `set -o NAME` for bash's own option names. Eight of them reached zsh's
    // faithful `optlookup`/`dosetopt` and failed there — six because zsh has no
    // such option at all (`errtrace`, `functrace`, `history`, `keyword`,
    // `nolog`, `posix` → "no such option"), two because zsh refuses to change a
    // startup-only option after startup (`monitor`, `onecmd` →
    // "can't change option", Src/options.c:746). Real bash returns 0 for every
    // one of them, and the state must round-trip through the `set -o` listing
    // and `$SHELLOPTS`.
    let bash = |script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    };
    for opt in [
        "posix",
        "errtrace",
        "functrace",
        "history",
        "keyword",
        "nolog",
        "monitor",
        "onecmd",
        "interactive-comments",
    ] {
        let (_, ok) = bash(&format!("set -o {opt}"));
        assert!(ok, "bash `set -o {opt}` must succeed");
        let (_, ok) = bash(&format!("set +o {opt}"));
        assert!(ok, "bash `set +o {opt}` must succeed");
    }
    // `set -o onecmd` must NOT truncate the script: real bash runs both
    // commands (`bash -c 'set -o onecmd; echo a; echo b'` prints a and b).
    assert_eq!(bash("set -o onecmd; echo a; echo b").0, "a\nb\n");
    // State round-trips through the listing…
    let state_of = |listing: &str, opt: &str| -> Option<String> {
        listing.lines().find_map(|l| {
            let (n, v) = l.split_once('\t')?;
            (n.trim() == opt).then(|| v.trim().to_string())
        })
    };
    assert_eq!(state_of(&bash("set -o").0, "posix").as_deref(), Some("off"));
    assert_eq!(
        state_of(&bash("set -o posix; set -o").0, "posix").as_deref(),
        Some("on")
    );
    assert_eq!(
        state_of(&bash("set -o posix; set +o posix; set -o").0, "posix").as_deref(),
        Some("off")
    );
    // …and through the reusable `set +o` form.
    assert!(bash("set -o errtrace; set +o").0.contains("set -o errtrace"));

    // $SHELLOPTS is the colon-joined list of the enabled `set -o` options in
    // bash's (alphabetical) table order; it was empty before. These three are
    // on by default in a non-interactive `bash -c`. (Membership, not an exact
    // string: this harness passes `-f`, which in a Bourne-letters mode is
    // NO_GLOB — c:Src/options.c:424 — so `noglob` legitimately joins the list.)
    let opts = bash("echo $SHELLOPTS").0;
    let names: Vec<&str> = opts.trim().split(':').collect();
    for want in ["braceexpand", "hashall", "interactive-comments"] {
        assert!(names.contains(&want), "$SHELLOPTS missing {want}: {opts:?}");
    }
    assert!(!names.contains(&"posix"), "posix is off by default: {opts:?}");
    assert!(
        names.windows(2).all(|w| w[0] < w[1]),
        "$SHELLOPTS must keep bash's alphabetical order: {opts:?}"
    );
    // Toggling a name adds / removes it.
    assert!(bash("set -o posix; echo $SHELLOPTS")
        .0
        .split(':')
        .any(|n| n.trim() == "posix"));
    assert!(!bash("set +o braceexpand; echo $SHELLOPTS")
        .0
        .split(':')
        .any(|n| n.trim() == "braceexpand"));

    // --zsh is untouched: these are not zsh options and must still be rejected.
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "set -o posix"])
        .output()
        .expect("spawn");
    assert!(
        !out.status.success(),
        "--zsh must still reject `set -o posix` (no such zsh option)"
    );
}

#[test]
fn bash_subshell_depth_parameter() {
    // bash BASH_SUBSHELL: 0 at the top level, incremented per subshell.
    // zshrs aliases it to the zsh-native ZSH_SUBSHELL in --bash mode; before
    // that it expanded to the empty string at every depth.
    let bash = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert_eq!(
        // NB: the inner nesting needs a space between the parens — `((` would
        // start an arithmetic command, not a second subshell.
        bash(r#"echo "$BASH_SUBSHELL"; (echo "$BASH_SUBSHELL"); ( ( echo "$BASH_SUBSHELL" ) )"#),
        "0\n1\n2\n"
    );
}

#[test]
fn caller_outside_a_subroutine_fails() {
    // bash(1) `caller`: "The return value is 0 unless the shell is not
    // executing a subroutine call". At the top level bash prints NOTHING and
    // returns non-zero; zshrs printed a synthetic `0 main` frame and returned
    // 0, so the standard `caller || …` guard took the wrong branch.
    let run = |script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    };
    assert_eq!(run("caller").1, false, "top-level `caller` must fail");
    assert_eq!(run("caller").0, "", "top-level `caller` prints nothing");
    // Inside a function it still reports the frame and succeeds.
    let (out, ok) = run("f() { caller; }; f");
    assert!(ok, "`caller` in a function must succeed");
    assert!(out.contains('f'), "frame names the function: {out:?}");
}

#[test]
fn posix_faithful_echo_interprets_escapes() {
    // zsh's `emulate sh` sets BSD_ECHO, so `echo "a\tb"` prints the two
    // characters literally. Every real POSIX `sh` this matrix references does
    // the opposite — macOS `/bin/sh` (xpg_echo-by-default bash) and Linux
    // `/bin/sh` (dash) both emit a TAB — so the posix-faithful drop-in modes
    // must interpret. `--bash` is excluded (bash's own `echo` needs `-e`), and
    // the zsh-STYLE leg keeps zsh's behavior because its reference IS zsh.
    let run = |flags: &[&str], script: &str| -> String {
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(["-f", "-c", script]);
        let out = Command::new(zshrs_bin()).args(&args).output().expect("spawn");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    for mode in [["--sh"], ["--dash"], ["--ash"], ["--ksh"]] {
        assert_eq!(
            run(&mode, r#"echo "a\tb""#),
            "a\tb\n",
            "{mode:?} echo must interpret escapes like the real shell"
        );
    }
    // bash keeps escapes literal without -e, and honours -e.
    assert_eq!(run(&["--bash"], r#"echo "a\tb""#), "a\\tb\n");
    assert_eq!(run(&["--bash"], r#"echo -e "a\tb""#), "a\tb\n");
    // zsh-STYLE sh: zsh's BSD_ECHO wins (reference is `zsh -c 'emulate sh'`).
    assert_eq!(run(&["--sh", "--zsh"], r#"echo "a\tb""#), "a\\tb\n");
}

#[test]
fn zsh_style_legs_install_a_non_fully_emulation() {
    // `--sh --zsh` / `--ksh --zsh` mean "a zsh that ran `emulate X`", which is
    // exactly how the parity matrix references them. zshrs installed that
    // emulation the way a DROP-IN shell does — early (before the option letters
    // were read) and FULLY (c:Src/init.c:361 `emulate(nam, 1, …)`). The
    // `emulate` builtin passes `fully` only for `-R`, so a plain `emulate X`
    // resets only the OPT_EMULATE options (c:Src/options.c:516). Two
    // consequences, both fixed by deferring the call to the end of startup and
    // dropping `fully`:
    //   * options outside OPT_EMULATE kept zsh's defaults in the reference but
    //     were reset here — `$options[banghist]` off vs zsh's on,
    //     `$options[promptsubst]` on vs zsh's off, `setopt` listing 2 lines
    //     against zsh's 8.
    //   * `-f` was resolved against `kshletters` (`-f` ↔ NO_GLOB,
    //     c:Src/options.c:424) because the emulation had already switched the
    //     letter table, where zsh resolves it against `zshletters` (`-f` ↔
    //     NO_RCS, c:Src/options.c:346) — the leg ran with globbing off.
    let Some(zsh) = find_shell(ZSH) else {
        eprintln!("skip: no zsh reference");
        return;
    };
    for (flag, emu) in [("--sh", "sh"), ("--ksh", "ksh")] {
        for probe in [
            "setopt",
            r#"echo "$- ${options[glob]} ${options[rcs]} ${options[banghist]} ${options[promptsubst]}""#,
        ] {
            let zout = Command::new(&zsh)
                .args(["-f", "-c", &format!("emulate {emu}\n{probe}")])
                .output()
                .expect("spawn zsh");
            let rout = Command::new(zshrs_bin())
                .args([flag, "--zsh", "-f", "-c", probe])
                .output()
                .expect("spawn zshrs");
            assert_eq!(
                String::from_utf8_lossy(&rout.stdout),
                String::from_utf8_lossy(&zout.stdout),
                "{flag} --zsh vs `emulate {emu}` diverged on {probe:?}"
            );
        }
    }
}

#[test]
fn zsh_style_legs_reject_parenthesised_cond_patterns() {
    // Under SH_GLOB — set by both `emulate sh` and `emulate ksh` — a `(` in a
    // `[[ ]]` operand is NOT a pattern group: word-initial it lexes as INPAR
    // (c:Src/lex.c:821) and mid-word it ends the token (c:Src/lex.c:1084), so
    // real zsh reports a parse error. zshrs carried two Rust-only `incond > 1`
    // exceptions that made these parse, so `[[ file.txt == *.(txt|md) ]]`
    // printed 1 where the oracle exited 1 with no output. The exceptions now
    // survive only for the REAL-SHELL drop-in modes, whose reference (bash /
    // ksh93) really does accept `[[ ab =~ (a)(b) ]]`.
    //
    // The oracle must parse the script UNDER the emulation, so it uses the
    // `emulate X -c "$src"` one-shot form: a plain `emulate X\nscript` -c
    // string is parsed by zsh in full BEFORE the `emulate` line runs, so
    // parse-time deltas like SH_GLOB never reach it.
    let Some(zsh) = find_shell(ZSH) else {
        eprintln!("skip: no zsh reference");
        return;
    };
    let oracle = |emu: &str, script: &str| -> (String, bool) {
        let out = Command::new(&zsh)
            .args([
                "-f",
                "-c",
                &format!("__oracle_src=$1; set --; emulate {emu} -c \"$__oracle_src\""),
                "zsh",
                script,
            ])
            .output()
            .expect("spawn zsh");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    };
    let zshrs = |flag: &str, script: &str| -> (String, bool) {
        let out = Command::new(zshrs_bin())
            .args([flag, "--zsh", "-f", "-c", script])
            .output()
            .expect("spawn zshrs");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    };
    for (flag, emu) in [("--sh", "sh"), ("--ksh", "ksh")] {
        for script in [
            "[[ hello == (hel*|wor*) ]]; print -r -- $?", // word-initial `(`
            "[[ ab =~ (a)(b) ]]; print -r -- $?",         // adjacent groups
        ] {
            assert_eq!(
                zshrs(flag, script),
                oracle(emu, script),
                "{flag} --zsh vs `emulate {emu} -c` on {script:?}"
            );
        }
    }
    // The drop-in modes keep the exception: real bash and ksh93 both match.
    for flag in ["--bash", "--ksh"] {
        let out = Command::new(zshrs_bin())
            .args([flag, "-f", "-c", "[[ ab =~ (a)(b) ]]; echo $?"])
            .output()
            .expect("spawn");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "0\n",
            "{flag} must still match `=~ (a)(b)` like the real shell"
        );
    }
}

/// Run `script` under `flags` and return `(stdout, exit_code)`.
fn run_zshrs(flags: &[&str], script: &str) -> (String, i32) {
    let mut args: Vec<&str> = flags.to_vec();
    args.push("-f");
    args.push("-c");
    args.push(script);
    let out = Command::new(zshrs_bin())
        .args(&args)
        .output()
        .expect("spawn zshrs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn quoted_star_with_no_positionals_is_one_empty_word() {
    // c:Src/subst.c:3032 — the QUOTED branch of paramsubst ends in
    // `val = sepjoin(aval, sep, 1)`. Joining an EMPTY list is `""`, so
    // `"$*"` always contributes exactly one word, even with no positional
    // parameters — unlike `"$@"`, which contributes none. Verified against
    // zsh 5.9, bash 5.3, dash and ksh93: all four print `0||7`.
    //
    // zshrs routed `"$*"` through EXPAND_TEXT, whose multsub returns zero
    // nodes for the empty case, so the argument was ELIDED and printf's
    // format recycled one operand early: `0|7|0`.
    for flags in [
        &["--zsh"][..],
        &["--bash"][..],
        &["--ksh"][..],
        &["--dash"][..],
        &["--sh"][..],
    ] {
        assert_eq!(
            run_zshrs(flags, r#"set --; printf '%d|%s|%d\n' $# "$*" 7"#).0,
            "0||7\n",
            "{flags:?}: quoted \"$*\" must stay one (empty) word"
        );
        // `shift` down to zero is the shape the fuzzer found.
        assert_eq!(
            run_zshrs(
                flags,
                r#"set -- a b c; shift 3; printf '%d|%s|%d\n' $# "$*" $?"#
            )
            .0,
            "0||0\n",
            "{flags:?}: \"$*\" after shifting everything off"
        );
        // "$@" genuinely vanishes — the two must NOT be conflated.
        assert_eq!(
            run_zshrs(flags, r#"set --; printf '%d|%s|%d\n' 1 "$@" 7"#).0,
            "1|7|0\n",
            "{flags:?}: quoted \"$@\" must still elide"
        );
        // Non-empty joins keep honoring IFS[0].
        assert_eq!(
            run_zshrs(flags, r#"set -- a b; IFS=-; printf '[%s]' "$*""#).0,
            "[a-b]",
            "{flags:?}: \"$*\" IFS join"
        );
        // Braced `"${*}"` is the same expansion.
        assert_eq!(
            run_zshrs(flags, r#"set --; printf '%d|%s|%d\n' $# "${*}" 7"#).0,
            "0||7\n",
            "{flags:?}: quoted \"${{*}}\" must stay one (empty) word"
        );
    }
}

#[test]
fn set_o_monitor_succeeds_in_posix_family_drop_ins() {
    // bash(1) `set -m`: "Monitor mode. Job control is enabled." POSIX.1-2017
    // XCU `set -m` likewise specifies no error for a script. Measured:
    // `bash|dash|ksh|mksh -c 'set -o monitor; printf "%d\n" $?'` → `0`.
    //
    // zsh refuses when it has no controlling terminal (c:Src/options.c:854
    // `if (SHTTY == -1) return -1;`), which under POSIX_BUILTINS also KILLS
    // the script — so a `set -m` anywhere in a sourced rc file aborted it.
    for flags in [
        &["--bash"][..],
        &["--ksh"][..],
        &["--mksh"][..],
        &["--pdksh"][..],
        &["--dash"][..],
        &["--ash"][..],
        &["--sh"][..],
    ] {
        assert_eq!(
            run_zshrs(
                flags,
                r#"set -o monitor; printf '%d\n' $?; set +o monitor; printf '%d\n' $?"#
            ),
            ("0\n0\n".to_string(), 0),
            "{flags:?}: `set -o monitor` must succeed and not abort"
        );
        // Short form too, and the option must actually READ back as set.
        assert_eq!(
            run_zshrs(flags, r#"set -m; printf '%d\n' $?; set +m; printf '%d\n' $?"#),
            ("0\n0\n".to_string(), 0),
            "{flags:?}: `set -m` must succeed"
        );
    }

    // --zsh keeps zsh's refusal verbatim: `zsh -fc 'set -o monitor'` warns
    // "can't change option: monitor" and exits 1.
    let (stdout, code) = run_zshrs(&["--zsh"], r#"set -o monitor; printf '%d\n' $?"#);
    assert_eq!(stdout, "", "--zsh must not reach the printf");
    assert_eq!(code, 1, "--zsh must keep zsh's exit 1");
}

#[test]
fn korn_drop_ins_have_sparse_arrays() {
    // mksh(1) / ksh(1) arrays are sparse exactly like bash's. Measured:
    // `mksh -c 'a=(x y z); a[5]=q; print -r -- "${!a[@]}"'` → `0 1 2 5`
    // and `${#a[@]}` → `4`. zshrs tracked holes only under --bash, so the
    // Korn drop-ins reported the dense `0 1 2 3 4 5` / `6`.
    for flags in [&["--ksh"][..], &["--mksh"][..], &["--pdksh"][..]] {
        assert_eq!(
            run_zshrs(flags, r#"a=(x y z); a[5]=q; print -r -- "${#a[@]}""#).0,
            "4\n",
            "{flags:?}: sparse ${{#a[@]}}"
        );
        assert_eq!(
            run_zshrs(flags, r#"a=(x y z); a[5]=q; print -r -- "${!a[@]}""#).0,
            "0 1 2 5\n",
            "{flags:?}: sparse ${{!a[@]}}"
        );
        assert_eq!(
            run_zshrs(flags, r#"a=(x y z); a[5]=q; print -r -- "${a[*]}""#).0,
            "x y z q\n",
            "{flags:?}: sparse joined splat skips the holes"
        );
        // A full reassign clears the holes again.
        assert_eq!(
            run_zshrs(flags, r#"a=(x y z); a[5]=q; a=(m n); print -r -- "${#a[@]}""#).0,
            "2\n",
            "{flags:?}: reassign resets to dense"
        );
    }

    // --zsh stays DENSE: zsh arrays are 1-based and pad.
    assert_eq!(
        run_zshrs(&["--zsh"], r#"a=(x y z); a[6]=q; print -r -- "${#a[@]}""#).0,
        "6\n",
        "--zsh must keep dense padding"
    );
}

#[test]
fn bash_case_double_semi_amp_continues_matching() {
    // bash(1), Compound Commands: "Using ;;& in place of ;; causes the shell
    // to test the next pattern list in the statement, if any, and execute any
    // associated list on a successful match." That is zsh's `;|`.
    assert_eq!(
        run_zshrs(&["--bash"], r#"case x in x) printf a;;& x) printf b;; esac"#).0,
        "ab",
        "--bash `;;&` must fall through to the next pattern test"
    );
    // A non-matching second pattern still stops the output at `a`.
    assert_eq!(
        run_zshrs(&["--bash"], r#"case x in x) printf a;;& y) printf b;; esac"#).0,
        "a",
        "--bash `;;&` re-tests but only runs matching arms"
    );
    // Plain `;;` is unchanged.
    assert_eq!(
        run_zshrs(&["--bash"], r#"case x in x) printf a;; x) printf b;; esac"#).0,
        "a",
        "--bash `;;` must still terminate the case"
    );
    // `;&` (unconditional fall-through) is bash's and zsh's alike.
    assert_eq!(
        run_zshrs(&["--bash"], r#"case x in x) printf a;& y) printf b;; esac"#).0,
        "ab",
        "--bash `;&` falls through unconditionally"
    );

    // ksh93 and dash both REJECT `;;&`; so must the drop-ins for them.
    // (`ksh: syntax error at line 1: '&' unexpected`,
    //  `dash: 1: Syntax error: "&" unexpected (expecting word)`.)
    for flags in [&["--ksh"][..], &["--dash"][..], &["--zsh"][..]] {
        let (stdout, code) = run_zshrs(flags, r#"case x in x) printf a;;& x) printf b;; esac"#);
        assert_eq!(stdout, "", "{flags:?}: `;;&` must not run anything");
        assert_ne!(code, 0, "{flags:?}: `;;&` must be a syntax error");
    }
}

#[test]
fn zero_operand_dash_conditions_match_zsh_status() {
    // c:Src/parse.c:2549 — `dble` needs `!s1[2]`, so it is true for ANY
    // two-char `-X` inside `[[ ]]` (n_testargs == 0 short-circuits the
    // strspn) and false for a longer word. c:2586-2592 then splits:
    //   two-char, no operand  → par_cond_multi(s1, newlinklist())
    //                           → COND_MOD with ZERO operands
    //                           → c:Src/cond.c:186-193 warns and `return 2`
    //   longer,   no operand  → par_cond_double(dupstring("-n"), s1)
    //                           → an ordinary non-empty-string test → 0
    // Verified against zsh 5.9: `[[ -n ]]` / `[[ -z ]]` / `[[ -o ]]` exit 2,
    // `[[ -prefix ]]` exits 0.
    for op in ["-n", "-z", "-o", "-f", "-d"] {
        let script = format!("[[ {op} ]]");
        assert_eq!(
            run_zshrs(&["--zsh"], &script).1,
            2,
            "`[[ {op} ]]` must exit 2 (zero-operand COND_MOD)"
        );
    }
    for op in ["-prefix", "-between", "-nonesuch"] {
        let script = format!("[[ {op} ]]");
        assert_eq!(
            run_zshrs(&["--zsh"], &script).1,
            0,
            "`[[ {op} ]]` is `[[ -n \"{op}\" ]]` → true"
        );
    }
    // c:Src/cond.c:177-181 — an ARITY violation on a real conddef is also
    // status 2. `-between` takes exactly 2 operands.
    assert_eq!(
        run_zshrs(&["--zsh"], "[[ -between a ]]").1,
        2,
        "`[[ -between a ]]` must exit 2 (min/max arity check)"
    );
    // Unchanged: the ordinary unary tests with an operand.
    assert_eq!(run_zshrs(&["--zsh"], "[[ -n a ]]").1, 0);
    assert_eq!(run_zshrs(&["--zsh"], "[[ -z a ]]").1, 1);
    assert_eq!(run_zshrs(&["--zsh"], "[[ -5 -lt -3 ]]").1, 0);
}

#[test]
fn unterminated_cond_group_is_a_parse_error() {
    // c:Src/parse.c:2543-2544 — `if (tok != OUTPAR) YYERROR(ecused);`. The
    // macro sets `tok = LEXERR`, which is what par_dinbrack's c:1818
    // `if (tok != DOUTBRACK) YYERRORV(oecused)` then trips on. zsh 5.9:
    // `zsh -fc '[[ ( a ]]'` → "parse error near `]]'", exit 1.
    let (stdout, code) = run_zshrs(&["--zsh"], "[[ ( a ]]");
    assert_eq!(stdout, "", "an unterminated cond group must not evaluate");
    assert_eq!(code, 1, "`[[ ( a ]]` must be a parse error");
    // The balanced form is unaffected.
    assert_eq!(run_zshrs(&["--zsh"], "[[ ( a ) ]]").1, 0);
    assert_eq!(run_zshrs(&["--zsh"], "[[ ( -n a && -n b ) ]]").1, 0);
    assert_eq!(run_zshrs(&["--zsh"], "[[ ( -z a ) ]]").1, 1);
}

#[test]
fn dash_family_reports_two_for_a_fatal_shell_error() {
    // dash's `sh_error()` unwinds through `exraise(EXERROR)`, whose handler
    // sets `exitstatus = 2` before `exitshell()` — so every fatal expansion,
    // assignment and arithmetic error leaves 2, not zsh's
    // `lastval == ERRFLAG_ERROR == 1`. Measured on dash and ash:
    //   dash -c '(set -u; : "$nope") 2>/dev/null; printf "%d\n" $?'  → 2
    //   dash -c '(: "${nope:?msg}")  2>/dev/null; printf "%d\n" $?'  → 2
    //   dash -c '(readonly r=1; r=2) 2>/dev/null; printf "%d\n" $?'  → 2
    //   dash -c '(: $((1/0)))        2>/dev/null; printf "%d\n" $?'  → 2
    // bash, ksh93, mksh and zsh all answer 1 for the same four.
    let fatal = [
        r#"set -u; : "$nope""#,
        r#": "${nope:?msg}""#,
        r#": "${nope:?}""#,
        r#"readonly r=1; r=2"#,
        r#": $((1/0))"#,
        r#": $((1%0))"#,
    ];
    for body in fatal {
        // Inside a subshell — dash's unwind stops at the `( … )` boundary.
        let sub = format!(r#"({body}) 2>/dev/null; printf '%d\n' $?"#);
        for flags in [&["--dash"][..], &["--ash"][..]] {
            assert_eq!(
                run_zshrs(flags, &sub).0,
                "2\n",
                "{flags:?}: `({body})` must report 2"
            );
        }
        // At the top level the unwind reaches the shell's own exit.
        for flags in [&["--dash"][..], &["--ash"][..]] {
            assert_eq!(
                run_zshrs(flags, body).1,
                2,
                "{flags:?}: `{body}` must exit 2"
            );
        }
        // Every other personality keeps 1.
        for flags in [
            &["--zsh"][..],
            &["--bash"][..],
            &["--ksh"][..],
            &["--mksh"][..],
        ] {
            assert_eq!(
                run_zshrs(flags, &sub).0,
                "1\n",
                "{flags:?}: `({body})` must stay at 1"
            );
        }
    }

    // A deliberate `exit N` is NOT an error unwind and keeps its own status.
    assert_eq!(
        run_zshrs(&["--dash"], r#"(exit 5); printf '%d\n' $?"#).0,
        "5\n"
    );
    assert_eq!(
        run_zshrs(&["--dash"], r#"(exit 5; : "${nope:?}") 2>/dev/null; printf '%d\n' $?"#).0,
        "5\n",
        "`exit 5` runs first, so no error is ever raised"
    );
    // …but an error raised BEFORE the exit wins, because exraise assigns
    // exitstatus at the raise. dash agrees: this prints 2.
    assert_eq!(
        run_zshrs(&["--dash"], r#"(: "${nope:?}"; exit 5) 2>/dev/null; printf '%d\n' $?"#).0,
        "2\n"
    );
    // Ordinary non-zero statuses are untouched.
    assert_eq!(run_zshrs(&["--dash"], r#"(false); printf '%d\n' $?"#).0, "1\n");
    assert_eq!(
        run_zshrs(&["--dash"], r#"nonexistent_cmd_zz 2>/dev/null; printf '%d\n' $?"#).0,
        "127\n"
    );
}

#[test]
fn prefix_assignment_persists_for_posix_special_builtins() {
    // POSIX.1-2017 XCU 2.9.1 Simple Commands: "If the command name is a
    // special built-in utility, variable assignments shall affect the
    // current execution environment." c:Src/exec.c:4114-4126 is zsh's
    // implementation — under POSIX_BUILTINS the save/restore is skipped
    // for a shell function or a BINF_PSPECIAL / BINF_ASSIGN builtin unless
    // `command` prefixed it. zshrs pushed the save frame unconditionally,
    // so `v=0; v=1 :` left `v` at 0 in every POSIX-family drop-in.
    //
    // Reference matrix, measured (`v=0; v=1 X; printf '[%s]\n' "$v"`):
    //             `:` (special)  `export` (assign)  `true` (regular)  fn
    //   dash / ash      1               1                  0           0
    //   ksh93           1               1                  0           4
    //   mksh            1               1                  0           0
    //   /bin/sh         1               1                  0           4
    //   bash            0               0                  0           0
    let probe = r#"v=0; v=1 :; printf '[%s]' "$v"; v=2 true; printf '[%s]' "$v"; v=3 export xx; printf '[%s]' "$v"; v=9 command :; printf '[%s]\n' "$v""#;
    for flags in [
        &["--dash"][..],
        &["--ash"][..],
        &["--ksh"][..],
        &["--mksh"][..],
        &["--sh"][..],
    ] {
        assert_eq!(
            run_zshrs(flags, probe).0,
            "[1][1][3][3]\n",
            "{flags:?}: special/assign builtins persist, `true` and `command :` do not"
        );
    }
    // bash only does this under `set -o posix` (bash(1), POSIX Mode:
    // "Assignment statements preceding POSIX special builtins persist in
    // the shell environment after the builtin completes.").
    assert_eq!(
        run_zshrs(&["--bash"], probe).0,
        "[0][0][0][0]\n",
        "--bash without `set -o posix` keeps the save/restore"
    );
    assert_eq!(
        run_zshrs(&["--bash"], &format!("set -o posix; {probe}")).0,
        "[1][1][3][3]\n",
        "--bash with `set -o posix` persists"
    );
    // --zsh has POSIX_BUILTINS off, so nothing persists.
    assert_eq!(
        run_zshrs(&["--zsh"], r#"v=0; v=1 true; printf '[%s]\n' "$v""#).0,
        "[0]\n"
    );

    // The shell-function leg splits: C has one, the Almquist family does not.
    let fn_probe = r#"f() { :; }; v=0; v=4 f; printf '[%s]\n' "$v""#;
    for flags in [&["--ksh"][..], &["--sh"][..]] {
        assert_eq!(
            run_zshrs(flags, fn_probe).0,
            "[4]\n",
            "{flags:?}: ksh93 and bash-as-sh persist across a function call"
        );
    }
    for flags in [&["--dash"][..], &["--ash"][..], &["--bash"][..]] {
        assert_eq!(
            run_zshrs(flags, fn_probe).0,
            "[0]\n",
            "{flags:?}: dash/ash/bash restore across a function call"
        );
    }

    // The assignment must still reach the command's ENVIRONMENT either way.
    for flags in [&["--dash"][..], &["--bash"][..], &["--zsh"][..]] {
        let (stdout, _) = run_zshrs(flags, r#"zzq=1 env"#);
        assert!(
            stdout.lines().any(|l| l == "zzq=1"),
            "{flags:?}: prefix assignment must be exported to the child"
        );
    }
}

#[test]
fn funcnest_overflow_aborts_the_script() {
    // c:Src/exec.c:6060-6063 —
    //     zerr("maximum nested function level reached; increase FUNCNEST?");
    //     lastval = 1;
    //     goto undoshfunc;
    // `zerr` raises errflag, which is what makes the overflow FATAL. zsh 5.9:
    //     zsh -fc 'FUNCNEST=2; f() { f; }; f; printf after'
    // prints the diagnostic and exits 1 with no `after`. bash(1) FUNCNEST
    // agrees: "Function invocations that exceed this nesting level cause the
    // current command to abort."
    //
    // The zshrs stack backstop printed the same message but returned 1
    // WITHOUT the flag, so the script ran on and exited 0.
    for flags in [&["--zsh"][..], &["--bash"][..], &["--ksh"][..]] {
        let (stdout, code) = run_zshrs(
            flags,
            r#"FUNCNEST=2; f() { f; }; f 2>/dev/null; printf 'after\n'"#,
        );
        assert_eq!(stdout, "", "{flags:?}: nothing after the overflow may run");
        assert_eq!(code, 1, "{flags:?}: the shell must exit 1");
    }
    // A finite recursion well inside FUNCNEST is untouched.
    assert_eq!(
        run_zshrs(
            &["--zsh"],
            r#"f() { [ "$1" -le 0 ] && return; f $(( $1 - 1 )); }; f 50; printf 'ok\n'"#
        ),
        ("ok\n".to_string(), 0)
    );
}

#[test]
fn bash_shopt_table_matches_bash() {
    // bash(1), The Shopt Builtin. Three things were wrong:
    //   * the status — "The return status when listing options is zero if
    //     all optnames are enabled, non-zero otherwise"; zshrs always
    //     returned 0, so `shopt -p cdable_vars` said 0 where bash says 1.
    //   * unknown names were accepted and reported `-u`; bash prints
    //     "shopt: NAME: invalid shell option name" and exits 1.
    //   * the plain listing shape is `NAME` padded to 20 then TAB then
    //     on/off, not the `-p` re-inputtable form.
    // The name list, defaults and per-name storage live in
    // dash_mode::BASH_SHOPTS; the twelve rows backed by a real zsh option
    // are seeded with bash's default at --bash startup (zsh's `histappend`
    // is ON, bash's is OFF).
    let sh = |script: &str| run_zshrs(&["--bash"], script);

    // Status rule, both query shapes.
    assert_eq!(sh("shopt -p cdable_vars").1, 1, "unset option → status 1");
    assert_eq!(sh("shopt cdable_vars").1, 1);
    assert_eq!(sh("shopt -q cdable_vars"), (String::new(), 1));
    assert_eq!(sh("shopt -s cdable_vars; shopt -p cdable_vars").1, 0);
    assert_eq!(sh("shopt -p checkwinsize").1, 0, "on-by-default → status 0");
    // All-or-nothing across several names.
    assert_eq!(sh("shopt -q checkwinsize cmdhist").1, 0);
    assert_eq!(sh("shopt -q checkwinsize cdable_vars").1, 1);

    // Output shapes.
    assert_eq!(sh("shopt -p cdable_vars").0, "shopt -u cdable_vars\n");
    assert_eq!(sh("shopt cdable_vars").0, "cdable_vars         \toff\n");
    assert_eq!(sh("shopt checkwinsize").0, "checkwinsize        \ton\n");
    // A name at or past the pad width gets no padding.
    assert_eq!(
        sh("shopt no_empty_cmd_completion").0,
        "no_empty_cmd_completion\toff\n"
    );

    // Unknown name.
    let (stdout, code) = sh("shopt -p zznope");
    assert_eq!(stdout, "", "no state is printed for an unknown name");
    assert_eq!(code, 1);
    assert_eq!(sh("shopt -s zznope").1, 1);

    // Set / unset round-trips, including the three names whose state does
    // not live in a same-named zsh option.
    for name in [
        "extglob",      // → zsh kshglob
        "failglob",     // → zsh nomatch
        "nocasematch",  // → its own flag
        "histappend",   // → zsh histappend, bash default OFF
        "cdable_vars",  // → zsh cdablevars (alias-canonicalised)
        "globskipdots", // → bash-only side table, default ON
    ] {
        assert_eq!(
            sh(&format!("shopt -s {name}; shopt -p {name}")).0,
            format!("shopt -s {name}\n"),
            "{name}: -s must read back as set"
        );
        assert_eq!(
            sh(&format!("shopt -u {name}; shopt -p {name}")).0,
            format!("shopt -u {name}\n"),
            "{name}: -u must read back as unset"
        );
    }

    // failglob is the behavior, not just the flag: bash(1), "failglob: If
    // set, patterns which fail to match filenames during filename expansion
    // result in an expansion error." Measured:
    // `bash -c 'shopt -s failglob; printf "[%s]\n" ./nonexistent_zz*'`
    // prints nothing and exits 1.
    //
    // NOT through `sh` — that helper passes `-f`, which in bash is `set -f`
    // (globbing off), so no expansion happens and the pattern stays literal
    // in bash too (verified: `bash -f -c` prints the literal, exit 0).
    let glob = |script: &str| -> (String, i32) {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-c", script])
            .output()
            .expect("spawn zshrs");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    };
    let (stdout, code) = glob(r#"shopt -s failglob; printf '[%s]\n' ./nonexistent_zz_qq*"#);
    assert_eq!(stdout, "", "failglob must not emit the literal pattern");
    assert_eq!(code, 1);
    // Without it, bash leaves the pattern literal and exits 0.
    assert_eq!(
        glob(r#"printf '[%s]\n' ./nonexistent_zz_qq*"#),
        ("[./nonexistent_zz_qq*]\n".to_string(), 0)
    );
    // nullglob still deletes the word instead.
    assert_eq!(
        glob(r#"shopt -s nullglob; printf '[%s]\n' ./nonexistent_zz_qq*"#),
        ("[]\n".to_string(), 0)
    );

    // `shopt` / `shopt -p` with no names lists all 59 rows.
    assert_eq!(sh("shopt").0.lines().count(), 59);
    assert_eq!(sh("shopt -p").0.lines().count(), 59);
    assert!(sh("shopt -p")
        .0
        .lines()
        .all(|l| l.starts_with("shopt -s ") || l.starts_with("shopt -u ")));
}

#[test]
fn pdksh_line_has_pipestatus_but_ksh93_does_not() {
    // mksh(1): "PIPESTATUS: An array variable holding the exit statuses of
    // the last pipeline." ksh93 has no such parameter. Measured:
    //   mksh -c 'true|false|true; print -r -- "[${PIPESTATUS[*]}]"' → [0 1 0]
    //   ksh  -c  (same)                                             → []
    // `--mksh`/`--pdksh` and `--ksh` install the same emulate-ksh preset, so
    // this needs dash_mode::PDKSH_FAMILY to tell the two lines apart.
    for flags in [&["--mksh"][..], &["--pdksh"][..]] {
        assert_eq!(
            run_zshrs(flags, r#"true | false | true; print -r -- "[${PIPESTATUS[*]}]""#).0,
            "[0 1 0]\n",
            "{flags:?}: PIPESTATUS must carry every stage"
        );
        assert_eq!(
            run_zshrs(flags, r#"(exit 3) | (exit 4); print -r -- "[${PIPESTATUS[*]}]""#).0,
            "[3 4]\n"
        );
        assert_eq!(
            run_zshrs(
                flags,
                r#"true | false; print -r -- "${PIPESTATUS[0]}|${PIPESTATUS[1]}|${#PIPESTATUS[@]}""#
            )
            .0,
            "0|1|2\n",
            "{flags:?}: element and count reads"
        );
    }
    // ksh93 must stay empty.
    assert_eq!(
        run_zshrs(&["--ksh"], r#"true | false | true; print -r -- "[${PIPESTATUS[*]}]""#).0,
        "[]\n",
        "--ksh (ksh93) has no PIPESTATUS"
    );
    // bash keeps it; zsh's own name is `$pipestatus`, and PIPESTATUS is an
    // ordinary (unset) parameter there.
    assert_eq!(
        run_zshrs(&["--bash"], r#"true | false | true; printf '[%s]\n' "${PIPESTATUS[*]}""#).0,
        "[0 1 0]\n"
    );
    assert_eq!(
        run_zshrs(&["--zsh"], r#"true | false | true; print -r -- "[${PIPESTATUS[*]}]""#).0,
        "[]\n"
    );

    // The pdksh line also restores a prefix assignment across a shell-
    // function call where ksh93 keeps it (see
    // prefix_assignment_persists_for_posix_special_builtins).
    let fn_probe = r#"f() { :; }; v=0; v=4 f; print -r -- "[$v]""#;
    assert_eq!(run_zshrs(&["--mksh"], fn_probe).0, "[0]\n");
    assert_eq!(run_zshrs(&["--ksh"], fn_probe).0, "[4]\n");
}

#[test]
fn korn_funsub_and_valsub_run_in_the_current_shell() {
    // ksh(1), Command Substitution: "${ command;} … the command is executed
    // in the current shell environment", and the value is its standard
    // output with trailing newlines removed. mksh(1) names the two forms
    // funsub `${ … ;}` and valsub `${| … ;}`; a valsub's value is the value
    // of REPLY and its stdout is NOT captured.
    //
    // zsh has neither: `${` followed by a blank or `|` reaches paramsubst
    // as a malformed name and errors "bad substitution", which is what
    // zshrs did in every mode.
    //
    // Reference outputs measured against ksh 93u+m and mksh R59.
    for flags in [&["--ksh"][..], &["--mksh"][..], &["--pdksh"][..]] {
        // Value is the captured stdout.
        assert_eq!(
            run_zshrs(flags, r#"print -r -- "${ printf inner; }""#).0,
            "inner\n",
            "{flags:?}: funsub value is the body's stdout"
        );
        assert_eq!(
            run_zshrs(flags, r#"print -r -- "${ print -n a; print -n b; }""#).0,
            "ab\n",
            "{flags:?}: a multi-command body"
        );
        // Trailing newlines are stripped, like `$( … )`.
        assert_eq!(
            run_zshrs(flags, "v=${ printf 'a\\n\\n\\n'; }; print -r -- \"[$v]\"").0,
            "[a]\n",
            "{flags:?}: trailing newlines removed"
        );
        // THE distinguishing property: state survives, where `$( … )`
        // would discard it.
        assert_eq!(
            run_zshrs(flags, r#"x=0; y=${ x=5; print -n out; }; print -r -- "x=$x y=$y""#).0,
            "x=5 y=out\n",
            "{flags:?}: a funsub shares the current shell environment"
        );
        assert_eq!(
            run_zshrs(flags, r#"x=0; y=$(x=5; print -n out); print -r -- "x=$x y=$y""#).0,
            "x=0 y=out\n",
            "{flags:?}: `$( … )` must still isolate"
        );
        // It is a command substitution, so it publishes the body's exit.
        assert_eq!(
            run_zshrs(flags, r#"v=${ false; }; print -r -- "rc=$?""#).0,
            "rc=1\n",
            "{flags:?}: funsub exit status"
        );
        // Quoting inside the body survives — the body is a fresh command
        // line, not a quoted expansion.
        assert_eq!(
            run_zshrs(flags, r#"q=qq; print -r -- "${ print -n "$q"; }""#).0,
            "qq\n",
            "{flags:?}: an expansion inside the body"
        );
        assert_eq!(
            run_zshrs(flags, r#"v=${ print -n "x  y"; }; print -r -- "[$v]""#).0,
            "[x  y]\n",
            "{flags:?}: quoted spaces in the body"
        );
    }

    // The valsub is the pdksh line's; ksh93 has no `${| … }`.
    for flags in [&["--mksh"][..], &["--pdksh"][..]] {
        assert_eq!(run_zshrs(flags, r#"print -r -- "${|REPLY=x;}""#).0, "x\n");
        assert_eq!(
            run_zshrs(flags, r#"print -r -- "${|REPLY=a; REPLY=$REPLY-b;}""#).0,
            "a-b\n"
        );
        assert_eq!(
            run_zshrs(flags, r#"print -r -- "${|REPLY=$((1+1));}""#).0,
            "2\n"
        );
        // Shares state like the funsub …
        assert_eq!(
            run_zshrs(flags, r#"x=0; y=${|x=5; REPLY=v;}; print -r -- "x=$x y=$y""#).0,
            "x=5 y=v\n"
        );
        // … but REPLY itself is local to it: the outer value is neither
        // visible inside nor clobbered after.
        assert_eq!(
            run_zshrs(flags, r#"REPLY=outer; y=${|:;}; print -r -- "[$y][$REPLY]""#).0,
            "[][outer]\n"
        );
        // stdout is NOT captured — it goes straight through.
        assert_eq!(
            run_zshrs(flags, r#"y=${|print hi; REPLY=v;}; print -r -- "y=$y""#).0,
            "hi\ny=v\n"
        );
    }

    // The POSIX-family drop-ins keep the "bad substitution" rejection,
    // because all three references do — measured on this host:
    //   bash -c 'printf "%s\n" "${ printf inner; }"'  -> rc 1
    //   dash -c 'printf "%s\n" "${ printf inner; }"'  -> rc 2
    //   sh   -c 'printf "%s\n" "${ printf inner; }"'  -> rc 1
    //
    // `--zsh` is NOT in this list any more. zsh 5.10 added the same three
    // forms natively as "nofork command substitution" (c:Src/subst.c:
    // 1913-1922, pinned by zsh's own D10nofork.ztst), and zshrs implements
    // them for every non-POSIX-drop-in mode — see the gate in
    // compile_zsh.rs and BUILTIN_KSH_FUNSUB. The zsh 5.9 binaries this
    // suite runs against still answer "bad substitution", so this is a
    // deliberate step AHEAD of the installed reference, not a divergence
    // from it; korn_funsub_and_valsub_run_in_the_current_shell pins the
    // ksh/mksh half above and the zsh half is covered by the nofork
    // tests.
    for flags in [&["--bash"][..], &["--dash"][..], &["--sh"][..]] {
        let (stdout, code) = run_zshrs(flags, r#"print -r -- "${ printf inner; }""#);
        assert_eq!(stdout, "", "{flags:?}: `${{ … }}` must not expand");
        assert_ne!(code, 0, "{flags:?}: `${{ … }}` must be an error");
    }
    // And an ordinary `${name}` / `${name-word}` is untouched everywhere.
    for flags in [&["--ksh"][..], &["--mksh"][..], &["--zsh"][..]] {
        assert_eq!(
            run_zshrs(flags, r#"x=abc; print -r -- "${x}|${y-def}|${#x}""#).0,
            "abc|def|3\n",
            "{flags:?}: ordinary braced expansions"
        );
    }
}
