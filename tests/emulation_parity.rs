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
            "/opt/homebrew/bin/bash",
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
        candidates: &["/opt/homebrew/bin/ksh", "ksh", "/bin/ksh", "/usr/bin/ksh"],
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
    // getopts: an option with an argument glued onto a cluster (`-abval`) must
    // advance past the whole word (dash-family OPTIND bias looped forever).
    "set -- -abval x; while getopts ab: o; do printf '%s:%s\\n' \"$o\" \"$OPTARG\"; done", // cluster + glued optarg
    "set -- -ab -c; while getopts abc o; do printf '%s' \"$o\"; done; echo",              // cluster then separate flag
    // Quoted "${*:-w}" joins the positionals on IFS[0]; under KSH_ARRAYS (sh/ksh/bash
    // emulation) the bare-array clamp must not apply to `$*` (c:Src/params.c:2288).
    "set -- a b; printf '[%s]' \"${*:-x}\"; echo",                // → [a b]
    "set -- a b; printf '[%s]' \"${*:+x}\" \"${*-y}\"; echo",     // → [x][a b]
    "set --; printf '[%s]' \"${*:-x}\"; echo",                    // → [x]
    // ── expanded corpus (differential sweep vs the real shells) ──
    "unset v; printf '%s\\n' \"${v-d}\" \"${v:-d}\"",
    "v=; printf '%s\\n' \"${v-d}\" \"${v:-d}\" \"${v+alt}\" \"${v:+alt}\"",
    "v=x; printf '%s\\n' \"${v-d}\" \"${v:-d}\" \"${v+alt}\" \"${v:+alt}\"",
    "unset v; printf '%s\\n' \"${v:=set}\"; printf '%s\\n' \"$v\"",
    "unset v; : ${v:=assigned}; printf '%s\\n' \"$v\"",
    "v=hello; printf '%s\\n' \"${v#h}\" \"${v##h*l}\" \"${v%o}\" \"${v%%l*}\"",
    "v=a.b.c; printf '%s\\n' \"${v#*.}\" \"${v##*.}\" \"${v%.*}\" \"${v%%.*}\"",
    "v=/usr/local/bin/tool; printf '%s\\n' \"${v##*/}\" \"${v%/*}\"",
    "v=abcabc; printf '%s\\n' \"${v#a?c}\" \"${v%a?c}\"",
    "v=abc123; printf '%s\\n' \"${v#[a-z]}\" \"${v##[a-z]*[a-z]}\" \"${v%[0-9]}\"",
    "v=foo; printf '%s\\n' \"${#v}\"",
    "v=; printf '%s\\n' \"${#v}\"",
    "v=\"a b c\"; printf '%s\\n' \"${#v}\"",
    "set -- a b c; printf '%s\\n' \"$#\" \"$1\" \"$2\" \"$3\"",
    "set -- a b c; shift; printf '%s\\n' \"$#\" \"$1\"",
    "set -- a b c d e; shift 3; printf '%s\\n' \"$#\" \"$1\" \"$2\"",
    "set -- \"a b\" c; for x in \"$@\"; do printf '[%s]' \"$x\"; done; printf '\\n'",
    "set -- \"a b\" c; for x in \"$*\"; do printf '[%s]' \"$x\"; done; printf '\\n'",
    "set -- \"a b\" c; for x in $*; do printf '[%s]' \"$x\"; done; printf '\\n'",
    "set -- \"a b\" c; IFS=:; printf '%s\\n' \"$*\"; unset IFS",
    "set -- a b c; IFS=; printf '%s\\n' \"$*\"; unset IFS",
    "set --; printf '%s\\n' \"$#\"; for x in \"$@\"; do printf 'never'; done; printf 'done\\n'",
    "set -- a b; printf '%s\\n' \"$@\" \"$@\"",
    "echo $((1+2*3)) $(((1+2)*3)) $((10/3)) $((10%3))",
    "echo $((-7/2)) $((-7%3)) $((7/-2)) $((7%-3))",
    "echo $((3>2)) $((3<2)) $((3>=3)) $((3<=2)) $((3==3)) $((3!=3))",
    "echo $((1&&0)) $((1||0)) $((!0)) $((!5))",
    "echo $((1?10:20)) $((0?10:20))",
    "echo $((0x1f)) $((010)) $((0xFF+1))",
    "x=5; echo $((x+1)) $((x*x)) $((x-10)); echo $x",
    "x=5; echo $((x=x+3)); echo $x",
    "x=5; echo $((x+=2)) $((x-=1)) $((x*=3)) $((x/=2)) $((x%=4))",
    "echo $(( 2 + 3 )) $((  4*5  ))",
    "x=3; y=4; echo $((x*x+y*y))",
    "i=0; while [ $i -lt 5 ]; do i=$((i+1)); [ $i = 3 ] && continue; printf '%s' $i; done; echo",
    "i=0; while :; do i=$((i+1)); [ $i -ge 4 ] && break; done; echo $i",
    "i=0; until [ $i -ge 3 ]; do printf '%s' $i; i=$((i+1)); done; echo",
    "for i in 1 2; do for j in a b; do printf '%s%s ' $i $j; done; done; echo",
    "for i in 1 2 3; do for j in a b c; do [ $j = b ] && break; printf '%s%s ' $i $j; done; done; echo",
    "for i in 1 2 3; do for j in a b c; do [ $j = b ] && continue 2; printf '%s%s ' $i $j; done; done; echo",
    "for i in 1 2 3; do for j in a b; do [ $i = 2 ] && break 2; printf '%s%s ' $i $j; done; done; echo",
    "for i in; do echo never; done; echo empty-ok",
    "for i in a*z_no_match; do printf '%s\\n' \"$i\"; done",
    "set -- x y; for i; do printf '%s;' \"$i\"; done; echo",
    "case abc in a*) echo A;; *) echo other;; esac",
    "case abc in b*) echo B;; *) echo other;; esac",
    "case abc in a|b) echo ab;; abc) echo abc;; esac",
    "case x in [a-z]) echo lower;; esac",
    "case \"\" in \"\") echo empty;; *) echo non;; esac",
    "case a in ?) echo one;; esac",
    "case ab in ?) echo one;; ??) echo two;; esac",
    "case \"a b\" in \"a b\") echo quoted;; esac",
    "case a in \"*\") echo star;; *) echo glob;; esac",
    "case '*' in \"*\") echo star;; *) echo glob;; esac",
    "case x in y) echo no;; esac; echo $?",
    "case x in x) false;; esac; echo $?",
    "f() { return 3; }; f; echo $?",
    "f() { return; }; false; f; echo $?",
    "f() { echo \"$#:$1:$2\"; }; f a b c",
    "f() { echo \"$1\"; shift; echo \"$1\"; }; f a b",
    "f() { g() { echo inner; }; g; }; f",
    "f() { echo \"$@\"; }; f \"a b\" c",
    "f() { for x in \"$@\"; do printf '<%s>' \"$x\"; done; echo; }; f \"a b\" \"\" c",
    "f() { [ $1 -le 1 ] && { echo 1; return; }; echo $(( $1 * $(f $(($1-1))) )); }; f 5",
    "f() { echo one; }; f() { echo two; }; f",
    "x=outer; f() { x=inner; }; f; echo $x",
    "x=outer; f() { x=inner; }; (f; echo $x); echo $x",
    "x=1; (x=2; echo $x); echo $x",
    "(exit 3); echo $?",
    "(exit 3) || echo failed",
    "! true; echo $?; ! false; echo $?",
    "true | false; echo $?",
    "false | true; echo $?",
    "echo hi | cat",
    "echo a b | { read x y; echo \"$y $x\"; }",
    "printf 'a\\nb\\n' | { read x; read y; echo \"$x$y\"; }",
    "printf 'a b c\\n' | { read x y; echo \"[$x][$y]\"; }",
    "printf '  a  b  \\n' | { read x; echo \"[$x]\"; }",
    "printf 'a,b,c\\n' | { IFS=, read x y z; echo \"$z$y$x\"; }",
    "printf 'a\\\\\\nb\\n' | { read x; echo \"[$x]\"; }",
    "printf 'a\\\\\\nb\\n' | { read -r x; echo \"[$x]\"; }",
    "printf 'one\\ntwo\\n' | while read l; do echo \"<$l>\"; done",
    "echo $x",
    "echo \"a$(echo b)c\"",
    "echo \"$(echo a; echo b)\"",
    "echo $(echo a; echo b)",
    "x=$(printf 'a\\n\\n\\n'); echo \"[$x]\"",
    "echo `echo back`",
    "echo \"`echo \"nested\"`\"",
    "echo $(echo $(echo deep))",
    "x=$(false); echo $?",
    "x=$(exit 4); echo $?",
    "echo \"${x:-$(echo default)}\"",
    "echo 'single $x \"quoted\"'",
    "echo \"double \\$x \\\"quoted\\\" \\\\ \\`\"",
    "echo a\\ b",
    "echo 'a''b'",
    "echo \"a\"\"b\"",
    "echo a\"b\"'c'",
    "x='a b'; echo $x; echo \"$x\"",
    "x='a  b'; echo $x; echo \"$x\"",
    "x='*'; echo \"$x\"",
    "IFS=:; x='a:b:c'; set -- $x; echo $#; unset IFS",
    "IFS=:; x='a::b'; set -- $x; echo $#; unset IFS",
    "IFS=' :'; x='a : b'; set -- $x; echo $#; unset IFS",
    "IFS=''; x='a b'; set -- $x; echo $#; unset IFS",
    "x=; set -- $x; echo $#",
    "x=; set -- \"$x\"; echo $#",
    "set -f; set -- *; echo \"$1\"; set +f",
    "[ -n abc ] && echo nonempty",
    "[ -z \"\" ] && echo empty",
    "[ -z abc ] || echo notempty",
    "[ a = a ] && [ b != c ] && echo ok",
    "[ 1 -eq 1 ] && [ 1 -ne 2 ] && [ 1 -lt 2 ] && [ 2 -gt 1 ] && [ 1 -le 1 ] && [ 1 -ge 1 ] && echo nums",
    "[ -d / ] && echo dir",
    "[ -f /etc/passwd ] && echo file",
    "[ -e /nonexistent_zz ] || echo gone",
    "[ ! -e /nonexistent_zz ] && echo notexists",
    "[ -n \"\" ] || echo nope",
    "[ \"a\" ] && echo str",
    "[ \"\" ] || echo emptystr",
    "[ a = a -a b = b ] && echo and",
    "[ a = b -o b = b ] && echo or",
    "test 3 -gt 2 && echo gt",
    "test -r /etc/passwd && echo readable",
    "[ -x /bin/sh ] && echo exec",
    "[ 10 -gt 9 ] && echo numeric",
    "[ 10 \\> 9 ] && echo lex-gt",
    "[ abc \\< abd ] && echo lex-lt",
    "[ -L /nonexistent_zz ] || echo notlink",
    "[ -s /etc/passwd ] && echo nonempty-file",
    "eval 'echo evaluated'",
    "eval 'x=5'; echo $x",
    "eval echo '$((1+1))'",
    "cmd='echo dyn'; eval \"$cmd\"",
    "eval 'f() { echo ef; }'; f",
    "x='a b'; eval \"set -- $x\"; echo $#",
    "eval \"echo \\$#\" a b c",
    ". /dev/null; echo $?",
    ": ; echo $?",
    ": foo bar; echo $?",
    "true; echo $?",
    "command echo cmd",
    "echo $?; echo $?",
    "x=1; export x; sh -c 'echo $x'",
    "x=1 sh -c 'echo $x'; echo \"[$x]\"",
    "x=1; x=2 true; echo $x",
    "export y=exp; sh -c 'echo $y'",
    "unset z; export z; sh -c 'echo \"[${z-unset}]\"'",
    "readonly r=1; echo $r",
    "unset x; echo \"[${x}]\"; x=1; unset x; echo \"[${x}]\"",
    "set -e; false; echo not-reached",
    "set -e; true; echo reached",
    "set -e; (false); echo not-reached",
    "set -e; false || echo guarded",
    "set -e; if false; then :; fi; echo after-if",
    "set -e; ! false; echo negated",
    "trap 'echo bye' EXIT; echo main",
    "trap 'echo bye' EXIT; exit 3",
    "(trap 'echo sub' EXIT; echo in); echo out",
    "trap 'echo a' EXIT; trap - EXIT; echo none",
    "{ echo a; echo b; } | wc -l | tr -d ' '",
    "{ echo a; echo b; } > /dev/null; echo $?",
    "echo err >&2 2>/dev/null; echo done",
    "echo out; echo err >&2",
    "( echo a; echo b ) | sed -n 2p",
    "exec 3>&1; echo fd3 >&3; exec 3>&-",
    "echo a > /dev/null; echo $?",
    "echo hi > /dev/null 2>&1; echo $?",
    "cat /nonexistent_zz 2>/dev/null; echo $?",
    "cat < /dev/null; echo $?",
    "printf '%5s|\\n' ab",
    "printf '%-5s|\\n' ab",
    "printf '%05d\\n' 42",
    "printf '%x %X %o\\n' 255 255 8",
    "printf '%s %s\\n' a b c",
    "printf '%s\\n' 'a\\tb'",
    "printf 'a\\tb\\n'",
    "printf '%s' ''; echo",
    "printf '%d %d\\n' 1",
    "printf '%s\\n'",
    "printf '\\n'",
    "printf '%3d|%-3d|\\n' 5 5",
    "printf '%+d % d\\n' 5 5",
    "printf '%.3s\\n' abcdef",
    "printf '%*d\\n' 5 42",
    "printf '%i\\n' 7",
    "printf '%u\\n' 7",
    "printf '%5s\\n' abcdefgh",
    "echo \"a  b\"",
    "echo a    b",
    "echo \"\"",
    "echo \" lead\"",
    "echo",
    "set -- a b c; echo \"${1}${2}${3}\"",
    "set -- a b c; echo \"${10}\" 2>/dev/null; echo ok",
    "x=abc; echo \"${x}d\" \"$x\"d",
    "x=abc; echo \"${x}\"",
    "v=ab; echo \"${v}${v}\"",
    "v=1; echo \"$((v+1))\" \"$(( v + 1 ))\" \"$(($v+1))\"",
    "v=3; echo $(( v > 2 ? v : 2 ))",
    "v=a; case $v in a) echo A;; esac",
    "v=\"a b\"; case $v in \"a b\") echo AB;; esac",
    "v=x; [ \"$v\" = x ] && echo same",
    "v=; [ \"$v\" = \"\" ] && echo empty",
    "v=a; w=b; echo \"$v$w\" \"${v}_${w}\"",
    "echo a; echo b; echo c",
    "echo a;echo b",
    "echo a && echo b || echo c",
    "false && echo a || echo b",
    "true || echo a && echo b",
    "{ echo a; } && echo b",
    "( echo a ) && echo b",
    "! { false; } && echo neg",
    "x=1; { x=2; }; echo $x",
    "if true; then echo t; else echo f; fi",
    "if false; then echo t; elif true; then echo e; else echo f; fi",
    "if false; then echo t; fi; echo $?",
    "if true; then :; fi; echo $?",
    "if [ 1 = 1 ] && [ 2 = 2 ]; then echo both; fi",
    "while false; do :; done; echo $?",
    "for i in 1 2; do :; done; echo $?",
    "f() { return 7; }; if f; then echo y; else echo n$?; fi",
    "f() { false; }; f && echo y || echo n",
    "f() { echo $1; }; f $(echo a b)",
    "f() { echo $#; }; f \"$(echo a b)\"",
    "f() { echo $#; }; f $(echo a b)",
    "echo a b | tr a-z A-Z",
    "echo abc | cut -c2",
    "echo 1 2 3 | awk '{print $2}'",
    "echo hello | sed 's/l/L/g'",
    "printf 'x\\n' | grep -c x",
    // pathname expansion: a drop-in must glob like the real shell (the harness used to hand it `-f`, which is `set -f` there).
    "printf '%s\\n' /etc/pass[w]d",
    "printf '%s\\n' /et?",
    "set -- /etc/pass*; printf '%s\\n' \"$#\"",
    "for f in /etc/passw[d]; do printf '%s\\n' \"$f\"; done",
    "printf '%s\\n' /nonexistent_zz*",
    "x='/etc/pass*'; printf '%s\\n' \"$x\"",
    "set -f; printf '%s\\n' /etc/pass*; set +f",
    // ── tenth-pass sweep: portable constructs verified byte-identical on all nine ways ──
    "v=hello; printf '%s\\n' \"${v:-x}\" \"${v:+y}\" \"${v-z}\" \"${v+w}\"",
    "unset v; printf '%s|%s|%s|%s\\n' \"${v:-x}\" \"${v:+y}\" \"${v-z}\" \"${v+w}\"",
    "v=; printf '%s|%s|%s|%s\\n' \"${v:-x}\" \"${v:+y}\" \"${v-z}\" \"${v+w}\"",
    "unset v; : \"${v:=assigned}\"; printf '%s\\n' \"$v\"",
    "v=abcabc; printf '%s %s %s %s\\n' \"${v#a*b}\" \"${v##a*b}\" \"${v%b*c}\" \"${v%%b*c}\"",
    "v=/usr/local/bin/tool.tar.gz; printf '%s %s %s %s\\n' \"${v##*/}\" \"${v%/*}\" \"${v%.*}\" \"${v%%.*}\"",
    "v=abc; printf '%s\\n' \"${#v}\"",
    "v=\"\"; printf '%s\\n' \"${#v}\"",
    "set -- a b c d; printf '%s\\n' \"$#\" \"$1\" \"$4\" \"${4}\"",
    "set -- a b c d; shift 2; printf '%s %s\\n' \"$#\" \"$1\"",
    "set -- \"a b\" c; printf '[%s]\\n' \"$@\"",
    "set -- \"a b\" c; printf '[%s]\\n' \"$*\"",
    "set -- a b c; IFS=:; printf '%s\\n' \"$*\"; unset IFS",
    "set -- a b c; IFS=-; printf '[%s]\\n' \"$@\"; unset IFS",
    "f() { printf '%s\\n' \"$#\"; }; f; f a; f a b c",
    "f() { printf '%s\\n' \"$0\" | grep -c . ; }; f",
    "f() { return 3; }; f; printf '%s\\n' \"$?\"",
    "f() { return 0; }; f && printf yes; printf '\\n'",
    "x=outer; f() { x=inner; }; f; printf '%s\\n' \"$x\"",
    "f() { printf '%s\\n' \"$1\"; shift; printf '%s\\n' \"$1\"; }; f a b",
    "f() { g() { printf inner; }; g; printf '\\n'; }; f",
    "f() { printf '%s' \"$1\"; if [ \"$1\" -gt 0 ]; then f $(( $1 - 1 )); fi; }; f 3; printf '\\n'",
    "fact() { if [ \"$1\" -le 1 ]; then printf 1; else printf '%s' $(( $1 * $(fact $(( $1 - 1 ))) )); fi; }; fact 5; printf '\\n'",
    "printf '%s\\n' $((7 % 3)) $((-7 % 3)) $((7 / 2)) $((-7 / 2))",
    "printf '%s\\n' $((1<<4)) $((256>>2)) $((5&3)) $((5|3)) $((5^3)) $((~5))",
    "printf '%s\\n' $((3>2)) $((3<2)) $((3==3)) $((3!=3)) $((3>=3)) $((3<=2))",
    "printf '%s\\n' $((1&&0)) $((1||0)) $((!0)) $((!5))",
    "printf '%s\\n' $((1?2:3)) $((0?2:3))",
    "x=5; printf '%s\\n' $((x++)) 2>/dev/null; printf '%s\\n' \"$x\"",
    "x=5; x=$((x+1)); x=$((x*2)); printf '%s\\n' \"$x\"",
    "x=5; printf '%s\\n' $((x+=3)) $x",
    "x=0x1f; printf '%s\\n' $((x)) ",
    "printf '%s\\n' $((010)) $((0x10))",
    "printf '%x %o %X\\n' 255 8 255",
    "printf '%5d|%-5d|%05d\\n' 42 42 42",
    "printf '%.3s|%10s|%-10s|\\n' abcdef abc abc",
    "printf '%c\\n' hello",
    "printf '%s %s\\n' a",
    "printf '%s\\n' a b c",
    "printf '%d %d\\n' 1 2 3 4",
    "printf '%e\\n' 1234.5",
    "printf '%g %g\\n' 100000 1000000",
    "printf 'a\\\\nb\\n'",
    "printf '%s' ''; printf 'x\\n'",
    "printf '\\101\\102\\n'",
    "for i in a b c; do printf '%s,' \"$i\"; done; printf '\\n'",
    "for i in 1 2 3; do if [ \"$i\" = 2 ]; then continue; fi; printf '%s' \"$i\"; done; printf '\\n'",
    "for i in 1 2 3; do if [ \"$i\" = 2 ]; then break; fi; printf '%s' \"$i\"; done; printf '\\n'",
    "for i in 1 2; do for j in a b; do printf '%s%s ' \"$i\" \"$j\"; done; done; printf '\\n'",
    "for i in 1 2 3; do for j in a b; do if [ \"$j\" = b ]; then break 2; fi; printf '%s%s ' \"$i\" \"$j\"; done; done; printf '\\n'",
    "i=0; until [ \"$i\" -ge 3 ]; do printf '%s' \"$i\"; i=$((i+1)); done; printf '\\n'",
    "i=3; while [ \"$i\" -gt 0 ]; do printf '%s' \"$i\"; i=$((i-1)); done; printf '\\n'",
    "while false; do printf never; done; printf '%s\\n' \"$?\"",
    "for i in; do printf never; done; printf '%s\\n' \"$?\"",
    "case abc in a*) printf A;; *) printf B;; esac; printf '\\n'",
    "case abc in x*|a*) printf A;; esac; printf '\\n'",
    "case abc in [a-c]*) printf A;; esac; printf '\\n'",
    "case \"\" in \"\") printf empty;; *) printf non;; esac; printf '\\n'",
    "case a in b) printf B;; esac; printf '%s\\n' \"$?\"",
    "case a.b in *.b) printf yes;; esac; printf '\\n'",
    "case 'a b' in \"a b\") printf yes;; esac; printf '\\n'",
    "case x in ?) printf one;; esac; printf '\\n'",
    "case ab in ?) printf one;; ??) printf two;; esac; printf '\\n'",
    "x=5; if [ \"$x\" -eq 5 ] && [ \"$x\" -lt 10 ]; then printf ok; fi; printf '\\n'",
    "x=5; if [ \"$x\" -gt 5 ] || [ \"$x\" -lt 10 ]; then printf ok; fi; printf '\\n'",
    "if [ -z \"\" ]; then printf z; fi; if [ -n \"x\" ]; then printf n; fi; printf '\\n'",
    "if [ ! -e /nonexistent_zzz ]; then printf missing; fi; printf '\\n'",
    "if [ -d / ]; then printf dir; fi; if [ -f / ]; then printf file; fi; printf '\\n'",
    "if [ \"a\" = \"a\" ] && [ \"a\" != \"b\" ]; then printf ok; fi; printf '\\n'",
    "if [ 1 -eq 1 ]; then printf a; elif [ 2 -eq 2 ]; then printf b; else printf c; fi; printf '\\n'",
    "if [ 1 -eq 2 ]; then printf a; elif [ 2 -eq 2 ]; then printf b; else printf c; fi; printf '\\n'",
    "if [ 1 -eq 2 ]; then printf a; elif [ 2 -eq 3 ]; then printf b; else printf c; fi; printf '\\n'",
    "test 3 -lt 10 && printf lt; printf '\\n'",
    "[ \"10\" -lt \"9\" ] || printf numeric; printf '\\n'",
    "[ -z \"$undefined_zz\" ] && printf unset; printf '\\n'",
    "x=$(printf 'a\\nb\\n'); printf '[%s]\\n' \"$x\"",
    "x=$(printf 'a  \\n\\n\\n'); printf '[%s]\\n' \"$x\"",
    "x=`printf backtick`; printf '%s\\n' \"$x\"",
    "x=$(printf '%s' \"$(printf nested)\"); printf '%s\\n' \"$x\"",
    "x=$(printf '%s' \"a b\"); set -- $x; printf '%s\\n' \"$#\"",
    "printf '%s\\n' \"$(printf 'a\\\\nb')\"",
    "( printf sub; exit 2 ); printf ' %s\\n' \"$?\"",
    "( x=inner ); printf '[%s]\\n' \"$x\"",
    "x=outer; ( x=inner; printf '%s' \"$x\" ); printf ' %s\\n' \"$x\"",
    "{ printf grouped; } ; printf '\\n'",
    "{ x=in; }; printf '%s\\n' \"$x\"",
    "cd / && printf '%s\\n' \"$PWD\"",
    "( cd / ; printf '%s\\n' \"$PWD\" ); [ \"$PWD\" != / ] || printf same; printf '\\n'",
    "printf 'a\\nb\\nc\\n' | while read -r l; do printf '<%s>' \"$l\"; done; printf '\\n'",
    "printf 'a b c\\n' | { read x y z; printf '%s|%s|%s\\n' \"$x\" \"$y\" \"$z\"; }",
    "printf 'a b c d\\n' | { read x y; printf '%s|%s\\n' \"$x\" \"$y\"; }",
    "printf 'a\\n' | { read x y; printf '%s|%s\\n' \"$x\" \"$y\"; }",
    "printf '  a  b  \\n' | { read x; printf '[%s]\\n' \"$x\"; }",
    "printf 'a:b:c\\n' | { IFS=: read x y z; printf '%s|%s|%s\\n' \"$x\" \"$y\" \"$z\"; }",
    "printf 'a\\\\b\\n' | { read -r x; printf '%s\\n' \"$x\"; }",
    "printf 'a\\\\\\nb\\n' | { read x; printf '%s\\n' \"$x\"; }",
    "printf '' | { read x; printf '%s|%s\\n' \"$?\" \"$x\"; }",
    "printf 'x' | { read v; printf '%s|%s\\n' \"$?\" \"$v\"; }",
    "printf 'x\\n' > /dev/null; printf 'y\\n'",
    "printf 'x\\n' >&2 2>/dev/null; printf 'y\\n'",
    "{ printf out; printf err >&2; } 2>/dev/null; printf '\\n'",
    "{ printf out; printf err >&2; } 2>&1 | cat; printf '\\n'",
    "printf a > /dev/null && printf b; printf '\\n'",
    "exec 3>&1; printf fd3 >&3; printf '\\n'",
    "printf 'abc' | wc -c | tr -d ' '",
    "printf 'a\\nb\\n' | grep -c .",
    "printf 'b\\na\\nc\\n' | sort | tr '\\n' ' '; printf '\\n'",
    "printf 'a\\n' | cat - | cat",
    "true; printf '%s\\n' \"$?\"",
    "false; printf '%s\\n' \"$?\"",
    "! true; printf '%s\\n' \"$?\"",
    "! false; printf '%s\\n' \"$?\"",
    "true && false || printf recovered; printf '\\n'",
    "false && printf no || printf yes; printf '\\n'",
    ": ; printf '%s\\n' \"$?\"",
    "x=1 true; printf '[%s]\\n' \"$x\"",
    "x=1 sh -c 'printf \"%s\\n\" \"$x\"'",
    "export X=exp; sh -c 'printf \"%s\\n\" \"$X\"'",
    "X=noexp; sh -c 'printf \"[%s]\\n\" \"$X\"'",
    "export Y; Y=late; sh -c 'printf \"%s\\n\" \"$Y\"'",
    "unset U; printf '[%s]\\n' \"$U\"",
    "x=1; unset x; printf '[%s]\\n' \"${x-unset}\"",
    "command -v printf >/dev/null && printf found; printf '\\n'",
    "command -v nonexistent_zzz >/dev/null 2>&1; printf '%s\\n' \"$?\"",
    "eval 'x=5'; printf '%s\\n' \"$x\"",
    "eval 'printf \"%s\\n\" a b'",
    "x='printf hi'; eval \"$x\"; printf '\\n'",
    "eval \"f() { printf fn; }\"; f; printf '\\n'",
    "eval 'set -- 1 2 3'; printf '%s\\n' \"$#\"",
    "v=x; eval \"$v=1\"; printf '%s\\n' \"$x\"",
    "trap 'printf trapped' EXIT; printf start; printf '\\n'",
    "(trap 'printf sub-exit\\n' EXIT; printf in\\n)",
    "trap 'printf int\\n' INT; kill -INT $$; printf after\\n",
    "trap '' INT; kill -INT $$; printf survived\\n",
    "trap 'printf one\\n' USR1; kill -USR1 $$; printf two\\n",
    "f() { trap 'printf fnexit\\n' EXIT; }; f; printf done\\n",
    "set -e; true; printf reached",
    "set -e; false || printf ok; printf '\\n'",
    "set -e; if false; then :; fi; printf ok; printf '\\n'",
    "set -e; ! false; printf ok; printf '\\n'",
    "set -u; printf '%s\\n' \"${undef_zz-default}\"",
    "set -f; printf '%s\\n' *",
    "set +f; cd /; printf '%s\\n' /bi*",
    "set -- a b; printf '%s\\n' \"$@\" | wc -l | tr -d ' '",
    "getopts 'ab:' o -a; printf '%s %s\\n' \"$o\" \"$OPTIND\"",
    "set -- -a -b val rest; while getopts 'ab:' o; do printf '%s=%s ' \"$o\" \"$OPTARG\"; done; printf '%s\\n' \"$OPTIND\"",
    "set -- -x; getopts 'a' o 2>/dev/null; printf '%s\\n' \"$o\"",
    "set -- -ab; while getopts 'ab' o; do printf '%s' \"$o\"; done; printf '\\n'",
    "set -- -- -a; while getopts 'a' o; do printf '%s' \"$o\"; done; printf '%s\\n' \"$OPTIND\"",
    "alias_test() { :; }; alias_test; printf '%s\\n' \"$?\"",
    "wait; printf '%s\\n' \"$?\"",
    "sleep 0 & wait $!; printf '%s\\n' \"$?\"",
    "sh -c 'exit 7' & wait $!; printf '%s\\n' \"$?\"",
    "sh -c 'exit 7'; printf '%s\\n' \"$?\"",
    "x=a; y=b; printf '%s%s\\n' \"$x\" \"$y\"",
    "x=a; printf '%s\\n' \"${x}b\" \"$xb\" ",
    "x=a; printf '%s\\n' \"$x$x\" \"${x}${x}\"",
    "printf '%s\\n' \"a  b\" 'a  b' a\\ \\ b",
    "printf '%s\\n' \"$\"; printf '%s\\n' '$'",
    "printf '%s\\n' \"\\$x\" '\\$x' \"\\\\\" '\\\\'",
    "printf '%s\\n' \"a\\\"b\" 'a\"b' \"a'b\"",
    "printf '%s\\n' ab\"cd\"ef 'ab'cd\"ef\"",
    "x=\"a b\"; printf '[%s]' $x; printf '\\n'",
    "x=\"a b\"; printf '[%s]' \"$x\"; printf '\\n'",
    "x=\"  a   b  \"; printf '[%s]' $x; printf '\\n'",
    "x=\"*\"; set -f; printf '[%s]' $x; printf '\\n'",
    "x=a:b:c; IFS=:; printf '[%s]' $x; unset IFS; printf '\\n'",
    "x=a::c; IFS=:; printf '[%s]' $x; unset IFS; printf '\\n'",
    "x=:a:; IFS=:; printf '[%s]' $x; unset IFS; printf '\\n'",
    "x='a b'; IFS=; printf '[%s]' $x; unset IFS; printf '\\n'",
    "printf '%s\\n' ~ | grep -c /",
    "printf '%s\\n' \"~\"",
    "printf '%s\\n' ~root | grep -c root",
    "printf '%s\\n' a{b,c}d 2>/dev/null | head -1 | grep -c .",
    "cd /tmp 2>/dev/null; printf '%s\\n' \"$OLDPWD\" | grep -c .",
    ": > /tmp/zshrs_nine_$$; [ -e /tmp/zshrs_nine_$$ ] && printf exists; rm -f /tmp/zshrs_nine_$$; printf '\\n'",
    "printf data > /tmp/zshrs_nine_$$; cat /tmp/zshrs_nine_$$; rm -f /tmp/zshrs_nine_$$; printf '\\n'",
    "printf a > /tmp/zshrs_nine_$$; printf b >> /tmp/zshrs_nine_$$; cat /tmp/zshrs_nine_$$; rm -f /tmp/zshrs_nine_$$; printf '\\n'",
    "printf 'l1\\nl2\\n' > /tmp/zshrs_nine_$$; while read -r l; do printf '<%s>' \"$l\"; done < /tmp/zshrs_nine_$$; rm -f /tmp/zshrs_nine_$$; printf '\\n'",
    "x=$(cat < /dev/null); printf '[%s]\\n' \"$x\"",
    "printf '%s\\n' \"$$\" | grep -c '^[0-9][0-9]*$'",
    "printf '%s\\n' \"$#\"; printf '%s\\n' \"$?\"",
    "true; f() { printf '%s\\n' \"$?\"; }; false; f",
    "printf '%s\\n' \"${PATH:+set}\"",
    "printf '%s\\n' \"$((0))\" \"$((-1))\" \"$((- -1))\"",
    "printf '%s\\n' \"$(( (1+2)*3 ))\" \"$(( 1+2*3 ))\"",
    "printf '%s\\n' \"$(( 10 / 3 ))\" \"$(( 10 % 3 ))\"",
    "x=3; y=4; printf '%s\\n' \"$(( x*x + y*y ))\"",
    "printf '%s\\n' \"$(( 1 , 2 ))\" 2>/dev/null || printf 2\\\\n",
    "n=0; while [ $n -lt 3 ]; do n=$((n+1)); [ $n -eq 2 ] && continue; printf '%s' $n; done; printf '\\n'",
    "s=\"\"; for w in a b c; do s=\"$s$w\"; done; printf '%s\\n' \"$s\"",
    "s=\"\"; for w in a b c; do s=\"${s:+$s,}$w\"; done; printf '%s\\n' \"$s\"",
    "for f in \"a b\" c; do printf '[%s]' \"$f\"; done; printf '\\n'",
    "for i in $(printf '1 2 3'); do printf '%s' $((i*2)); done; printf '\\n'",
    "set -- 3 1 2; for a in \"$@\"; do printf '%s' \"$a\"; done; printf '\\n'",
    "set -- ; for a in \"$@\"; do printf never; done; printf '%s\\n' \"$#\"",
    "f() { for a; do printf '%s' \"$a\"; done; printf '\\n'; }; f x y z",
    "f() { while [ $# -gt 0 ]; do printf '%s' \"$1\"; shift; done; printf '\\n'; }; f x y z",
    "f() { case $1 in a) return 1;; esac; return 0; }; f a; printf '%s' \"$?\"; f b; printf '%s\\n' \"$?\"",
    "f() { printf '%s' \"$1\"; }; g() { f \"$1$1\"; }; g ab; printf '\\n'",
    "f() { x=$1; }; f 5; printf '%s\\n' \"$x\"",
    "f() { printf '%s\\n' \"$@\"; }; f \"a b\" c",
    "f() { printf '%s\\n' $#; }; f \"\" \"\"",
    "f() { [ -z \"$1\" ] && printf empty; printf '\\n'; }; f \"\"",
    "f() { echo_n() { printf '%s' \"$1\"; }; echo_n \"$1\"; printf '\\n'; }; f hi",
    "cat <<EOF2\nhere doc\nEOF2",
    "x=val; cat <<EOF2\n$x\nEOF2",
    "x=val; cat <<'EOF2'\n$x\nEOF2",
    "cat <<EOF2\n$(printf sub)\nEOF2",
    "cat <<-EOF2\n\ttabbed\n\tEOF2",
    "cat <<EOF2\na\\$b\nEOF2",
    "cat <<EOF2\nline1\nline2\nEOF2",
    "cat <<EOF2 | tr a-z A-Z\nshout\nEOF2",
    "f() { cat <<EOF2\nin $1\nEOF2\n}; f arg",
    "x=1; cat <<EOF2\n${x:-d} ${y:-d} $((x+1))\nEOF2",
    "cat <<\"EOF2\"\n$notexpanded `x`\nEOF2",
    "cat <<EOF2; printf after\\\\n\nbody\nEOF2",
    "while read -r l; do printf '<%s>' \"$l\"; done <<EOF2; printf '\\n'\na\nb\nEOF2",
    "cat <<A <<B\nfirst\nA\nsecond\nB",
    // A command substitution's output is split on IFS and nothing else: a
    // `$name` in it is never expanded again.
    "x=1; printf '%s\\n' $(echo '$x') \"$(echo '$x')\" $(echo \\$x)",
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
    // A function whose body is a bare compound command (no braces): the
    // `name ()` form takes ANY command as its body, with no SHORTLOOPS gate
    // (Src/parse.c:2110 `par_cmd`). `( … )` was a parse error in every
    // Bourne-family mode, where SHORTLOOPS is off.
    "f() ( printf '%s\\n' sub ); f",                       // subshell function body
    "f() ( printf '%s\\n' a; printf '%s\\n' b ); f",       // multi-command subshell body
    // bash and ksh93 number $LINENO inside a function by the absolute script
    // line; zsh counts from the line the function was defined on.
    "f() { printf '%s\\n' \"$LINENO\"; }\nf\nf",           // one-line def, called twice
    "g() {\nprintf '%s\\n' \"$LINENO\"\n}\ng",             // body on its own line
    // bash/zsh brace expansion keeps/drops an empty alternative the same way
    // as their reference: an unquoted empty brace word is removed in bash.
    "printf '[%s]' {,a}; printf '\\n'",                    // leading empty alternative
    "printf '[%s]' a{,b}c {a,}; printf '\\n'",             // trailing / embedded empty
    "for i in {,a,}; do printf '[%s]\\n' \"$i\"; done",    // for-list drops empties
    "true & printf '[%s]\\n' \"${!:+set}\"",               // $! set after a background job
    // A QUOTED substitution in a `case` pattern is literal even when GLOB_SUBST
    // is on (sh/ksh/bash emulation): C never shtokenizes a quoted expansion
    // (Src/subst.c:822/830), only an unquoted one.
    "x='a*'; case abc in \"$x\") echo lit;; *) echo nolit;; esac",   // quoted → literal
    "x='b*'; case abc in a\"$x\") echo lit;; *) echo nolit;; esac",  // quoted tail → literal
    "x='a*'; case abc in \"$x\"*) echo m;; *) echo n;; esac",        // quoted prefix + glob
    "x='a*'; case 'a*' in \"$x\") echo m;; *) echo n;; esac",        // literal still matches itself
    // `read -a` / `-N` are bash letters: zsh rejects them (Src/builtin.c:109).
    "printf 'a b c\\n' | { read -a arr; echo ${#arr[@]}; }",          // -a: bash/ksh array, zsh error
    "printf 'abc\\n' | { read -N 2 v; echo \"[$v]\"; }",              // -N: bash/ksh count, zsh error
    // `export NAME` of a never-assigned parameter does not enter the
    // environment until the first assignment (Src/builtin.c:2299-2301).
    "export Z; env | grep -c '^Z='",
    "typeset -x Y; env | grep -c '^Y='",
    "export Z; Z=1; env | grep -c '^Z='",
    "export A B=2; sh -c 'echo ${A-unset} $B'",
    "Z=1; export Z; env | grep -c '^Z='",
    // ── expanded corpus (differential sweep vs the real shells) ──
    "a=(x y z); echo ${a[0]} ${a[1]} ${a[2]}",
    "a=(x y z); echo \"${a[@]}\"; echo \"${#a[@]}\"",
    "a=(x y z); a[3]=w; echo \"${a[@]}\" \"${#a[@]}\"",
    "a=(x y z); a+=(w); echo \"${a[@]}\"",
    "a=(x y z); unset 'a[1]'; echo \"${#a[@]}\"",
    "a=(\"a b\" c); for x in \"${a[@]}\"; do echo \"[$x]\"; done",
    "a=(\"a b\" c); for x in \"${a[*]}\"; do echo \"[$x]\"; done",
    "a=(x y z); echo \"${a[@]:1}\"",
    "a=(x y z w); echo \"${a[@]:1:2}\"",
    "a=(x y z); echo \"${a[-1]}\"",
    "a=(x y z); i=1; echo \"${a[i]}\" \"${a[i+1]}\"",
    "a=(1 2 3); echo $((a[0]+a[1]+a[2]))",
    "a=(x y z); b=(\"${a[@]}\"); b[0]=q; echo \"${a[0]}${b[0]}\"",
    "a=(a b c); echo \"${a[@]/b/X}\"",
    "a=(abc abd); echo \"${a[@]#ab}\"",
    "a=(a b); echo \"${a[@]}\" \"${a[@]}\"",
    "a=(x y z); set -- \"${a[@]}\"; echo $# $2",
    "a=(x y z); echo ${#a[1]}",
    "declare -a a; a[2]=x; echo \"${#a[@]}\"",
    "x=hello; echo \"${x:1}\" \"${x:1:3}\" \"${x: -2}\" \"${x: -3:2}\"",
    "x=hello; echo \"${x:0:0}\" \"${x:5}\" \"${x:2:100}\"",
    "x=hello; echo \"${x/l/L}\" \"${x//l/L}\" \"${x/#h/H}\" \"${x/%o/O}\"",
    "x=hello; echo \"${x/ll}\" \"${x//l}\"",
    "x=hello; echo \"${x/l*/X}\" \"${x//[el]/_}\"",
    "x=a.b.c; echo \"${x//./-}\" \"${x/./-}\"",
    "x=hello; echo \"${x/#x/Y}\" \"${x/%x/Y}\"",
    "x=hello; y=l; echo \"${x//$y/L}\"",
    "x=hello; echo \"${x/\\l/L}\"",
    "x=\"a b\"; echo \"${x/ /_}\"",
    "x=abc; echo \"${x^}\" \"${x^^}\" 2>/dev/null",
    "x=ABC; echo \"${x,}\" \"${x,,}\" 2>/dev/null",
    "[[ abc == a* ]] && echo glob",
    "[[ abc == \"a*\" ]] || echo quoted-literal",
    "[[ abc != b* ]] && echo notmatch",
    "[[ -n x && -z \"\" ]] && echo and",
    "[[ -z x || -n y ]] && echo or",
    "[[ ! -z x ]] && echo not",
    "[[ 1 -eq 1 ]] && echo numeq",
    "[[ 2 -gt 1 && 3 -lt 4 ]] && echo numcmp",
    "[[ abc < abd ]] && echo lt",
    "[[ abd > abc ]] && echo gt",
    "[[ a == a ]] && echo eq",
    "[[ a = a ]] && echo eq1",
    "[[ \"a b\" == \"a b\" ]] && echo spaced",
    "x='a b'; [[ $x == \"a b\" ]] && echo nosplit",
    "x=; [[ -z $x ]] && echo empty",
    "[[ -e /etc/passwd ]] && echo exists",
    "[[ -d / && -f /etc/passwd ]] && echo types",
    "[[ a == [a-c] ]] && echo class",
    "[[ a == ?(a|b) ]] && echo ext 2>/dev/null",
    "(( 1 + 1 == 2 )) && echo math",
    "(( 3 > 2 )) && echo gt",
    "(( 0 )) || echo zero",
    "(( 5 )) && echo nonzero",
    "x=5; (( x++ )); echo $x",
    "x=5; (( x-- )); echo $x",
    "x=5; (( ++x )); echo $x",
    "x=5; (( x += 3 )); echo $x",
    "x=5; (( y = x * 2 )); echo $y",
    "(( 0 )); echo $?",
    "(( 7 % 4 == 3 )) && echo mod",
    "echo $((2**3)) $((2**10)) $((3**0))",
    "echo $((10 > 5 ? 1 : 2))",
    "echo $((x = 4, x * 2))",
    "echo $((16#ff)) $((2#101)) $((8#17))",
    "echo $((1 + 2)) $(( 1+2 ))",
    "let \"x = 3 + 4\"; echo $x",
    "let x=2 y=3; echo $((x*y))",
    "let \"x = 0\"; echo $?",
    "let \"x = 5\"; echo $?",
    "echo {a,b,c}",
    "echo {1..5}",
    "echo {a..e}",
    "echo {1..10..3}",
    "echo {5..1}",
    "echo a{b,c}d",
    "echo {a,b}{1,2}",
    "echo {a,b,{c,d}}",
    "echo x{}y",
    "echo {a}",
    "echo pre{1..3}post",
    "echo {01..05}",
    "echo {a,b}{,c}",
    "for i in {1..3}; do printf '%s' $i; done; echo",
    "echo \"{a,b}\"",
    "echo '{a,b}'",
    "echo \\{a,b\\}",
    "function f { echo func-kw; }; f",
    "f() { return 2; }; f; echo $?",
    "typeset -i n=5; n=n*2; echo $n",
    "typeset -r c=1; echo $c",
    "typeset -u u=abc 2>/dev/null; echo $u",
    "typeset -l l=ABC 2>/dev/null; echo $l",
    "typeset -a arr; arr=(1 2); echo \"${arr[@]}\"",
    "echo a b c | { read -a arr; echo \"${arr[1]}\"; } 2>/dev/null",
    "echo \"a:b\" | { IFS=: read x y; echo \"$y\"; }",
    "printf '%q\\n' \"a b\"",
    "printf '%q\\n' abc",
    "printf '%s\\n' \"$(printf '%q' \"it's\")\"",
    "printf -v v '%s-%s' a b; echo $v",
    "echo $'a\\tb'",
    "echo $'a\\nb'",
    "echo $'\\x41'",
    "echo $'\\101'",
    "echo $'it\\'s'",
    "echo $\"text\" 2>/dev/null",
    "echo ${!a[@]} 2>/dev/null",
    "a=(x y z); echo \"${!a[@]}\"",
    "unset a; echo \"${a-unset}\"",
    "x=1; echo \"${x:+set}\" \"${y:+set}\"",
    "x=; echo \"${x:-d}\" \"${x-d}\"",
    "echo \"${undef_zz:?}\" 2>/dev/null; echo $?",
    "v=a; echo \"${v:=b}\" \"$v\"",
    "OPTIND=1; echo $OPTIND",
    "shopt -s nullglob 2>/dev/null; echo ok",
    "trap 'echo err' ERR; false; echo after 2>/dev/null",
    "trap 'echo debug' DEBUG 2>/dev/null; : ; trap - DEBUG",
    "trap 'echo ret' RETURN 2>/dev/null; f() { :; }; f; trap - RETURN",
    "[[ $(( 1 + 2 )) -eq 3 ]] && echo arith-in-cond",
    "x=1; [[ -v x ]] && echo xset",
    "unset x; [[ -v x ]] || echo xunset",
    "[[ -n ${x+set} ]] || echo unset-test",
    "for ((i=0; i<3; i++)); do printf '%s' $i; done; echo",
    "for ((;;)); do break; done; echo c-for",
    "for ((i=0,j=10; i<3; i++,j--)); do printf '%s,%s ' $i $j; done; echo",
    "i=5; while ((i>0)); do ((i-=2)); done; echo $i",
    "echo $(( $(echo 3) + 1 ))",
    "echo \"${a[0]:-none}\"",
    "declare -A m; m[k]=v; echo \"${m[k]}\"",
    "typeset -A m; m[x]=1; echo \"${m[x]}\"",
    // ── tenth-pass sweep: arrays / [[ ]] / (( )) / braces / string ops verified on zsh, bash, ksh and ksh/zsh-style ──
    "a=(x y z); printf '%s\\n' \"${a[0]}\" \"${a[1]}\" \"${a[2]}\"",
    "a=(x y z); printf '[%s]' \"${a[*]}\"; printf '\\n'",
    "a=(\"a b\" c); printf '[%s]' \"${a[@]}\"; printf '\\n'",
    "a=(\"a b\" c); printf '[%s]' \"${a[*]}\"; printf '\\n'",
    "a=(x y z); a[1]=Y; printf '%s' \"${a[@]}\"; printf '\\n'",
    "a=(x y); a+=(z); printf '%s' \"${a[@]}\"; printf '\\n'",
    "a=(x y); a[5]=q; printf '%s\\n' \"${#a[@]}\"",
    "a=(x y z); unset 'a[1]'; printf '%s\\n' \"${#a[@]}\"",
    "a=(x y z); printf '%s\\n' \"${a[@]:1}\" ",
    "a=(x y z w); printf '%s ' \"${a[@]:1:2}\"; printf '\\n'",
    "a=(x y z w); printf '%s ' \"${a[@]: -2}\"; printf '\\n'",
    "a=(1 2 3); for i in \"${a[@]}\"; do printf '%s' $((i*i)); done; printf '\\n'",
    "a=(x y z); printf '%s\\n' \"${a[-1]}\"",
    "a=(x y z); printf '%s\\n' \"${a[@]/y/Y}\" ",
    "a=(abc abd); printf '%s ' \"${a[@]#ab}\"; printf '\\n'",
    "a=(abc abd); printf '%s ' \"${a[@]%?}\"; printf '\\n'",
    "a=(a b c); set -- \"${a[@]}\"; printf '%s\\n' \"$#\"",
    "set -- a b c; a=(\"$@\"); printf '%s\\n' \"${#a[@]}\"",
    "a=(3 1 2); printf '%s\\n' \"${a[@]}\" | sort | tr '\\n' ' '; printf '\\n'",
    "a=(one two); i=1; printf '%s\\n' \"${a[i]}\"",
    "a=(one two); i=0; printf '%s\\n' \"${a[i+1]}\"",
    "x=(a b c); printf '%s\\n' \"${x[1]}${x[2]}\"",
    "[[ abc == a* ]] && printf yes; printf '\\n'",
    "[[ abc == \"a*\" ]] || printf no; printf '\\n'",
    "[[ abc = abc ]] && printf eq; printf '\\n'",
    "[[ abc != abd ]] && printf ne; printf '\\n'",
    "[[ -n x && -z \"\" ]] && printf both; printf '\\n'",
    "[[ -z x || -n x ]] && printf either; printf '\\n'",
    "[[ ! -e /nonexistent_zz ]] && printf neg; printf '\\n'",
    "[[ 5 -gt 3 ]] && printf gt; printf '\\n'",
    "[[ 5 -eq 05 ]] && printf numeq; printf '\\n'",
    "[[ a < b ]] && printf lt; printf '\\n'",
    "[[ abc =~ b ]] && printf match; printf '\\n'",
    "[[ abc =~ ^a.c$ ]] && printf anchored; printf '\\n'",
    "[[ abc =~ ^b ]] || printf nomatch; printf '\\n'",
    "[[ \"a b\" == \"a b\" ]] && printf sp; printf '\\n'",
    "x=\"a b\"; [[ $x == \"a b\" ]] && printf unq; printf '\\n'",
    "x=; [[ -z $x ]] && printf empty; printf '\\n'",
    "[[ ( a == a ) && ( b == b ) ]] && printf grp; printf '\\n'",
    "[[ -f /etc/passwd && -r /etc/passwd ]] && printf rf; printf '\\n'",
    "[[ x == x ]]; printf '%s\\n' \"$?\"",
    "[[ x == y ]]; printf '%s\\n' \"$?\"",
    "(( 3 > 2 )) && printf gt; printf '\\n'",
    "(( 3 < 2 )) || printf notlt; printf '\\n'",
    "(( x = 5 + 3 )); printf '%s\\n' \"$x\"",
    "(( x = 5 )); (( x += 2 )); (( x *= 3 )); printf '%s\\n' \"$x\"",
    "(( x = 7 )); (( x %= 4 )); printf '%s\\n' \"$x\"",
    "(( x = 1 )); (( x <<= 3 )); printf '%s\\n' \"$x\"",
    "(( 0 )); printf '%s\\n' \"$?\"",
    "(( 1 )); printf '%s\\n' \"$?\"",
    "x=5; (( x++ )); printf '%s\\n' \"$x\"",
    "x=5; (( ++x )); printf '%s\\n' \"$x\"",
    "x=5; (( x-- )); printf '%s\\n' \"$x\"",
    "x=5; y=$(( x++ + 1 )); printf '%s %s\\n' \"$x\" \"$y\"",
    "x=5; y=$(( ++x + 1 )); printf '%s %s\\n' \"$x\" \"$y\"",
    "for (( i=0; i<3; i++ )); do printf '%s' \"$i\"; done; printf '\\n'",
    "for (( i=3; i>0; i-- )); do printf '%s' \"$i\"; done; printf '\\n'",
    "for (( i=0, j=10; i<3; i++, j-- )); do printf '%s,%s ' \"$i\" \"$j\"; done; printf '\\n'",
    "(( a = 2 ** 3 )); printf '%s\\n' \"$a\"",
    "printf '%s\\n' $(( 16#ff )) $(( 2#101 )) $(( 8#17 ))",
    "printf '%s\\n' $(( 0x10 + 010 ))",
    "x=010; printf '%s\\n' $(( x ))",
    "x=08; printf '%s\\n' $(( 10#$x ))",
    "echo x{1,2}y",
    "echo a{,b}c",
    "echo {a,b,c}{,x}",
    "printf '%s\\n' {a,b}",
    "x=abc; printf '%s\\n' \"${x:1}\" \"${x:1:1}\" \"${x: -1}\" \"${x:0:2}\" \"${x:5}\"",
    "x=abcdef; printf '%s\\n' \"${x:2:2}\" \"${x: -3:2}\" \"${x:(-2)}\"",
    "x=hello; printf '%s\\n' \"${x/l/L}\" \"${x//l/L}\" \"${x/#h/H}\" \"${x/%o/O}\" \"${x/xyz/Q}\"",
    "x=hello; printf '%s\\n' \"${x/l}\" \"${x//l}\"",
    "x=a.b.c; printf '%s\\n' \"${x//./-}\" \"${x/./-}\"",
    "x='a/b'; printf '%s\\n' \"${x//\\//_}\"",
    "x=abc; printf '%s\\n' \"${x/#a/X}\" \"${x/%c/X}\" \"${x/#b/X}\"",
    "x=aaa; printf '%s\\n' \"${x//a/bb}\" \"${x/a/}\" ",
    "x='*'; printf '%s\\n' \"${x//\\*/star}\"",
    "a=(p q r); printf '%s\\n' \"${#a[1]}\"",
    "x=abc; printf '%s\\n' \"${#x}\" \"${#x[@]}\"",
    "printf '%s\\n' $'a\\tb'",
    "printf '%s\\n' $'\\x41\\101A'",
    "printf '%s\\n' $'a\\nb' | wc -l | tr -d ' '",
    "printf '%s\\n' $'it\\'s'",
    "x=$'a\\tb'; printf '%s\\n' \"${#x}\"",
    "cat <<< \"here string\"",
    "x=val; cat <<< \"$x\"",
    "cat <<< 'lit $x'",
    "read -r a b <<< \"1 2\"; printf '%s %s\\n' \"$a\" \"$b\"",
    "read -r -a arr <<< \"x y z\"; printf '%s\\n' \"${#arr[@]}\"",
    "while read -r l; do printf '<%s>' \"$l\"; done <<< $'a\\nb'; printf '\\n'",
    "cat <(printf 'proc')",
    "diff <(printf 'a\\n') <(printf 'a\\n') && printf same; printf '\\n'",
    "i=0; while (( i < 3 )); do printf '%s' $i; (( i++ )); done; printf '\\n'",
    "i=0; until (( i >= 3 )); do printf '%s' $i; ((i++)); done; printf '\\n'",
    "case a in a) printf 1;& b) printf 2;; c) printf 3;; esac; printf '\\n'",
    "case ab in a?) printf yes;; esac; printf '\\n'",
    "case X in [[:upper:]]) printf up;; esac; printf '\\n'",
    "case 5 in [0-9]) printf d;; esac; printf '\\n'",
    "typeset -i n=10; n+=5; printf '%s\\n' \"$n\"",
    "let x=5+5; printf '%s\\n' \"$x\"",
    "let \"x = 3 * 3\"; printf '%s\\n' \"$x\"",
    "let x=2 y=3; printf '%s\\n' \"$((x+y))\"",
    "let 0; printf '%s\\n' \"$?\"",
    "let 1; printf '%s\\n' \"$?\"",
    "printf -v v '%s-%s' a b; printf '%s\\n' \"$v\"",
    "printf -v v '%05d' 42; printf '%s\\n' \"$v\"",
    "printf '%q\\n' \"it's\"",
    "printf '%q\\n' ''",
    "printf '%s\\n' \"${RANDOM:+set}\"",
    "printf '%s\\n' \"${SECONDS:+set}\"",
    "trap 'printf err\\n' ERR; false; printf after\\\\n",
    "set -o pipefail; false | true; printf '%s\\n' \"$?\"",
    "set -o pipefail; true | false | true; printf '%s\\n' \"$?\"",
    "printf 'x\\n' | { read -r v; printf '%s\\n' \"$v\"; }",
    "x=1; { x=2; } | cat; printf '%s\\n' \"$x\"",
    "shopt -q nonexistent 2>/dev/null; printf '%s\\n' \"$?\"",
    "set +o noclobber; printf x > /tmp/zshrs_nine_$$; set -o noclobber; (printf y > /tmp/zshrs_nine_$$) 2>/dev/null; cat /tmp/zshrs_nine_$$; rm -f /tmp/zshrs_nine_$$; printf '\\n'",
    "set -o noclobber; printf z >| /tmp/zshrs_nine_$$; cat /tmp/zshrs_nine_$$; rm -f /tmp/zshrs_nine_$$; printf '\\n'",
    "[ \"$(printf '%s' abc | wc -c | tr -d ' ')\" = 3 ] && printf ok; printf '\\n'",
    "type printf | grep -c .",
    "command -V printf >/dev/null 2>&1; printf '%s\\n' \"$?\"",
    "hash -r; printf '%s\\n' \"$?\"",
    "ulimit -n >/dev/null; printf '%s\\n' \"$?\"",
    "wait 999999 2>/dev/null; printf '%s\\n' \"$?\"",
    "kill -0 $$; printf '%s\\n' \"$?\"",
    "kill -l 2 2>/dev/null; printf '\\n'",
];

/// zsh-only corpus — zsh constructs with no equivalent in the other shells, so
/// they run ONLY against the real zsh (`--zsh` vs zsh). Each probe was a
/// measured `--zsh` divergence.
const ZSH_ONLY_CORPUS: &[&str] = &[
    "sleep 0.05 & p=$!; wait $p; wait $p 2>/dev/null; echo $?",
    "(( 1/0 )) 2>/dev/null; echo $?",
    // The command-name word is globbed in the shell (Src/exec.c:3350-3354,
    // before any fork), so a NOMATCH / bad pattern there ends the list.
    "print a; nope* x 2>/dev/null; print after",
    "print a; [b c 2>/dev/null; print after",
    "print a; h[b c]=2 foo 2>/dev/null; print after",
    // The same abort applies behind `command` / `nocorrect` / `builtin`
    // (the precommand walk, Src/exec.c:3104-3350); a chained prefix pair does not.
    "print a; command nope* x 2>/dev/null; print after",
    "print a; command -p nope* x 2>/dev/null; print after",
    "print a; builtin nope* x 2>/dev/null; print after",
    "print a; nocorrect nope* x 2>/dev/null; print after",
    "print a; command [b c 2>/dev/null; print after",
    "command command nope* x 2>/dev/null; print after",
    // GLOB_ASSIGN runs the RHS through globlist, which honours the `(N)`
    // qualifier's per-glob NULL_GLOB (Src/exec.c:2611-2620, glob.c:1567-1569).
    "setopt globassign; x=nope*(N); print \"[$x]\" $?",
    // `[0,0]` under KSH_ZERO_SUBSCRIPT is the first element/char
    // (Src/params.c:2155-2160); strict mode keeps the empty range.
    "setopt kshzerosubscript; s=hello; a=(a b c); print \"[${s[0,0]}] [${a[0,0]}] [${s[0]}] [${a[0]}]\"",
    // `typeset NAME` on a zsh/parameter special hash lists its scan
    // (printparamnode with PRINT_INCLUDEVALUE, Src/params.c:6304-6310).
    "zmodload zsh/parameter; alias q=w; typeset aliases | grep -c '\\[q\\]=w'",
    "zmodload zsh/parameter; typeset functions nameddirs",
    // Without EXTENDEDGLOB the `(dir/)#` repeat is not recognised, so the group
    // holding a `/` is a bad pattern (Src/glob.c:745-748 tests zpc_special[ZPC_HASH]).
    "print a/(*/)#f 2>&1; print rc=$?",
    "print (nosuch_d/)#f 2>&1; print rc=$?",
    "setopt extendedglob; print (nosuch_d/)#f 2>&1; print rc=$?",
    // A `-` / `+` in a literal glob word must not hoist its expansion ahead of
    // earlier words: the FIRST failing glob is the one named (Src/exec.c:3755-3757).
    "print nosuch_a* nosuch_b-* 2>&1; print rc=$?",
    "print nosuch_a(.) nosuch_b(m-1) 2>&1; print rc=$?",
    "print nosuch_a(m0) nosuch_b+(.) 2>&1; print rc=$?",
    // OCTALZEROES: a variable holding `010` is an octal literal when read in
    // arithmetic (Src/params.c:2639 matheval(getstrvalue)).
    "setopt octalzeroes; x=010; print $((x)) $((x + 1)) $((010))",
    "setopt octalzeroes; x=-010; y=0; print $((x)) $((y + 1))",
    "x=010; print $((x)) $((x + 1))",
    // `%1 &` is `bg %1`, `%1 &!` is `disown %1` run in the shell (Src/exec.c:3019-3027);
    // the diagnostics go to /dev/null because zsh 5.9 words them with a trailing period.
    "(sleep 1 & %1 & print $?; %1 &!; print done) 2>/dev/null",
    // Bare `$name[sub]` under KSH emulation inside a larger word is `${name[0]}`
    // followed by the literal `[sub]` (Src/params.c:2342-2358).
    "setopt ksharrays; typeset -A h=(a 1); print x$h[a] 2>&1",
    // CSH_NULL_GLOB drops the name silently; the next word becomes the command.
    "setopt cshnullglob; nope* nosuchcmd_zz 2>/dev/null; print rc=$?",
    // A command word starting with `%` is a job spec: `%1` runs `fg %1`.
    "%1 2>/dev/null; print rc=$?",
    // `%n` is the passwd name of the real uid (get_username), not $USER.
    "USER=bogus_zz; LOGNAME=bogus_zz; print -P %n | grep -c bogus_zz",
    // An unbraced `$name[@]` inside double quotes splices like the braced form.
    "a=(a b); print -l \"x$a[@]y\"",
    "a=(a b); print -l \"x$a[@]\" \"$a[@]y\"",
    // Strict zero subscript: `s[0]` / `a[0,0]` are off the start of the index
    // range, so the reference is unset (Src/params.c:2146-2171, subst.c:2806).
    "s=hello; print \"[${s[0]-x}]\" \"[${+s[0]}]\" \"[${s[0,0]-x}]\" \"[${+s[0,0]}]\"",
    "a=(a b); print \"[${a[0,0]-x}]\" \"[${+a[0,0]}]\" \"[${a[0]-x}]\"",
    // A forward `(r)` miss on a scalar is a set, empty substring; `(R)` is unset.
    "s=ab; print \"[${s[(r)z]-x}]\" \"[${+s[(r)z]}]\" \"[${s[(R)z]-x}]\" \"[${+s[(R)z]}]\"",
    // `typeset NAME` on an assoc quotes the key like `typeset -p` does.
    "typeset -A h; h=('k 1' 'v 1' k2 v2); typeset h",
    // A zsh/parameter table nothing has loaded is shown by name only.
    "typeset aliases; typeset -m 'comm*'",
    // `-pcre-match` always speaks PCRE (`\\d` is a digit class), whatever
    // REMATCHPCRE says; that option only picks the engine for `=~`.
    "zmodload zsh/pcre 2>/dev/null; [[ a1 -pcre-match '\\d' ]]; echo $?; [[ a1 -pcre-match '\\D' ]]; echo $?",
    // zpty diagnostics go through zwarnnam (`zsh:zpty:N: msg`).
    "zmodload zsh/zpty 2>/dev/null; zpty nonesuch 2>&1; echo rc=$?",
    // `zstyle -L [context [style]]` takes at most two arguments.
    "zstyle ':a:b' x 1; zstyle -L ':a:b' x y 2>&1; echo rc=$?",
    // A single forced-split field returns the offset of the word's SUFFIX, so
    // the suffix's own expansions (`$b[2]`) are still scanned.
    "setopt shwordsplit; b=(q r); x=ab; print ${x}$b[2] ${b[1]}$b[1] ${x}${x}$b[2]",
    // `(%)` follows C's `isarr`: a subscript pick or a (j) join leaves a scalar.
    "a=('%%1' '%%2'); print -r -- ${(%)a[2]} ${(%)a[2,2]} ${(%j:,:)a} ${(%)a}",
    // zstyle takes only -L -e -d -s -b -a -t -T -m -q -g; anything else fails.
    "zstyle -n :x foo 2>/dev/null; print $?; zstyle -l 2>/dev/null; print $?",
    // `**/` with an `(e:…:)` qualifier must stay serial: workers raced on $REPLY.
    "d=$(mktemp -d); mkdir -p $d/a/b $d/c; : > $d/a/b/y; : > $d/c/z; : > $d/a/x; cd $d; print -l **/*(.e:'[[ $REPLY == *y ]]':); cd /; command rm -rf $d",
    // `+=` on a scalar reads the old text through the param's getfn.
    "typeset -T FOO foo; foo=(a b); FOO+=:d; print $foo; PATH=/a:/b; PATH+=:/c; print -l $path",
    // ── expanded corpus (differential sweep vs the real shells) ──
    "a=(x y z); print $a[1] $a[-1] $a[2,3] $#a",
    "a=(x y z); print ${a[(i)y]} ${a[(r)y]} ${a[(I)y]}",
    "a=(c a b); print ${(o)a} ${(O)a} ${(on)a}",
    "a=(10 9 100); print ${(on)a} ${(On)a}",
    "a=(a b c); print ${(j:,:)a} ${(j:-:)a[1,2]}",
    "x=\"a,b,c\"; print ${(s:,:)x}; print ${#${(s:,:)x}}",
    "x=hello; print ${(U)x} ${(C)x} ${(L)${(U)x}}",
    "x=\"hello world\"; print ${(C)x}",
    "x=abc; print ${(l:6::-:)x} ${(r:6::-:)x}",
    "x=abc; print ${(l:2:)x}",
    "x='a b'; print ${(q)x} ${(qq)x} ${(qqq)x} ${(qqqq)x}",
    "x='a b'; print ${(Q)${(q)x}}",
    "x=abc; print ${(e)x} ${(%)x}",
    "print ${(%):-%n} | grep -c .",
    "x=a:b; print ${(s.:.)x}",
    "x=(a b c); print ${(w)#x}",
    "x=\"a b  c\"; print ${(w)#x} ${(s: :)x}",
    "x=\"one two\"; print ${${=x}[2]}",
    "a=(1 2 3); print ${(@)a}; print \"${(@)a}\"",
    "a=(1 2 3); print ${(M)a:#[12]} ${(R)a:#[12]}",
    "a=(foo bar baz); print ${(M)a:#b*}",
    "a=(foo bar baz); print ${a:#b*}",
    "a=(foo bar); print ${(k)a} 2>/dev/null; print ok",
    "typeset -A h; h=(a 1 b 2); print ${(ok)h} ${(ov)h}",
    "typeset -A h; h=(a 1); h[b]=2; print ${#h} ${h[a]}${h[b]}",
    "typeset -A h; h=(a 1 b 2); print -l ${(ok)h}",
    "typeset -A h; h=(a 1); print ${+h[a]} ${+h[z]}",
    "typeset -A h; h=(a 1); unset 'h[a]'; print ${#h}",
    "typeset -A h; h[a b]=1; print ${h[a b]} ${(k)h}",
    "x=(a b c); print ${x[2,-1]} ${x[-2,-1]} ${x[1,0]}",
    "x=abc; print ${x[2]} ${x[-1]} ${x[2,3]} ${x[1,1]}",
    "x=hello; print ${x:1:3} ${x: -3}",
    "x=hello; print ${x[2,4]} ${x[(i)l]} ${x[(I)l]}",
    "x=a/b/c; print ${x:h} ${x:t} ${x:r} ${x:e}",
    "x=/a/b.tar.gz; print ${x:h} ${x:t} ${x:r} ${x:e} ${x:t:r}",
    "x=foo.c; print ${x:r}.o ${x:e}",
    "x=abc; print ${x:u} ${x:l} 2>/dev/null",
    "x=abc; print ${x:s/b/X/} ${x:gs/b/X/}",
    "x=aabb; print ${x:gs/a/X/}",
    "x=a.b.c; print ${x//./-} ${x/./-}",
    "x=abc; print ${x/(#b)(b)/[$match[1]]} 2>/dev/null",
    "setopt extendedglob; x=abc; print ${x/(#b)(b)/[$match[1]]}",
    "setopt extendedglob; x=aaa; print ${x//a##/X}",
    "setopt extendedglob; print *(N)",
    "setopt extendedglob; [[ abc = (#i)ABC ]] && echo ci",
    "setopt extendedglob; [[ abc = a(#c1,2)bc ]] && echo rep",
    "setopt extendedglob; print ^x | grep -c .",
    "setopt nullglob; print /nonexistent_zz*; print done",
    "setopt nonomatch; print /nonexistent_zz*",
    "unsetopt nomatch; print /nonexistent_zz*",
    "print /nonexistent_zz* 2>&1 | head -1",
    "print -r -- $'a\\tb'",
    "print -l a b c",
    "print -r '\\n'",
    "print '\\n' | wc -c",
    "print -z foo 2>/dev/null; echo ok",
    "print -u2 err 2>/dev/null; echo ok",
    "print -C 2 a b c d",
    "print -c a b c",
    "print -o c a b",
    "print -O c a b",
    "print -f '%s-%s\\n' a b",
    "print -R -n a",
    "print -x 4 $'a\\tb'",
    "echo -n a; echo",
    "printf '%s\\n' a b",
    "printf '%q\\n' 'a b'",
    "printf '%5.2f|%-5d|%05d\\n' 3.14159 42 42",
    "printf -v v '%s' hi; print $v",
    "typeset -i n=10; print $((n/3)) $((n%3)) $((n**2))",
    "typeset -F 2 f=3.14159; print $f",
    "typeset -E 3 e=1234.5678; print $e",
    "typeset -F f=1; print $f $((f/3))",
    "print $((10/3.0))",
    "print $((3.0*2))",
    "print $(( 2 ** 0.5 ))",
    "print $((1e3))",
    "print $(( int(3.7) )) $(( float(3) ))",
    "print $(( abs(-3) )) 2>/dev/null",
    "print $(( sqrt(16) ))",
    "print $((0x1F)) $((8#17)) $((2#101)) $((16#FF))",
    "print $(( 7 / 2 )) $(( -7 / 2 )) $(( 7 % 3 )) $(( -7 % 3 ))",
    "print $((9223372036854775807))",
    "print $(( 5 <=> 3 )) 2>&1 | head -1",
    "integer i=5; (( i++ )); print $i",
    "integer i=5; (( i += 2 )); print $i",
    "(( x = 3 + 4 )); print $x",
    "print $(( x = 5, x * 2 ))",
    "for i in {1..3}; do print $i; done",
    "for i in {a..c}; do print $i; done",
    "for (( i=0; i<3; i++ )); do print $i; done",
    "repeat 3 do print x; done",
    "repeat 2 print y",
    "foreach x (a b); print $x; end 2>/dev/null; echo ok",
    "if (( 1 )); then print yes; fi",
    "if [[ -n x ]] { print braces } 2>/dev/null; echo ok",
    "for x in a b; print $x",
    "case x in (x) print m;; esac",
    "case $((1+1)) in 2) print two;; esac",
    "{ print a; print b } | cat",
    "f() { print $#; }; f a b c",
    "f() { print ${(@)argv}; }; f a b",
    "f() { print $funcstack[1]; }; f",
    "f() { local x=1; print $x; }; f",
    "f() { typeset -g y=2; }; f; print $y",
    "f() { typeset x=1; print $x; }; x=out; f; print $x",
    "f() { return 3 }; f; print $?",
    "function f { print kw }; f",
    "function f() { print kw2 }; f",
    "functions f 2>/dev/null | head -1; print ok",
    "autoload -U zmv 2>/dev/null; print ok",
    "zmodload zsh/mathfunc 2>/dev/null; print $(( sin(0) ))",
    "zmodload zsh/zutil 2>/dev/null; zparseopts -E -D a=o -- -a x; print $o",
    "zparseopts -E -D -- a=o b: -a x 2>/dev/null; print $o",
    "set -- -a -b val x; zparseopts -E a=o b:=p; print $o $p $#",
    "zstyle ':x:y' k v; zstyle -s ':x:y' k r; print $r",
    "zstyle ':x:*' k v2; zstyle -s ':x:y' k r; print $r",
    "zstyle -t ':a' nonexist; print $?",
    "zstyle ':a' b on; zstyle -t ':a' b; print $?",
    "bindkey -l | head -1 | grep -c .",
    "bindkey -M emacs '^A' 2>/dev/null | head -1 | grep -c .",
    "setopt | head -0; print ok",
    "unsetopt | head -0; print ok",
    "setopt kshglob; [[ abc = @(abc|x) ]] && echo kshglob",
    "setopt shwordsplit; x='a b'; set -- $x; print $#",
    "unsetopt shwordsplit; x='a b'; set -- $x; print $#",
    "setopt ksharrays; a=(x y); print ${a[0]} ${a[1]}",
    "setopt rcquotes; print 'it''s'",
    "setopt interactivecomments; print a # comment",
    "setopt pushdignoredups; print ok",
    "setopt noclobber; print x > /dev/null; print ok",
    "setopt printexitvalue 2>/dev/null; print ok",
    "setopt errexit; false; print not-reached",
    "setopt errreturn; f() { false; print nr; }; f; print after",
    "setopt pipefail; false | true; print $?",
    "setopt localoptions; print ok",
    "setopt globsubst; x='*zz_no'; print $x",
    "unsetopt globsubst; x='*'; print $x",
    "setopt rematchpcre 2>/dev/null; print ok",
    "setopt cbases; print $(( 0xff )) $(( [#16] 255 ))",
    "print $(( [#16] 255 )) $(( [##16] 255 ))",
    "setopt octalzeroes; print $(( 010 ))",
    "unsetopt octalzeroes; print $(( 010 ))",
    "[[ \"a b\" = \"a b\" ]] && echo q",
    "[[ -z \"\" && ( 1 = 1 || 2 = 3 ) ]] && echo grp",
    "[[ abc = a* ]] && echo g1",
    "[[ abc == a? ]] || echo g2",
    "[[ abc = (a|b)* ]] && echo alt",
    "[[ x = (#i)X ]] 2>/dev/null; echo $?",
    "[[ a -nt b ]] 2>/dev/null; echo $?",
    "[[ -o noglob ]] || echo glob-on",
    "[[ -o nonexist_zz ]] 2>/dev/null; echo $?",
    "[[ 1 -lt 2 ]] && [[ 2 -lt 3 ]] && echo chain",
    "[[ a < b ]] && echo lt",
    "[[ $(( 1+1 )) = 2 ]] && echo arith",
    "[[ -n ${a:+x} ]] || echo unset",
    "$(print -r -- echo cmdsub-cmd)",
    "print ${${:-a}:-b}",
    "print ${${(U)${:-abc}}}",
    "x=y; y=z; print ${(P)x}",
    "typeset -i I=1; print ${(t)I}",
    "typeset -A A; print ${(t)A}",
    "typeset -a B; print ${(t)B}",
    "print ${(t)nonexist_zz}",
    "print ${#${(f)\"$(print -l a b c)\"}}",
    "print ${(f)\"$(print -l a b c)\"}",
    "print \"${(F)${(f)\"$(print -l a b)\"}}\"",
    "print ${(z)${:-a b \"c d\"}}",
    "print ${(Az)${:-a b}}",
    "print $'é' | wc -c",
    "print ${#${:-é}}",
    "print ${(l:5::0:)${:-42}}",
    "print ${${:-abc}[2,3]}",
    "print ${(S)${:-abcabc}#b*c} ",
    "print ${${:-abcabc}#b*c}",
    "print ${${:-abcabc}##a*c}",
    "print ${(*)${:-abc}#a*} 2>/dev/null",
    "x=(a b c); x[2]=(); print $x 2>/dev/null; echo ok",
    "x=(a b c); x[2,3]=(); print $x",
    "x=(a b c); x[2]=q; print $x",
    "x=(a b c); x[5]=e; print ${#x} $x",
    "x=(a b c); x+=(d); x+=e 2>/dev/null; print $x",
    "x=a; x+=b; print $x",
    "x=(a b); y=($x c); print $y",
    "x=(a b); print ${x[@]:#a}",
    "x=(a b); print ${x[*]}; print \"${x[*]}\"; print \"${x[@]}\"",
    "x=(a b); IFS=:; print \"${x[*]}\"; unset IFS",
    "x=(1 2 3); print $((x[1]+x[2]))",
    "x=(); print ${#x} ${x[1]-unset}",
    "x=(); print \"[$x]\"; print \"[${x[@]}]\"",
    "typeset -U u=(a b a c); print $u",
    "typeset -g -a ga=(1 2); print $ga",
    "typeset -T TP tp=(a b); print $TP; tp+=(c); print $TP",
    "typeset -r R=1; R=2 2>/dev/null; print $R",
    "typeset +r R 2>/dev/null; print ok",
    "typeset -m 'PAT*' 2>/dev/null; print ok",
    "typeset -f f 2>/dev/null; print ok",
];

/// bash-only corpus — constructs where bash differs from the Korn shells and
/// zsh, so they run ONLY against the real bash (`--bash` vs bash). Each probe
/// was a measured `--bash` divergence.
const BASH_ONLY_CORPUS: &[&str] = &[
    "echo -e \"a\\x\" \"\\x41\"; echo \"b\\x\"; printf \"%s\\n\" $'d\\x'",
    "a=(x y z); a[5]=w; unset \"a[1]\"; echo \"${a[@]:1:2}\" \"${a[@]:2}\" \"${a[@]:3}\" \"${a[@]:0:2}\" \"${a[@]: -2}\"; echo \"[${a[@]: -9}]\"",
    "a=$BASHPID; b=$(echo $BASHPID); (d=$BASHPID; echo $((d==a))); echo $((a==$$)) $((b==a)); echo x | { echo $((BASHPID==a)); }",
    "x=a/b; echo \"[${x///:}] [${x///}] [${x////}] [${x///b}] [${x//a/}]\"",
    "command -x 2>/dev/null; echo $?",
    "command -x echo hi 2>/dev/null; echo $?",
    "command -pv echo; command -p -V echo; command -pV ls",
    "eval \"echo '\" 2>/dev/null; echo $?",
    "eval 'echo (' 2>/dev/null; echo $?",
    "eval false; echo $?",
    "set -- a b; shift abc 2>/dev/null; echo $?",
    "type -p echo; type -p ls | grep -c ls",
    "sleep 0.05 & p=$!; wait $p; wait $p 2>/dev/null; echo $?",
    "read -t 0 x </dev/null; echo $?",
    "echo a | { read -t 0 x; echo $?; }",
    "sleep 0.4 & sleep 0.4 & jobs; wait",
    "sleep 0.4 & jobs; wait",
    "cd //tmp; pwd; echo $PWD",
    "cd ///tmp; pwd",
    "cd /tmp//; pwd",
    "echo \"${GROUPS[@]}|${#GROUPS[@]}|${GROUPS[0]}\"",
    "f() { echo hi; }; g() { :; }; set | grep -A3 '^f '",
    "ZZ_V=; set | grep '^ZZ_V='",
    "echo \"[${CDPATH-u}][${MAILCHECK-u}][${TMOUT-u}]\"",
    "(exit abc) 2>/dev/null; echo $?",
    "f() { return abc; }; f 2>/dev/null; echo $?",
    "type if",
    "export X=1; export -n X; env | grep -c '^X='; echo $X",
    "set -o nosuchopt_zz 2>/dev/null; echo $?; echo after",
    "printf '%3$s %1$s\\n' a b c 2>/dev/null; echo $?",
    // `${!#}` is indirection through `$#`: the LAST positional.
    "set -- a b c; printf '[%s]' \"${!#}\"; printf '\\n'",
    // Integer arithmetic wraps at 64 bits (zsh truncates the literal instead).
    "echo $((-9223372036854775808)) $((9223372036854775807+1))",
    // bash 5.2 `patsub_replacement`: `&` is the matched text, `\\&` a literal `&`.
    "v=abc; printf '%s\\n' \"${v/b/&}\" \"${v/b/\\&}\" \"${v//[ac]/<&>}\" \"${v/b/&&}\" \"${v/b/\\\\&}\"",
    "v=abc; shopt -u patsub_replacement; printf '%s\\n' \"${v/b/&}\"",
    // `trap -p SIG...` lists only the named signals.
    "trap 'echo x' INT USR1; trap -p USR1",
    "trap 'echo x' INT USR1; trap -p INT USR1",
    // `local` / `return` outside a function are errors that do not abort.
    "local x=1 2>/dev/null; echo rc=$?",
    "return 3 2>/dev/null; echo rc=$?",
    // Sourcing a missing file returns 1 and the script continues.
    ". /nonexistent 2>/dev/null; echo rc=$?",
    // Function listings: `declare -f` layout and the `declare -F` name list.
    "f() { echo a; echo b; }; declare -f f",
    "f() { echo a; }; declare -F f; declare -F; declare -F nofunc; echo rc=$?",
    // `${unset@Q}` is empty; set-but-empty quotes to `''`.
    "unset u; printf '[%s]' \"${u@Q}\"; u=; printf '[%s]' \"${u@Q}\"; printf '\\n'",
    // A function body must be a compound command; `$((1.5))` is an error
    // (integer-only arithmetic); `$HOSTNAME` is set; `$!` is empty until a
    // background job exists.
    "f() echo hi; f",
    "echo $((1.5+1))",
    "[ -n \"$HOSTNAME\" ] && echo set",
    "printf '[%s]' \"$!\"; printf '\\n'",
    "true & wait; [ -n \"$!\" ] && echo set",
    // Nested functions: LINENO is the absolute line.
    "g() {\necho $LINENO\nh() { echo $LINENO; }\nh\n}\ng",
    // `trap … RETURN` fires when the function that installed it returns; a
    // caller's RETURN trap is not inherited by the functions it calls.
    "f() { trap 'echo ret' RETURN; echo in; }; f; echo after",
    "g() { echo g; }; f() { trap 'echo ret-f' RETURN; echo in; }; f; g; f; trap 'echo top' RETURN; g; h() { g; echo hh; }; h; trap -p RETURN",
    // `hash`: empty notice, hits column, -p / -t / -d / -r / -l.
    "hash",
    "hash -p /bin/ls foo; hash -t foo; hash",
    "ls / >/dev/null; ls / >/dev/null; hash; hash -l",
    "hash ls; hash -r ls; hash",
    "hash -p /bin/ls foo; hash -d foo; hash",
    "hash nosuch_cmd_x 2>/dev/null; echo rc=$?",
    // `enable -n` disables a builtin (lookup falls through to PATH) and the
    // listings use `enable [-n] NAME`.
    "enable -n echo; type -t echo; enable -n; enable echo; type -t echo",
    "enable -n echo; enable -a | grep -n 'enable -n'",
    "enable -s | head -3; enable | wc -l",
    "enable -n nosuch_builtin 2>/dev/null; echo rc=$?",
    // `declare -f` lays compound commands out the way bash's print_cmd does.
    "f() { if true; then echo a; elif false; then echo b; else echo c; fi; }; declare -f f",
    "f() { for i in 1 2; do echo $i; done; while false; do :; done; until true; do :; done; }; declare -f f",
    "f() { case $1 in a|b) echo y; echo z;; *) echo n;; esac; (echo s; echo t); { echo g; echo h; } > /dev/null; echo end; }; declare -f f",
    "f() { for ((i=0;i<2;i++)); do echo $i; done; g() { echo inner; }; }; declare -f f",
    // Brace expansion precedes parameter expansion in bash.
    "x=b; echo $x{,a}",
    "x=; echo $x{,a}",
    "x=; printf '[%s]' \"$x\"{,a} \"\"{,a} $x\"\"{,a} a$x{,b}; printf '\\n'",
    "x='1 2'; echo {a,b}$x; for i in {1,2}$x; do echo \"[$i]\"; done",
    "x=q; echo {a,b}${x:-z,y}$((1+2)){c,d}`echo e`",
    // A quoted-empty part keeps an otherwise-empty expansion as one empty word.
    "x=; printf '[%s]' $x\"\" a ''$x \"$x\"$x; printf '\\n'",
    // `wait -n` returns the status of whichever child finishes first (127 with none).
    "(sleep 0.1; exit 4) & (sleep 0.3; exit 5) & wait -n; echo $?; wait -n; echo $?; wait -n; echo $?",
    "wait -n; echo $?",
    // `mapfile -d ''` splits on NUL; `-O` starts at an origin and keeps the array.
    "printf 'a\\0b\\0' | { mapfile -t -d '' arr; echo ${#arr[@]} ${arr[1]}; }",
    "printf 'a\\nb\\nc\\n' | { mapfile -s 1 -O 3 arr; echo ${!arr[@]} ${arr[3]}; }",
    "arr=(x y); printf 'a\\nb\\n' | { mapfile -t -O 1 arr; echo ${arr[@]}; }",
    // bash-only `set` letters.
    "set -B; echo rc=$?; set +B; echo rc=$?; set +h; set -h; echo rc=$?; set -E -H -P -T; set -o | grep -E 'braceexpand|errtrace|histexpand|physical|functrace'",
    // `caller` reads BASH_LINENO / FUNCNAME / BASH_SOURCE; FUNCNAME is unset outside functions.
    "f() { caller; echo rc=$?; caller 0; echo rc=$?; caller 1; echo rc=$?; }; f; caller; echo top rc=$?",
    "f() { echo \"${FUNCNAME[@]}\"; g; }; g() { echo \"${FUNCNAME[@]}\"; }; f; echo \"[${FUNCNAME[@]}]\"",
    // `type` reports a missing name on stderr; `type -P` forces a $PATH search.
    "type nosuch_cmd_x 2>/dev/null; echo rc=$?; type -P ls; type -P nosuch_cmd_x; echo rc=$?",
    // A bare attribute query lists in the reusable `declare -X NAME=\"v\"` form.
    "declare -x BX=1; declare -x | grep '^declare -x BX='; export | grep '^declare -x BX='",
    "declare -r BR=1; declare -r | grep '^declare -r BR='; readonly | grep '^declare -r BR='",
    "declare -i BI=1; declare -i | grep BI=; declare -A BM=([a]=1); declare -A | grep BM=",
    // Arithmetic is integer-only: a negative exponent is an error.
    "echo $((2**-1)); echo rc=$?; echo $((2**3))",
    // Values with control characters print as one `$'…'` (printf %q, declare -p).
    "printf '%q|' $'a\\nb' 'a b' ''; echo",
    "x=$'a\\nb'; declare -p x; a=(x $'y\\nz' 'q r'); declare -p a",
    // bash turns extglob on while parsing `[[ ]]`, so the ksh groups work in the
    // pattern with the `extglob` shopt off.
    "[[ abc == @(a|x)bc ]] && echo y; echo done",
    "[[ abc == +(a|b)c ]] && echo y; echo done",
    "[[ abc != @(a|x)bc ]] && echo y; echo done",
    "[[ abc == !(x)bc ]] && echo y; echo done",
    "[[ abc == a?(x)bc ]] && echo y; echo done",
    // Without `inherit_errexit` a command substitution runs with -e cleared.
    "set -e; echo $(false; echo hi)",
    "set -e; x=$(false; echo hi); echo \"[$x]\"",
    "shopt -s inherit_errexit; set -e; x=$(false; echo hi); echo \"[$x]\"; echo no",
    // `printf %b` reads a bare `\NNN` (1-3 octal digits) as well as `\0NNN`.
    "printf '%b\\n' '\\x41\\101\\0101'",
    "printf '%b|' '\\101' '\\1' '\\18' '\\0101'; echo",
    // $BASH_VERSION / $BASH_VERSINFO / $HOSTTYPE / $MACHTYPE track bash 5.x:
    // `5.N.P(1)-release`, a 6-element array whose tail is the GNU triple.
    "[[ $BASH_VERSION == 5.*'(1)-release' ]] && echo ver; echo ${BASH_VERSINFO[0]} ${BASH_VERSINFO[4]} ${#BASH_VERSINFO[@]}; echo $HOSTTYPE; echo ${MACHTYPE%%-*}; [[ ${BASH_VERSINFO[5]} == $MACHTYPE ]] && echo same",
    // EPOCHSECONDS / EPOCHREALTIME are always-on specials (no module load);
    // EPOCHREALTIME has microsecond granularity.
    "[ \"$EPOCHSECONDS\" -gt 1000000000 ] && echo secs; case $EPOCHREALTIME in *.??????) echo real;; esac; [ -n \"${SRANDOM+x}\" ] && echo sr",
    // `local -` restores the shell option flags on function return.
    "f() { local -; set -f; echo $-; }; set +f; f; echo $-",
    // `wait -f` is accepted and waits for termination.
    "sleep 0.1 & wait -f $!; echo rc=$?; sleep 0.1 & wait -f; echo rc=$?",
    // A quoted part of a `=~` pattern is matched as a literal string.
    "[[ abc =~ \"b+\" ]]; echo $?; [[ 'b+' =~ \"b+\" ]]; echo $?; [[ abc =~ \"a.c\" ]]; echo $?; [[ a.c =~ \"a.c\" ]]; echo $?; [[ abc =~ b+ ]]; echo $?; [[ 'a(b' =~ a\\(b ]]; echo $?",
    // With `expand_aliases` off (the non-interactive default) `type` and
    // `command -v` do not see aliases; with it on, `type` says `aliased to`.
    "alias e='echo hi'; type e 2>/dev/null; echo rc=$?; command -v e; echo rc=$?",
    "shopt -s expand_aliases; alias e='echo hi'; type e; type -t e; command -v e",
    // ERR trap: not inherited by functions without `set -E`; with it the
    // failing call fires the trap again after the inner command did.
    "trap 'echo ERR' ERR; f() { false; echo in; }; f; false; echo done",
    "set -E; trap 'echo ERR' ERR; f() { false; }; f; echo done",
    // DEBUG fires before each command with $BASH_COMMAND set; function
    // definitions and (without `set -T`) function bodies are skipped.
    "trap 'echo DBG $BASH_COMMAND' DEBUG; f() { echo in; }; f; echo out",
    // `declare -f` prints a here-document at column 0 followed by a blank line.
    "f() { cat <<EOF2\nhello $1\nEOF2\n}; declare -f f",
    "f() { if true; then cat <<\"Q\"\nbody\nQ\nfi; echo x; }; declare -f f",
    // `compgen`: `--` ends options, -P/-S/-X, no sort, status 1 on no match,
    // and the -A action names.
    "compgen -W 'c a b' -- b; echo rc=$?; compgen -W 'c a b' -- x; echo rc=$?; compgen -W 'c a b'",
    "compgen -W 'ab ac b' -P p- -S -s a; compgen -W 'ab ac b' -X 'ab' a; compgen -W 'ab ac b' -X '!ab' a",
    "compgen -A signal -- SIGH; compgen -A keyword -- wh; compgen -A shopt -- ext; compgen -A builtin -- ech; compgen -A setopt -- errt",
    "a=(1); f() { :; }; compgen -A function; compgen -A arrayvar -- a; compgen -A variable -- BASH_VERSI",
    // `complete -p [NAME...]` prints specs; a missing one is status 1.
    "complete -W 'x y' foo; complete -p; complete -p foo; complete -p nope 2>/dev/null; echo rc=$?",
    // `history -c` clears the list; `fc -l` on an empty list is silent status 0.
    "history -c; echo rc=$?; history | wc -l | tr -d ' '; fc -l; echo rc=$?",
    // `declare -p` of a nameref lists the TARGET name with the `n` attribute.
    "x=1; declare -n r=x; r=5; echo $x; declare -p r; declare -n | head -1",
    // `${!name}` indirects through a subscripted value; `${!a[*]}` is the index list.
    "a=(x y z); n=a; echo ${!n}; n='a[1]'; echo ${!n}; n='a[@]'; echo ${!n}; echo ${!a[@]}; echo ${!a[*]}; declare -A m=([k]=v); n='m[k]'; echo ${!n}",
    // `shopt -s lastpipe` runs the last pipeline stage in the current shell.
    "shopt -s lastpipe; echo a | read x; echo \"[$x]\"; shopt -u lastpipe; echo b | read y; echo \"[$y]\"",
    // `coproc`: COPROC=(read-fd write-fd) and COPROC_PID are script-visible.
    "coproc cat; echo hi >&${COPROC[1]}; read -t 5 -u ${COPROC[0]} x; echo \"got $x\"; [ -n \"$COPROC_PID\" ] && echo pid; kill $COPROC_PID 2>/dev/null",
    // `$'...'`: `\\cX` is a control char; an unknown escape and a digitless
    // `\\x` / `\\u` keep their backslash.
    "printf '%s|' $'\\z' $'\\x' $'\\u' $'\\q'; printf '%s' $'\\cA\\c?' | od -An -c",
    // A bare associative-array name reads key \"0\".
    "declare -A m=([k]=v); echo \"[$m]\" \"[${#m}]\" \"[${m:-d}]\"; m[0]=z; echo \"[$m]\" \"[${#m}]\"",
    "declare -A m=([k]=v); echo ${m@A}; echo \"[${m@K}]\"; m[0]=z; echo ${m@A}",
    // A syntax error in a `-c` string runs nothing and exits 2.
    "echo before; x=abc; case $x in +(a|b|c)) echo plus;; esac",
    // ── expanded corpus (differential sweep vs the real shells) ──
    "declare -A m=([a]=1); [[ -v m[a] ]] && echo has",
    "declare -A m=([a]=1); echo \"${m[a]+set}\" \"${m[b]+set}\"",
    "declare -n r=x; x=1; echo $r",
    "declare -n r=x; r=2; echo $x",
    "x=1; declare -n r=x; unset r; echo \"[$x]\"",
    "declare -i n=2+3; echo $n",
    "declare -i n; n=\"2*4\"; echo $n",
    "declare -ir c=5; echo $c",
    "declare -l l=ABC; echo $l; l=XyZ; echo $l",
    "declare -u u=abc; echo $u",
    "declare -f nonexist_zz >/dev/null; echo $?",
    "f() { echo hi; }; declare -f f",
    "x=abc; echo ${x^} ${x^^} ${x^^[ab]}",
    "x=ABC; echo ${x,} ${x,,} ${x,,[AB]}",
    "x=hello; echo ${x@U} ${x@u} ${x@L}",
    "x='a b'; echo ${x@Q}",
    "x=abc; echo ${x@a}; declare -i y=1; echo ${y@a}",
    "a=(x y); echo ${a[@]@Q}",
    "x=hello; y=x; echo ${!y}",
    "prefix_a=1; prefix_b=2; echo ${!prefix_*}",
    "prefix_a=1; prefix_b=2; echo ${!prefix_@}",
    "a=(x y z); echo ${!a[@]}",
    "x=hello; echo ${x:1:2} ${x: -2}",
    "a=(0 1 2 3 4); echo ${a[@]:2} ${a[@]: -2} ${a[@]:1:2}",
    "set -- a b c d; echo ${@:2} ${@:2:2} ${@: -1}",
    "set -- a b c; echo ${*:2}",
    "x=abc; echo ${x:1} ${x:-d} ${x:+e}",
    "shopt -s extglob; x=foo; case $x in @(foo|bar)) echo m;; esac",
    "shopt -s extglob; [[ abc == +(a|b|c) ]] && echo plus",
    "shopt -s extglob; [[ abc == !(x*) ]] && echo not",
    "shopt -s extglob; x=aXb; echo ${x//+([A-Z])/_}",
    "shopt -s nocasematch; [[ ABC == abc ]] && echo ci",
    "shopt -s nocasematch; case ABC in abc) echo ci;; esac",
    "shopt -u nocasematch; [[ ABC == abc ]] || echo cs",
    "shopt -s nullglob; echo /nonexistent_zz*; echo done",
    "shopt -s dotglob 2>/dev/null; echo ok",
    "shopt -q nullglob; echo $?",
    "shopt -s nullglob; shopt -q nullglob; echo $?",
    "shopt nullglob | grep -c off",
    "echo $PIPESTATUS; true | false | true; echo ${PIPESTATUS[@]}",
    "false | true; echo ${PIPESTATUS[0]} ${PIPESTATUS[1]}",
    "(exit 3); echo ${PIPESTATUS[@]}",
    "echo ${#PIPESTATUS[@]}",
    "echo \"$PS1\" | grep -c .",
    "echo $((BASH_SUBSHELL))",
    "( echo $BASH_SUBSHELL )",
    "( ( echo $BASH_SUBSHELL ) )",
    "echo $(echo $BASH_SUBSHELL)",
    "printf '%q ' 'a b' \"it's\" '$x' ''; echo",
    "printf '%(%s)T\\n' -1 | grep -c '^[0-9]*$'",
    "printf -v x '%05.1f' 3.14159; echo $x",
    "printf -v 'a[1]' '%s' v; echo ${a[1]}",
    "let \"x = 5 % 3\"; echo $x",
    "(( x = 5 > 3 ? 10 : 20 )); echo $x",
    "(( a[1] = 5 )); echo ${a[1]}",
    "x=010; echo $((x)) $((10#$x))",
    "echo $((  2 ** 3 ** 2 ))",
    "echo $(( -2 ** 2 ))",
    "echo $(( 1 / 0 )) 2>&1 | grep -c .",
    "echo $((9223372036854775807 + 1))",
    "echo $((0x7fffffffffffffff))",
    "echo $(( 5 > 3 && 2 > 1 ))",
    "echo $(( 1 ? 2 : 3 ? 4 : 5 ))",
    "echo $(( (1,2) + 3 ))",
    "echo $(( x++ + ++x )) 2>/dev/null",
    "x=3; echo $(( x++ )) $x; echo $(( ++x )) $x",
    "x=3; echo $(( x-- )) $x",
    "echo $(( 07 + 1 ))",
    "echo $(( 8#10 + 16#a ))",
    "echo $(( 'a' )) 2>/dev/null | grep -c .",
    "echo ${x:=$(( 1 + 1 ))}",
    "echo {a..c}{1..2}",
    "echo {a..e..2}",
    "echo {z..v}",
    "echo {10..8}",
    "echo {,a}",
    "echo {a,}x",
    "echo foo{,bar} ",
    "echo $'a\\\\b'",
    "echo -e 'a\\tb'",
    "echo -e 'a\\x41'",
    "echo -e 'a\\0101'",
    "echo -E 'a\\tb'",
    "echo -ne 'a\\n'",
    "echo -n",
    "echo -- -n",
    "echo -e '\\c' x",
    "cmd_not_found_zz 2>&1 | grep -c 'not found'",
    "builtin echo via-builtin",
    "exec 2>&1; echo ok",
    "set -B; echo {a,b}; set +B; echo {a,b}",
    "set -f; echo *; set +f",
    "set -C; echo x > /dev/null; echo $?; set +C",
    "set -C; echo x >| /dev/null; echo $?; set +C",
    "set -n; echo not-run; set +n 2>/dev/null; echo ok",
    "set -a; v1=1; set +a; sh -c 'echo $v1'",
    "set -h; echo ok",
    "set -P; echo ok; set +P",
    "set -T; echo ok; set +T",
    "set -E; echo ok; set +E",
    "set -H; echo ok; set +H",
    "set -k; echo ok; set +k",
    "set -t 2>/dev/null; echo ok",
    "set -- ; echo $#",
    "set -- -x; echo $1",
    "set -- --; echo $#",
    "a=(); echo \"${#a[@]}\"",
    "a=(x y); echo \"${a[@]:-d}\"; b=(); echo \"${b[@]:-d}\"",
    "function f() { echo func-kw-paren; }; f",
    "f() { local x=1; echo $x; }; f",
    "x=outer; f() { local x=inner; echo $x; }; f; echo $x",
    "f() { local x; echo \"[$x]\"; }; x=outer; f",
    "f() { local a=(1 2 3); echo \"${a[@]}\"; }; f",
    "f() { local -i n=5; n=n+1; echo $n; }; f",
    "f() { local x=1; g() { echo $x; }; g; }; f",
    "x=1; f() { local x=2; g; }; g() { echo $x; }; f",
    "declare -i n=3; n+=2; echo $n",
    "declare x=1; echo $x",
    "declare -x dx=1; sh -c 'echo $dx'",
    "declare -r dr=1; echo $dr",
    "declare -A m=([a]=1 [b]=2); echo \"${#m[@]}\"",
    "declare -A m=([a]=1); m[b]=2; echo \"${m[a]}${m[b]}\"",
    "declare -A m=([a]=1); unset 'm[a]'; echo \"${#m[@]}\"",
    "declare -A m=([a]=1); echo \"${m[zz]:-none}\"",
    // nullglob/failglob through `shopt`; `$PIPESTATUS` after an earlier command; `read -p` is a prompt, not the coprocess.
    "shopt -s nullglob; echo /nonexistent_zz*; echo done",
    "shopt -s nullglob; set -- /nonexistent_zz*; echo $#",
    "shopt -s failglob; echo /nonexistent_zz* 2>/dev/null; echo $?",
    "shopt -s dotglob; shopt -u dotglob; echo ok",
    ":; true | false | true; echo ${PIPESTATUS[@]}",
    "echo a >/dev/null; false | true; echo ${PIPESTATUS[0]} ${PIPESTATUS[1]}",
    "true; (exit 3) | true; echo ${PIPESTATUS[@]}",
    "read -p 'prompt: ' x <<< \"v\"; echo $x",
    "read -r -p 'p' a b <<< \"1 2\"; echo $b $a",
    // ── tenth-pass sweep ──
    // `${!name<operator>…}` applies the operator to the value of the variable NAMED
    // by $name; the bare `${!name}` form was the only one handled.
    "x=foo; y=x; printf '%s\\n' \"${!y:-none}\" \"${!y-none}\" \"${!y:+alt}\" \"${!y#f}\" \"${!y%o}\" \"${!y/o/0}\" \"${!y:1}\" \"${!y^^}\"",
    "y=x; unset x; printf '%s\\n' \"${!y:-none}\"; z=; y=z; printf '%s\\n' \"${!y:-none}\" \"${!y-none}\"",
    // A drop-in for another shell leaves ZSH_VERSION unset, so `${ZSH_VERSION:+…}`
    // zsh-detection takes the non-zsh branch.
    "printf '[%s]\\n' \"${ZSH_VERSION:+zsh}\" \"${BASH_VERSION:+bash}\"",
    // `set -f` / `set -o noglob` keeps a GLOB_SUBST result literal.
    "x='*'; set -f; printf '[%s]' $x; printf '\\n'; set +f; set -o noglob; y='*'; printf '[%s]' $y; printf '\\n'",
    // bash reads the `-c` text a line at a time: a runtime error (readonly assignment,
    // arithmetic error, failglob, a declaration builtin on a read-only name) discards the
    // rest of that LINE only, the next line still runs, and a syntax error stops the
    // text after running the lines before it.
    "readonly x=1; x=2; echo same\necho next",
    "echo $((1/0)); echo same\necho next",
    "shopt -s failglob; echo ./nonexistent_zz*; echo same\necho next",
    "declare -r s=1; declare -l s=ABC; echo after; echo $?\necho next",
    "typeset -r q=1; typeset q=2; echo after $?\necho next",
    "echo one\nf() echo hi\necho three",
    // The count may be glued onto the flag: `read -n2`, `read -N3`.
    "read -n2 x <<< 'abcdef'; printf '[%s]\\n' \"$x\"; read -N3 y <<< 'abcdef'; printf '[%s]\\n' \"$y\"",
    // Exceeding FUNCNEST ends a non-interactive bash.
    // `unset -f` of a name that is not a function is not an error.
    "unset -f never_defined_zz; printf '%d\\n' $?",
    // `exit abc` is reported with status 2 and does NOT exit; `${!x}` through an unset
    // name is fatal; `--3` is 3; `local` with no operands lists the function's locals;
    // a backslash at the very end of the input is literal.
    "exit abc 2>/dev/null; echo rc=$?",
    "echo \"[${!x}]\" 2>/dev/null; echo after",
    "echo $(( --3 )) $(( ++3 )) $(( - -3 ))",
    "f() { local a=1 b; local; }; f; g() { local; echo none; }; g",
    "echo \\",
    // printf: `\\c` in the format is literal, `\\E` is ESC, `%a` is the hex float,
    // `-n` is an invalid option (2).
    "printf 'a\\cb\\n'; printf '%s\\n' \"$(printf 'x\\Ey' | tr '\\033' _)\"; printf '%a %a %a %.2a %A\\n' 1 1.5 0 1.5 1.5; printf -n a 2>/dev/null; echo rc=$?",
    // read: `-e` off a pipe is a plain read; `-A` is invalid (2); a bad name is 1.
    "printf 'a\\n' | { read -e x; echo \"[$x]\"; }; printf 'a\\n' | { read -A a 2>/dev/null; echo rc=$?; }; printf 'a\\n' | { read 1x 2>/dev/null; echo rc=$?; }",
    // Brace ranges skip the backslash and pad `{0..02}`; test's three-operand rule
    // and strict integers.
    "echo {Z..a} {0..02} {00..2}; [ -a -a -a ]; echo $?; [ -n -a ]; echo $?; [ '' -eq 0 ] 2>/dev/null; echo $?; [ 9999999999999999999 -gt 1 ] 2>/dev/null; echo $?",
    // Bare `type` / `unset` / `builtin` are no-ops; `command -v` succeeds when any
    // name is found; `unset NAME` drops a function of that name.
    "type; echo rc=$?; unset; echo rc=$?; builtin; echo rc=$?; builtin 2>/dev/null; echo rc=$?; command -v echo nosuch; echo rc=$?; f() { :; }; unset f; type f 2>/dev/null; echo rc=$?",
    // `kill -l 0` is EXIT, past the table is 1; ZERR is no signal; unknown `set`
    // letters and option names are usage errors (2), as is `set -A`.
    "kill -l 0; kill -l 64 2>/dev/null; echo rc=$?; trap 'echo z' ZERR 2>/dev/null; echo rc=$?; set -q 2>/dev/null; echo rc=$?; set -o bgnice 2>/dev/null; echo rc=$?; set -A a x 2>/dev/null; echo rc=$?",
    // `alias -p`, `alias =z`, `unalias -a NAME`, bare `unalias`; no `nameref`, no
    // `typeset -L`; an unclosed `[` stays literal; a failed write is status 1.
    "alias x=1; alias -p; alias =z 2>/dev/null; echo rc=$?; unalias -a x; echo rc=$?; unalias 2>/dev/null; echo rc=$?; nameref r=x 2>/dev/null; echo rc=$?; typeset -L3 s=abc 2>/dev/null; echo rc=$?; cd /; echo b[in 2>/dev/null; echo a 1>&-; echo rc=$?",
    // `shift 1 2` ends a non-interactive shell (1); a subshell lists the
    // parent's traps, EXIT included, and an ignored signal stays listed.
    "trap 'echo e' EXIT; (trap); trap 'echo u' USR1; (trap); trap | grep -c SIG; trap '' INT; trap | grep -c INT; set -- a b; shift 1 2 2>/dev/null; echo after",
    // `! !` double-negates; blanks around a `test` integer are fine; `hash -d`
    // wants a name (1); a `-X` word after `kill -l` is a signal spec and lists
    // everything (`-L` is the same table); `kill` is 0 when any pid got the
    // signal; `command` runs builtins; the declaration commands are builtins;
    // a multi-digit fd before `>`; a tilde prefix ends at `:`.
    "! ! true; echo $?; test '1 ' -eq 1; echo $?; hash -d 2>/dev/null; echo $?; kill -l -1 | head -c 9; echo; kill -L | head -c 9; echo; kill -l ' 1'; kill -0 $$ 999999 2>/dev/null; echo $?; command : ; echo $?; command export x=1; echo $x; type local | head -c 30; echo; echo a 10>/dev/null; cd /; echo ~:x | head -c 3; echo",
    // A pipeline stage lists the parent's traps until its own `trap` changes
    // them; a signal name may be spelled `sigusr1`; `command trap` is trap.
    "trap 'echo x' INT; (trap - INT; trap) | wc -l | tr -d ' '; trap '' INT; (trap 'echo x' INT; trap) | head -1; trap 'echo t' sigusr1; trap | grep -c USR1; trap 'echo e' EXIT; trap | grep -c EXIT; command trap - EXIT; echo end",
    // `pwd` and `times` ignore operands; a `break` inside a function cannot
    // end the caller's loop (bash warns, 0); `umask -p`; `]]` and `in` are
    // keywords; `readonly -p` lists the named variables in the declare form;
    // a WINCH trap is delivered; the EXIT trap runs when a non-interactive
    // `exec` fails.
    "pwd a >/dev/null; echo $?; times a >/dev/null; echo $?; f() { break; echo in; }; for i in 1; do f 2>/dev/null; echo $i; done; umask -p | head -c 6; echo; command -v ']]'; echo $?; type in; readonly r=1; readonly -p | grep -c ' r='; trap 'echo t' WINCH; kill -WINCH $$; echo yes; trap 'echo e' EXIT; exec nonexist 2>/dev/null; echo no",
    // A foreground child or `( … )` fires a CHLD trap; a trap raised by `kill`
    // on the shell sees `$?` = 0; a pipeline stage runs the EXIT trap it set;
    // a subshell lists the parent's traps until its own `trap` changes them;
    // an untrapped TERM runs the EXIT trap, and so does `exit` inside a trap.
    "trap 'echo t' CHLD; (true); echo yes; /usr/bin/true; echo end; trap - CHLD; trap 'echo $?' USR1; false; kill -USR1 $$; trap 'echo s' EXIT | cat; trap 'echo x' USR2; (trap) | grep -c USR2; trap '' INT; (trap 'echo y' INT; trap) | grep -c INT; trap 'echo e' EXIT; trap 'echo T; exit 3' TERM; kill -TERM $$; echo no",
    // An untrapped HUP runs the EXIT trap before the shell dies (129).
    "trap 'echo e' EXIT; kill -HUP $$; echo no",
    // SIGQUIT is swallowed; setting an EXIT trap arms the terminating signals
    // (set_signal → initialize_terminating_signals), so INT, ALRM and PIPE then run
    // it, while a shell without one dies of INT at once.
    "kill -QUIT $$; echo q; trap 'echo e' EXIT; kill -INT $$; echo no",
    "kill -INT $$; echo no",
    "trap 'echo e' EXIT; kill -ALRM $$; echo no",
    // A forked child gets the original dispositions back (make_child), so a killed
    // `sleep &` dies of the signal even with an EXIT trap set; the ignore an `&` job
    // sets is not listed; a `( … )` killed from inside reports 143 and the parent's
    // EXIT trap still runs.
    "trap 'echo e' EXIT; sleep 5 & kill $!; wait $!; echo rc=$?; kill -PIPE $$; echo no",
    "trap 'echo x' USR2; (trap) & wait; trap '' INT; (trap) & wait",
    "trap 'echo e' EXIT; ( sh -c 'kill -TERM $PPID'; echo no ); echo $?",
];

/// bash + ksh corpus — constructs the two share but zsh does not, so they run
/// against the real bash and the real ksh (`--bash` / `--ksh`), never zsh.
const BASH_KSH_CORPUS: &[&str] = &[
    "typeset -A A; A[\"q r\"]=1; A[\"q\\\"r\"]=5; k=\"a b\"; A[$k]=2; A[x\"y\"]=3; for k in \"${!A[@]}\"; do echo \"<$k>=${A[$k]}\"; done | sort",
    "getopts a 2>/dev/null; echo $?",
    "(( 1/0 )) 2>/dev/null; echo $?",
    "ulimit -Z 2>/dev/null; echo $?",
    "printf '%ld %lld %hhd %zd|\\n' 1 2 3 4",
    "select x in a b; do break; done 2>/dev/null </dev/null; echo $?",
    "kill -9 2>/dev/null; echo $?",
    "echo $((0b11)) 2>/dev/null; echo $?",
    "alias 'z z=x' 2>/dev/null; echo $?",
    // A lone `-` is an ordinary `echo` argument (zsh's option parser eats it).
    "echo -",
    "echo - a",
    "echo -n -; echo",
    // `$"..."` is the plain string when no translation applies (zsh keeps the `$`).
    "echo $\"hello\"",
    "x=1; echo $\"a $x\" b",
    // `test -a FILE` / `test -o OPTION` as two-argument unary operators.
    "[ -a / ]; echo $?",
    "[ -a /nonexistent_zz ]; echo $?",
    "[ -o errexit ]; echo $?",
    "set -o errexit; [ -o errexit ]; echo $?",
    "test -o nosuchoption_zz; echo $?",
    // `unset` of a readonly variable reports and continues with status 1.
    "readonly R=1; unset R; echo $?",
    "readonly R=1; f() { unset R; echo in; }; f",
    // `test`/`[` string ordering (`<` sorts before, `>` after) and the sh-style PS4/IFS defaults a real bash/ksh starts with — both read zsh's own values before the personality gate (the `-f` that globally disabled globbing hid the glob half).
    "[ abc \\< abd ] && echo lt; [ abd \\> abc ] && echo gt; [ abd \\< abc ] || echo not-lt",
    "test a \\< b && echo T; test b \\< a || echo F",
    "[ 10 \\< 9 ] && echo lexical",
    "printf '[%s]\\n' \"$PS4\"; printf '%s\\n' \"${#IFS}\"",
    // ── tenth-pass sweep ──
    // A scalar subscripted `[@]` / `[*]` is a one-element array (zsh: string length).
    "x=abc; printf '%s %s %s\\n' \"${#x}\" \"${#x[@]}\" \"${#x[*]}\"; e=; printf '%s %s\\n' \"${#e[@]}\" \"${#e}\"; unset u; printf '%s\\n' \"${#u[@]}\"",
    "printf '[%s]\\n' \"${ZSH_VERSION:+zsh}\"",
    "x='*'; set -f; printf '[%s]' $x; printf '\\n'",
];

/// Scripts only ksh93 (and its zshrs drop-in) accept: `print`, `typeset -i/-l/-u`,
/// `set -A`, `float`/`integer`, ksh-glob patterns, `function f {}` scoping.
const KSH_ONLY_CORPUS: &[&str] = &[
    "M=9223372036854775807; echo $((M+1)) $((M+1000)) $((M*2)) $((-M-2)) $(( (-M-1) / -1 )) $((M-1)); echo $((2**60+1)) $((9007199254740993)) $((9007199254740993 % 10)); x=9007199254740993; echo $((x)) $((x==9007199254740992))",
    "echo \"a\\x\"; printf 'p\\x|\\n' | wc -c | tr -d ' '",
    "echo $((7.5%2)) $((-7.5%2)) $((9.9%4))",
    "test 1.5 -gt 1; echo $?; test 1.5 -lt 1; echo $?; test 2 -eq 2.0; echo $?; [ 0.1 -ne 0.1 ]; echo $?",
    "test 0x10 -eq 16; echo $?; test a -eq a 2>/dev/null; echo $?; test 1+1 -eq 2; echo $?; test 7 -eq 8; echo $?",
    "eval \"echo '\" 2>/dev/null; echo $?",
    "eval 'echo (' 2>/dev/null; echo $?",
    "eval false; echo $?",
    "sleep 0.05 & p=$!; wait $p; wait $p 2>/dev/null; echo $?",
    "type -t echo; type -t ls; type -t if; f() { :; }; type -t f; alias a=b; type -t a; type -t nosuch_zz; echo rc=$?",
    "cd //tmp; pwd",
    "cd '' 2>/dev/null; echo $?",
    "ZZ_V=\"it's\"; set | grep '^ZZ_V='",
    "echo \"[${CDPATH-u}][${MAILCHECK-u}][${TMOUT-u}]\"",
    "alias a1=b; command -v a1",
    "type if",
    "f() { local a=1; echo \"[$a]\"; echo rc=$?; }; f",
    "set -- a b; shift -1; echo no",
    "printf '%3$s %1$s\\n' a b c 2>/dev/null; echo $?",
    "builtin echo hi 2>/dev/null; echo $?",
    "echo \"[${OSTYPE+o}][${MACHTYPE+m}]\"",
    "echo \"[$PS2][${PS1+s}][${UID+s}][${EUID+s}]\"",
    "print hello",
    "print -r 'a\\tb'",
    "print -n a; print b",
    "print -- -x",
    "print -u2 err 2>/dev/null; print ok",
    "typeset -i n=5; n=n*2; print $n",
    "typeset -i2 b=5; print $b",
    "typeset -i16 h=255; print $h",
    "typeset -u U=abc; print $U",
    "typeset -l L=ABC; print $L",
    "typeset -r r=1; print $r",
    "typeset -A m; m[a]=1; m[b]=2; print ${m[a]}${m[b]}",
    "typeset -A m=([a]=1 [b]=2); print ${#m[@]}",
    "typeset -a arr; arr[0]=x; arr[2]=z; print ${#arr[@]} ${arr[2]}",
    "set -A arr a b c; print ${arr[1]} ${#arr[@]}",
    "set -A arr a b c; set +A arr x; print ${arr[@]}",
    "arr=(a b c); print ${arr[@]:1}",
    "arr=(a b c); print ${!arr[@]}",
    "arr=(a b c); print ${arr[@]#a}",
    "arr=(a b c); print \"${arr[*]}\"",
    "x=hello; print ${x:1:3}",
    "x=hello; print ${x#h} ${x##*l} ${x%o} ${x%%l*}",
    "x=hello; print ${#x}",
    "x=hello; print ${x/l/L} ${x//l/L}",
    "x=hello; print ${x^^} ${x^} 2>/dev/null",
    "x=HELLO; print ${x,,} 2>/dev/null",
    "integer n=3; print $((n*n))",
    "float f=1.5; print $((f*2))",
    "print $((3.0/2))",
    "print $((10/4)) $((10%4)) $((2**3))",
    "print $((1.5+1.5))",
    "print $(( 3 > 2 ? 10 : 20 ))",
    "let \"x = 4 + 5\"; print $x",
    "let x=3 y=4; print $((x*y))",
    "(( x = 2 )); (( x++ )); print $x",
    "(( 5 > 3 )) && print gt",
    "[[ abc == a* ]] && print m",
    "[[ -n x && -z \"\" ]] && print and",
    "[[ a < b ]] && print lt",
    "[[ -f /etc/passwd && -d / ]] && print files",
    "[[ abc == @(abc|def) ]] && print alt",
    "[[ abc == +(a|b|c) ]] && print plus",
    "[[ abc == !(x*) ]] && print not",
    "case abc in @(abc|d)) print ok;; esac",
    "case abc in a*(b)c) print star;; esac",
    "function f { print kw $1; }; f x",
    "function f { typeset x=1; print $x; }; x=out; f; print $x",
    "f() { typeset -i n=$1; print $((n+1)); }; f 4",
    "f() { print $#; }; f a b",
    "f() { return 3; }; f; print $?",
    "f() { print \"${1:-none}\"; }; f; f z",
    "f() { print $1; shift; print $1; }; f a b",
    "typeset -n ref=target; target=val; print $ref",
    "typeset -n ref=target; ref=new; print $target",
    "nameref r=x 2>/dev/null; print ok",
    "for i in a b c; do print -n $i; done; print",
    "for ((i=0;i<3;i++)); do print -n $i; done; print",
    "for i in {1..3}; do print -n $i; done; print",
    "i=0; while ((i<3)); do print -n $i; ((i++)); done; print",
    "i=0; until ((i>=3)); do print -n $i; ((i++)); done; print",
    "print \"a\\tb\"",
    "print 'single'",
    "print \"$(print sub)\"",
    "print `print back`",
    "print ${undef:-def} ${undef-def2}",
    "print ${undef:=asg}; print $undef",
    "print $(( $(print 3) + 1 ))",
    "print {a,b}{1,2}",
    "print {1..3}",
    "print $'tab\\there'",
    "print $'\\x41\\101'",
    "print -r $'a\\nb' | wc -l",
    "trap 'print bye' EXIT; print main",
    "trap 'print err' ERR; false; print after 2>/dev/null",
    "trap - EXIT; print ok",
    "(trap 'print sub' EXIT; print in); print out",
    "whence print; whence -v print | grep -c .",
    "whence -p ls | grep -c /",
    "print $ERRNO 2>/dev/null; print ok",
    "print $OPTIND",
    "print ${PS4}",
    "print ${#IFS}",
    "print $? ",
    "print $#",
    "true; print $?; false; print $?",
    "command print x",
    "exec 3>&1; print -u3 fd3; exec 3>&-",
    "print -u1 one",
    "print $OPTARG",
    "eval 'print ev'",
    "eval \"x=5\"; print $x",
    ". /dev/null; print $?",
    ": ; print $?",
    "exit 3",
    "(exit 4); print $?",
    "print done; exit",
    // ── ksh93u+m namespace, `$-`, printf bases and table-only `set -o` names ──
    "v=${.sh.version}; print -r -- \"[${v:0:20}]\"; f() { print -r -- \"fun=${.sh.fun}\"; }; f",
    "[[ abc =~ a(b)c ]]; print -r -- \"[${.sh.match[0]}][${.sh.match[1]}] n=${#.sh.match[@]}\"",
    "print -r -- \"$-\"; set -u; print -r -- \"$-\"; set +u; set -C; print -r -- \"$-\"",
    "printf '%..8d|%..16d|%5..8d|%.3..8d\\n' 300 255 5 5",
    "set -o keyword; [[ -o keyword ]]; print -r -- $?; set +o keyword; [[ -o keyword ]]; print -r -- $?",
    "set -o gmacs; print -r -- $?; set +o gmacs; print -r -- $?",
    "set -A a x y z; unset 'a[2]'; set -A a p q r s; print -r -- \"${#a[@]} ${a[*]}\"",
    // Constants and `**` go through double too: saturate when exact, else float.
    "echo $(( -9223372036854775808 )) $(( 9223372036854775808 )) $(( 99999999999999999999 )); echo $(( 2 ** 63 )) $(( 2 ** 64 )) $(( 3 ** 40 )) $(( -2 ** 63 ))",
    // A `$( )` capture stops at the first NUL; a negative substring length is empty.
    "x=\"$(printf 'a\\0b\\0c')\"; echo \"${#x}\" \"$x\"; y=abc; echo \"[${y:0:-1}][${y:1:-1}]\"",
    // printf: a `%` ending the format is literal, `%a` shows twelve mantissa digits,
    // `\\cX` in the format is the control character, `\\E` is ESC; no `{Z..a}`.
    "printf '%'; echo \" rc=$?\"; printf 'a%'; echo; printf '%a\\n' 1.5; printf 'a\\cb|' | od -An -c | tr -s ' '; printf '%s\\n' \"$(printf 'x\\Ey' | tr '\\033' _)\"; echo {Z..a} {x..z}",
    // A read-only assignment inside a function ends the script; `read -e` / `-x` are 2.
    "f() { readonly l=1; l=2; echo no; }; f; echo rc=$?; printf 'a\\n' | { read -e x 2>/dev/null; echo rc=$?; }; printf 'a\\n' | { read -x x 2>/dev/null; echo rc=$?; }",
    // Bare `type` is a usage error (2); `kill -l 0` / `128` are EXIT; ZERR is no
    // signal; `unalias -a NAME` is fine, bare `unalias` is 2; a failed write is 1;
    // an unclosed `[` stays literal. A bare `unset` ends the shell (2).
    "type 2>/dev/null; echo rc=$?; kill -l 0; kill -l 128; trap 'echo z' ZERR 2>/dev/null; echo rc=$?; unalias -a x; echo rc=$?; unalias 2>/dev/null; echo rc=$?; echo a 1>&-; echo rc=$?; cd /; echo b[in",
    "unset 2>/dev/null; echo no",
    // A backslash-escaped `.` in a `=~` regex is literal; a quoted span is not.
    "[[ abc =~ a\\.c ]]; echo rc=$?; [[ a.c =~ a\\.c ]]; echo rc=$?; [[ abc =~ \"a.c\" ]]; echo rc=$?",
    // `! !` double-negates; blanks around a `test` integer; `hash -d` is an
    // unknown option (2); `kill -l -1` lists everything, `kill -l -x` is a
    // usage error (2); `alias -p` is the reusable form; a tilde prefix ends at
    // `:`; `command` runs builtins; `sigusr1` is a signal name; `10>` is the
    // word `10`.
    "! ! true; echo $?; test '1 ' -eq 1; echo $?; hash -d 2>/dev/null; echo $?; kill -l -1 | head -1; kill -l -x 2>/dev/null; echo $?; alias x=y; alias -p; cd /; echo ~:x | head -c 3; echo; command : ; echo $?; trap 'echo t' sigusr1; echo $?; echo a 10>/dev/null",
    // `$[` is literal text; `pwd` ignores operands; a `break` inside a
    // function cannot end the caller's loop; no `local` or `declare`;
    // `command -V` keeps the alias wording; the POSIX specials and typeset
    // are "special shell builtin"; `in` is a keyword; a $PATH command is a
    // "tracked alias"; a signalled child is 256+sig (foreground or waited);
    // a WINCH trap is delivered; a failed non-interactive `exec` exits
    // WITHOUT running the EXIT trap.
    "echo $[1+2]; pwd a >/dev/null; echo $?; f() { break; echo in; }; for i in 1; do f; echo $i; done; command -v local; echo $?; command -v declare; echo $?; alias e=echo; command -V e; type . ; type typeset; type in; type ls | head -c 20; echo; sleep 5 & kill $!; wait $! 2>/dev/null; echo rc=$?; sh -c 'kill -TERM $$'; echo $?; trap 'echo t' WINCH; kill -WINCH $$; echo yes; trap 'echo e' EXIT; exec nonexist 2>/dev/null; echo no",
    // A trap raised by `kill` on the shell sees `$?` = 0; a pipeline stage
    // runs the EXIT trap it set; `( trap )` and `$(trap)` list the parent's
    // traps; a forked subshell (pipeline stage, nested `$( … )`) lists no
    // inherited trap; `>& word` is an error (1) that leaves the command unrun;
    // `<>file` opens fd 1; PATH unset still searches /bin; an untrapped TERM
    // runs the EXIT trap, and so does `exit` inside a trap.
    "trap 'echo $?' USR1; false; kill -USR1 $$; trap 'echo s' EXIT | cat; trap 'echo x' USR2; (trap) | grep -c USR2; echo [$(trap)] | grep -c USR2; echo a >& /dev/null; echo rc=$?; echo b <>/tmp/zshrs_rw_$$; cat /tmp/zshrs_rw_$$; rm -f /tmp/zshrs_rw_$$; (unset PATH; ls / >/dev/null; echo rc=$?); trap 'echo e' EXIT; trap 'echo T; exit 3' TERM; kill -TERM $$; echo no",
    // An untrapped HUP runs the EXIT trap before the shell dies (129).
    "trap 'echo e' EXIT; kill -HUP $$; echo no",
    // `trap` lists the parent's traps only in a subshell ksh93 runs without forking:
    // not a pipeline stage or `&` job, not a subshell nested in one, not a `( … )`
    // that has run an external command (a `$( … )` that has still does); a forked
    // child dies of a signal even with an EXIT trap set; ALRM and QUIT run the EXIT
    // trap; `trap -p SIG` prints the body alone.
    "trap 'echo x' USR2; trap | grep -c USR2; (trap) | grep -c USR2; (sleep 0; trap); echo \"[$(trap)]\"; (echo \"[$(trap)]\"); (x=$(/bin/true; trap); echo \"[$x]\"); (trap) & wait; trap 'echo e' EXIT; sleep 5 & kill $!; wait $!; echo rc=$?; kill -ALRM $$; echo no",
    "trap 'echo e' EXIT; kill -QUIT $$; echo no",
    "trap 'echo x' INT; trap -p INT 2>/dev/null; trap | grep -c INT; trap 'echo e' 0; trap | grep -c EXIT",
    "trap 'echo x' USR2; (cd /; trap); (exec 3>/dev/null; trap); f() { trap; }; (f); ( (trap) ); echo \"[$( (trap) )]\"; trap '' INT; (trap) | cat; (trap '' USR1; trap)",
];

/// Scripts every REAL Bourne-family shell (bash, ksh, sh, dash, mksh, ash) runs
/// the same way and zsh does not: zsh predeclares `OPTARG`, and refuses
/// `break` / `continue` outside a loop.
const REAL_SHELL_CORPUS: &[&str] = &[
    "x=abc; echo ${#x}; echo ${#x#a}; echo after",
    "set -- -z; getopts ab o 2>/dev/null; printf '[%s:%s]\\n' \"$o\" \"${OPTARG-u}\"",
    "set -- -a; getopts a: o 2>/dev/null; printf '[%s:%s]\\n' \"$o\" \"${OPTARG-u}\"",
    "echo a,b}; echo }; { echo blk; }; f() { echo fn; }; f; echo x}y",
    "which echo | head -1",
    "echo [$!]",
    "echo \"[${OPTARG+set}]\"",
    "echo \"[${COLUMNS+c}${LINES+l}]\"",
    "echo \"[${OPTARG-unset}]\"",
    "break; echo after=$?",
    "continue; echo after=$?",
    "f() { break; echo in=$?; }; f; echo out=$?",
    "for i in 1 2; do echo $i; break; done; echo done",
    // `exit` with two operands reports them and exits 1; zsh alone goes on.
    "exit 1 2; echo after",
    // zsh's `_` digit separator is a syntax error everywhere else.
    "echo $(( 1_000 )) 2>/dev/null; echo after",
    // An assignment is accepted as the middle operand of `?:`.
    "x=0; echo $(( x ? y = 1 : 2 )); x=1; echo $(( x ? y = 5 : 2 )) $y",
    // read: NUL bytes dropped (sh: line ends there), ONE trailing IFS delimiter dropped.
    "printf 'a\\0b\\n' | { read x; echo \"[$x]\"; }; printf 'a,b,\\n' | { IFS=, read x y; echo \"[$x][$y]\"; }; printf 'abcb\\n' | { IFS=b read x y; echo \"[$x][$y]\"; }; printf 'a,b,,\\n' | { IFS=, read x y; echo \"[$x][$y]\"; }",
    // printf `%d` saturates, `%05s` is the platform's; a lone `[` is a literal pattern.
    "printf '%d\\n' 9223372036854775808 -9223372036854775809 2>/dev/null; printf '%05s|\\n' ab; case x in [) echo 1;; esac; echo rc=$?",
];

/// Every shell but native zsh (including the zsh-STYLE legs): a bare `$NN` is
/// `${N}` followed by the digit (c:Src/params.c:2210, `bracks = -1`).
const BARE_POSITIONAL_CORPUS: &[&str] = &[
    "set -- a b c d e f g h i j k; echo $10 ${10} \"$10\" $11",
    "set -- a b; echo $1$2 $12",
];

/// bash, ksh93 and mksh count `$@` / `$*` with no positional parameters as
/// UNSET for `-` / `+` (dash and zsh count them as set), and leave `OPTARG` unset
/// after a `getopts` option that takes no argument (dash and zsh set it empty).
/// bash 5.3 continues after a FUNCNEST overflow; 5.2 (Ubuntu) ends the script, so
/// this only runs against bash >= 5.3.
const BASH_53_CORPUS: &[&str] = &[
    "FUNCNEST=3; d=0; f() { d=$((d+1)); f; }; f 2>/dev/null\nprintf 'not reached\\n'",
];

const BASH_KORN_CORPUS: &[&str] = &[
    "typeset -a e; e=(); echo \"[${e[@]-none}] [${e[*]-none}] [${e[@]+set}] [${e[@]:-none}]\"; g=(a); echo \"[${g[@]-none}] [${g[@]+set}]\"",
    "set -- -a x; getopts a o; echo \"$o [${OPTARG+set}]\"",
    "set --; printf '[%s]' \"${@-x}\"; printf '\\n'",
    "set --; printf '[%s]' \"${*-y}\"; printf '\\n'",
    "set --; printf '[%s]' \"${@+x}\"; printf '\\n'",
    "set --; printf '[%s]' \"${*+y}\"; printf '\\n'",
    "set --; printf '[%s]' \"${@:-x}\" \"${*:-y}\"; printf '\\n'",
    "set -- a; printf '[%s]' \"${@-x}\" \"${*+y}\"; printf '\\n'",
    // `-nt` is true when only the second file is missing, `-ot` when only the first is.
    "[ /etc/passwd -nt /nonexistent_zz ]; echo $?; [ /nonexistent_zz -ot /etc/passwd ]; echo $?; [ /nonexistent_zz -nt /etc/passwd ]; echo $?; [ /etc/passwd -ot /nonexistent_zz ]; echo $?",
    // NUL bytes are dropped from a `$( )` capture (ksh93 truncates; see KSH_ONLY).
    "x=\"$(printf 'a\\0b\\0c')\"; echo \"${#x}\" \"$x\"",
    // `cmd &&` at the end of the input is a syntax error (zsh runs `cmd`); blank
    // lines and trailing `;` are not.
    "echo a &&",
    "echo a\n\n\necho b; echo c;\n\necho d\n",
    // The Korn shells and bash accept `'a'` / `--3` differently; both reject `1.5`.
    "echo $(( 1.5 + 1 )) 2>/dev/null; echo after",
    // Arithmetic names in substring offsets; dangling `-a`/`-o`; `declare` exists
    // in bash only.
    "x=abcdef; i=2; echo ${x:i} ${x:$i:1} ${x:i+1:i}; [ 1 -eq 1 -a ] 2>/dev/null; echo $?; [ -n abc -o ] 2>/dev/null; echo $?; [ -a -a -a ] 2>/dev/null; echo $?; f() { declare x=3; echo $x; }; x=1; f 2>/dev/null; echo $x",
    // A numeric subscript on a scalar: `[0]` is the whole value, any other
    // index is empty (zsh indexes the characters).
    "x=abc; i=0; echo \"${x[0]}\" \"${x[1]:-e}\" \"${#x[0]}\" \"${x[@]}\" \"${x[i]}\"",
];

/// mksh / pdksh: `printf %d` parses a leading-zero operand as octal
/// (strtol base 0), though its arithmetic does not.
const PDKSH_CORPUS: &[&str] = &[
    "echo \"a\\x\" \"\\x41\"; printf \"%s\\n\" $'d\\x'",
    "cat /dev/null; hash; echo a; hash -r; echo $?; alias -t; hash ls; echo $?",
    "f() { echo ${1:?missing}; echo inf; }; f; echo \"after rc=$?\"; g() { echo ${v?unset}; echo ing; }; g; echo \"after2 rc=$?\"",
    "f() { echo ${1:?missing}; }; f 2>/dev/null; echo $?; : ${nosuch_zz:?m}; echo notreached",
    "sleep 0.05 & p=$!; wait $p; wait $p 2>/dev/null; echo $?",
    "(( 1/0 )) 2>/dev/null; echo $?",
    "trap 'echo x' INT; (trap) | wc -l | tr -d ' '; trap | wc -l | tr -d ' '; x=$(trap); echo \"[$x]\"; trap",
    "echo \"[${CDPATH-u}][${MAILCHECK-u}][${TMOUT-u}]\"",
    "echo $((2 ** 3)) 2>/dev/null; echo after",
    "set -- a b; shift -1; echo no",
    "echo $((0b11)) 2>/dev/null; echo $?",
    "alias 'z z=x' 2>/dev/null; echo $?",
    "printf '%d\\n' 010 0x1f 7",
    "echo $((010))",
    // A read-only violation is status 2 in mksh, 1 for the other fatal shapes.
    "readonly r=1; (r=2); echo a$?",
    "readonly r=1; (export r=3); echo b$?",
    "readonly r=1; (r=2 true); echo c$?",
    "readonly r=1; (: $((r=5))); echo d$?",
    "(set -u; : $zz_unset) 2>/dev/null; echo e$?",
    "(: ${zz_unset:?msg}) 2>/dev/null; echo f$?",
    // A failed `cd` is status 2; `{1..3}` is not a range (the comma form is).
    "cd /nonexistent_zz 2>/dev/null; echo $?",
    "echo {1..3} {a,b}c",
    // `$PS1` is `$ ` in mksh (unset in ksh93); no `$UID` / `$EUID`.
    "echo \"[$PS1][$PS2][${UID+s}][${EUID+s}]\"",
    "echo \"[${OSTYPE+o}][${MACHTYPE+m}]\"",
    // Constants are 32-bit; `'a'` is a character code; `test -eq` on a word is a plain false;
    // a negative substring length runs to the end.
    "echo $(( 9223372036854775807 )) $(( 4294967296 )) $(( 0x100000000 )) $(( 99999999999999999999 )) $(( 'a' + 1 )); [ a -eq 1 ] 2>/dev/null; echo $?; x=abc; echo \"[${x:1:-1}]\"",
    // The ELSE operand of `?:` may be an assignment; a syntax error in eval ends the shell;
    // a `;` with nothing before it is a syntax error.
    "x=0; echo $(( x ? 1 : y = 3 )) $y; eval 'if'; echo after",
    "; echo x",
    // `%b` reads a bare `\\NNN` as octal; `\\x41` comes out as `x41`.
    "printf '%b\\n' 'A\\101\\x41'",
    // The system printf: `\\E` and `\\c` drop their backslash, `%a` is the hex float,
    // `-n` fails with 1, `%c` with no operand prints nothing.
    "printf '%s\\n' \"$(printf 'x\\Ey\\n')\"; printf 'a\\cb\\n'; printf '%a\\n' 1.5; printf -n a 2>/dev/null; echo rc=$?; printf '[%c]\\n'",
    // No `{a..c}` ranges; `;;&` exists; `[^b]` is not a negated class.
    "echo a{1..3} {a..c} {1..3}{a,b}; case abc in abc) echo 1;;& a*) echo 2;; esac; case abc in a[^b]c) echo 1;; *) echo 2;; esac",
    // test: an identifier operand is its value, `1.0`/`0x1` are rejected, 32-bit
    // digits, dangling `-a`, no `-N`.
    "[ 1 -eq 1.0 ] 2>/dev/null; echo $?; [ 0x1 -eq 1 ] 2>/dev/null; echo $?; [ b -eq 0 ]; echo $?; b=5; [ b -gt 4 ]; echo $?; [ 1 -eq 1 -a ] 2>/dev/null; echo $?; [ -N /etc/passwd ] 2>/dev/null; echo $?",
    // A read-only assignment ends only the function (status 2); `read -p` / `-e` are 2.
    "f() { readonly l=1; l=2; echo no; }; f 2>/dev/null; echo rc=$?; printf 'a\\n' | { read -p P x 2>/dev/null; echo rc=$?; }; printf 'a\\n' | { read -e x 2>/dev/null; echo rc=$?; }",
    // Bare `type` / `unset` are no-ops; ZERR is no signal; `unalias -a NAME` is an
    // error here; a failed write is 1; an unclosed `[` stays literal.
    "type; echo rc=$?; unset; echo rc=$?; trap 'echo z' ZERR 2>/dev/null; echo rc=$?; unalias -a x; echo rc=$?; echo a 1>&-; echo rc=$?; cd /; echo b[in",
    // `kill -l NAME` is "bad number" (1); an empty test operand is 0; `unset`
    // of a local reveals the outer value; a shift past `$#` ends only the
    // function (1); a bare `export` lists names; an ignored signal stays
    // listed in a subshell.
    "kill -l TERM 2>/dev/null; echo rc=$?; [ '' -eq 0 ]; echo $?; f() { local x=5; unset x; echo \"[$x]\"; }; x=0; f; echo $x; g() { shift; echo \"[$@]\"; }; g 1; g 2>/dev/null; echo rc=$?; zz=1; export zz; export | grep -c '^zz$'; trap '' INT; (trap); trap | grep -c INT",
    // `! !` double-negates; blanks around a `test` integer; `kill -l -1` is an
    // unknown option (1); `alias -p` is the reusable form; `${#x}` counts
    // bytes; `sigusr1` is a signal name; `command` runs builtins; `10>` is the
    // word `10`; `${@:2}` is a fatal bad substitution (1).
    "! ! true; echo $?; test '1 ' -eq 1; echo $?; kill -l -1 2>/dev/null; echo $?; alias x=y; alias -p | tail -1; x=héllo; echo ${#x}; trap 'echo t' sigusr1; echo $?; command : ; echo $?; echo a 10>/dev/null; set -- a b c; echo ${@:2} 2>/dev/null; echo after",
    // `$[` is literal text; `cd ''` is a no-op; `times` ignores operands;
    // `alias NAME` for an unknown alias says so on stdout (1); `command -v`
    // stops at the first missing name; no `declare`; the POSIX specials are
    // "special shell builtin"; `in` is a keyword; a $PATH command is a
    // "tracked alias"; printf is the external command; `kill -l ' 1'`; a
    // `break` inside a function cannot end the caller's loop; a WINCH trap is
    // delivered; the EXIT trap runs once, before the `exec`, failed or not.
    "echo $[1+2]; cd ''; echo rc=$?; times a >/dev/null; echo $?; alias x; echo $?; command -v nonexist echo; echo $?; command -v declare; echo $?; type . ; type typeset; type in; type ls | head -c 20; echo; type printf | grep -c /; kill -l ' 1'; f() { break; echo in; }; for i in 1; do f 2>/dev/null; echo $i; done; trap 'echo t' WINCH; kill -WINCH $$; echo yes; trap 'echo e' EXIT; exec nonexist 2>/dev/null; echo no",
    "trap 'echo t' EXIT; exec true; echo no",
    // A trap raised by `kill` on the shell sees `$?` = 0; a pipeline stage
    // runs the EXIT trap it set; a `( … )` fires a CHLD trap; a subshell lists
    // only the traps it set itself or inherited ignored; the predefined aliases
    // list as `\\builtin …`; `>& word` is an error (1); PATH unset still
    // searches /bin; an untrapped TERM runs the EXIT trap.
    "trap 'echo $?' USR1; false; kill -USR1 $$; trap 'echo s' EXIT | cat; trap 'echo t' CHLD; (true); echo yes; trap - CHLD; trap 'echo x' USR2; (trap) | wc -l | tr -d ' '; trap '' INT; (trap 'echo y' INT; trap); alias local; type local; echo a >& /dev/null; echo rc=$?; (unset PATH; ls / >/dev/null; echo rc=$?); trap 'echo e' EXIT; kill -TERM $$; echo no",
    // An untrapped HUP ends the shell WITHOUT the EXIT trap (129).
    "trap 'echo e' EXIT; kill -HUP $$; echo no",
    // `exec BUILTIN` runs the builtin and exits through the EXIT trap; `exec
    // EXTERNAL` replaces the shell without it.
    "trap 'echo t' EXIT; exec echo hi",
    "trap 'echo t' EXIT; exec /usr/bin/true; echo no",
    // mksh catches INT, QUIT and TERM only: each runs the EXIT trap and the shell
    // exits with the trap's own status; ALRM is swallowed; HUP, USR1 and USR2 kill
    // it without the trap; the ignore an `&` job sets is not listed.
    "trap 'echo e' EXIT; kill -ALRM $$; echo a; kill -USR1 $$; echo no",
    "trap 'echo e; false' EXIT; kill -TERM $$",
    "trap 'echo e' EXIT; kill -INT $$",
    "trap 'echo e' EXIT; kill -QUIT $$",
    "trap 'echo e' EXIT; kill -HUP $$; echo no",
    "trap 'echo x' USR2; (trap) & wait; echo done",
];

/// dash and ash: `$@` counts as set with no positionals, so the quoted `-`
/// form is ONE empty word (bash, ksh and mksh drop it).
const DASH_FAMILY_CORPUS: &[&str] = &[
    "kill nosuch 2>/dev/null; echo after",
    "kill -n 0 $$ 2>/dev/null; echo after",
    "kill -CONT $$; echo $?",
    "ulimit -Z 2>/dev/null; echo after",
    "fg 2>/dev/null; echo $?",
    "umask 999 2>/dev/null; echo $?",
    "kill -9 2>/dev/null; echo $?",
    "d=$(mktemp -d); cd $d; touch .h a; echo .*; echo *; cd /; rm -rf $d",
    "set --; f() { echo $#; }; f \"${@-x}\"; f \"${@-}\"; f \"${@:-x}\"; f \"${@+x}\"; f ${@-x}",
    "set --; f() { echo $#; }; f \"${*-x}\" \"${@}\" \"${@}\" \"${@-x}\"",
    "set -- a b; f() { echo $#; }; f \"${@-x}\"",
    // No `builtin`, no `$UID` / `$EUID`.
    "builtin echo x 2>/dev/null; echo $?",
    "printf '%3$s %1$s\\n' a b c 2>/dev/null; echo $?",
    "echo \"[${OSTYPE+o}][${MACHTYPE+m}]\"",
    "echo \"[${UID+s}][${EUID+s}]\"",
];

/// dash proper (the host's `ash` is busybox, which has `pipefail` and `\x`).
const DASH_ONLY_CORPUS: &[&str] = &[
    "echo \"a\\x\" \"\\x41\"",
    "echo $((-9223372036854775808 / -1)) $((9223372036854775808)) $((-9223372036854775808)) $((99999999999999999999))",
    "(exec nosuch_zz) 2>/dev/null; echo $?; (exec /bin/echo hi); echo $?; (exec /nonexistent/x) 2>/dev/null; echo $?",
    "sleep 0.05 & p=$!; wait $p; wait $p 2>/dev/null; echo $?",
    "printf -v v '%s-%s' a b; echo \"[$v]\" $?; printf '%s\\n' ok",
    "trap 'echo x' INT; (trap) | wc -l | tr -d ' '; trap | wc -l | tr -d ' '; x=$(trap); echo \"[$x]\"; trap",
    "ZZ_V=1; set | grep '^ZZ_V='",
    "ZZ_V=\"it's\"; set | grep '^ZZ_V='",
    "alias a1=\"it's\"; alias a1",
    "echo \"[${CDPATH-u}][${MAILCHECK-u}][${TMOUT-u}]\"",
    "alias a1=echo; command -v a1",
    "kill -SIGCONT $$ 2>/dev/null; echo after",
    "echo \"\\x41|\\101|\\0101|a\\cb\"",
    "type if",
    "set -o pipefail 2>/dev/null; echo after",
    "set -o noglob; echo ok",
    "printf '\\x41|\\101|a\\cb\\n'; echo",
    // printf: no `%n`, `%b` keeps `\\e`, `%a` is the hex float, `%05s` zero-pads;
    // `read -p PROMPT`; a bad name is 2; `[^b]` is not a negated class.
    "printf '%n\\n' 2>/dev/null; echo rc=$?; printf '%b\\n' '\\e'; printf 'a\\n' | { read -p P x; echo \"[$x]\"; }; printf 'a\\n' | { read 1x 2>/dev/null; echo rc=$?; }; case abc in a[^b]c) echo 1;; *) echo 2;; esac; printf '%a\\n' 1.5; printf '%05s|\\n' ab",
    // `kill -l N M` names only the first, `kill -l 0` is 2; a failed null-command
    // redirection is 2; `{fd}` is a word; bare `type` / `unset` / `unalias` are
    // no-ops; `command -v` succeeds when any name is found; `unalias -a NAME` is
    // fine; `alias =z` is 1; a failed write is 1; an unclosed `[` stays literal.
    "kill -l 9 15; kill -l 0 2>/dev/null; echo rc=$?; < /nonexistent 2>/dev/null; echo rc=$?; echo a {fd}>/dev/null; echo rc=$?; type; echo rc=$?; unset; echo rc=$?; command -v echo nosuch; echo rc=$?; unalias -a x; echo rc=$?; unalias; echo rc=$?; alias =z 2>/dev/null; echo rc=$?; echo a 1>&- 2>/dev/null; echo rc=$?; echo b[in",
    // `type` knows none of zsh's extra keywords; there is no `builtin` (127);
    // kill usage errors are 2; a bare `export` lists `export NAME='v'`; an
    // ignored signal stays listed in a subshell.
    "type [[ 2>/dev/null; echo rc=$?; builtin nosuch 2>/dev/null; echo rc=$?; kill -s NOSUCH $$ 2>/dev/null; echo rc=$?; kill %1 2>/dev/null; echo rc=$?; zz=1; export zz; export | grep zz; trap '' INT; (trap); trap | grep -c INT",
    // `set` letters map to dash's option names (`-E` emacs, `-f` noglob);
    // `interactive`/`debug`/`nolog` are accepted; `shift 1 2` shifts by 1.
    "set -E; set -o interactive; set -o debug; set -o nolog; echo rc=$?; set -f; echo /etc/pass*; set +f; echo /etc/pass*; set -- a b c; shift 1 2; echo $#",
    // Blanks around a `test` integer; `hash -d` is an illegal option (2);
    // `kill -l -1` is an illegal option (2), `kill -l ' 1'` is HUP; `${#x}`
    // counts bytes; `builtin` is a command not found (127) with or without a
    // redirection; `usr1` is a signal name but `sigusr1` is not; `command`
    // runs builtins; `10>` is the word `10`.
    "test '1 ' -eq 1; echo $?; hash -d 2>/dev/null; echo $?; hash -d ls 2>/dev/null; echo $?; kill -l -1 2>/dev/null; echo $?; kill -l ' 1'; x=héllo; echo ${#x}; builtin 2>/dev/null; echo rc=$?; builtin >/dev/null 2>&1; echo rc=$?; trap 'echo t' usr1; echo $?; trap 'echo t' sigusr1 2>/dev/null; echo $?; command : ; echo $?; kill -0 $$ 999999 2>/dev/null; echo $?; echo a 10>/dev/null",
    // `>&` takes a single digit: `1>&10` is a syntax error that ends the shell (2).
    "echo a 1>&10; echo no",
    // No array subscripts: `${b[@]}` is a fatal bad substitution (2).
    "echo first; echo ${b[@]}; echo no",
    // `cd ''` is a no-op; `pwd` and `times` ignore operands; an alias listing
    // never ends in an empty '' segment; a `break` inside a function cannot
    // end the caller's loop; `command -v` looks at its first operand only;
    // `local` is a special builtin and `in` a keyword; `type /path` answers
    // any existing path; a WINCH trap is delivered; the EXIT trap runs when a
    // non-interactive `exec` fails.
    "cd ''; echo rc=$?; pwd a >/dev/null; echo $?; times a >/dev/null; echo $?; alias x=\"echo 'q'\"; alias x; f() { break; echo in; }; for i in 1; do f; echo $i; done; command -v echo ls; command -v nonexist echo; echo $?; type local; type in; type /etc; echo $?; trap 'echo t' WINCH; kill -WINCH $$; echo yes; trap 'echo e' EXIT; exec nonexist 2>/dev/null; echo no",
    // A trap raised by `kill` on the shell sees `$?` = 0; a pipeline stage
    // runs the EXIT trap it set; a foreground child or `( … )` fires a CHLD
    // trap; `( trap )` lists the ignored and the subshell's own traps while
    // `$(trap)` lists the parent's; `command -V ALIAS` uses type's wording; a
    // non-numeric kill operand (`--` included) is "Illegal number" (2).
    "trap 'echo $?' USR1; false; kill -USR1 $$; trap 'echo s' EXIT | cat; trap 'echo t' CHLD; (true); echo yes; /usr/bin/true; echo end; trap - CHLD; trap 'echo x' USR2; (trap) | wc -l | tr -d ' '; echo [$(trap)] | grep -c USR2; trap '' INT; (trap 'echo y' INT; trap) | grep -c INT; alias e=echo; command -V e; kill -0 -- $$ 2>/dev/null; echo $?; kill -0 abc 2>/dev/null; echo $?",
    // The ignore an `&` job sets is not listed; an untrapped INT kills the shell
    // (130) with no EXIT trap run.
    "trap 'echo x' USR2; (trap) & wait; echo done; trap 'echo e' EXIT; kill -INT $$; echo no",
];

/// zsh-only `type -t` / `read` flags that are bash letters: zsh answers
/// `bad option` with status 1 (Src/builtin.c:123 / :109).
/// Scripts whose ksh93u+m reference output only matches from 1.0.10 on.
const KSH_1_0_10_ONLY: &[&str] = &["echo $((-7/2)) $((-7%3)) $((7/-2)) $((7%-3))"];

const ZSH_BAD_OPTION_CORPUS: &[&str] = &[
    "type -t echo; echo rc=$?",
    "type -t nonexistent_zz; echo rc=$?",
];

/// `(major, minor)` from `<shell> --version` ("version 5.3.20", "5.2.21"), `None` if unparsable.
fn shell_version_at_least(shell: &str, major: u32, minor: u32) -> bool {
    let Ok(out) = std::process::Command::new(shell).arg("--version").output() else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let Some(v) = text.split("version ").nth(1) else {
        return false;
    };
    let mut it = v.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty());
    match (it.next().and_then(|p| p.parse::<u32>().ok()), it.next().and_then(|p| p.parse::<u32>().ok())) {
        (Some(a), Some(b)) => (a, b) >= (major, minor),
        _ => false,
    }
}

/// `--sh` against a `/bin/sh` that is bash (macOS ships bash 3.2 in POSIX mode
/// there). These are the places that shell measurably differs from both bash 5
/// and dash; they are skipped when `/bin/sh` is something else.
const SH_BASH_CORPUS: &[&str] = &[
    // Brace expansion stays on and `$-` carries `B`.
    "echo {a,b}; echo $-",
    // `exit abc` leaves at once with 255; `return` outside a function is status 1.
    "exit abc 2>/dev/null; echo no",
    "return 2>/dev/null; echo rc=$?; f() { return 300; }; f; echo $?",
    // `unset` of a read-only and `set -o BAD` fail with 1 and go on.
    "readonly R=1; unset R 2>/dev/null; echo rc=$?; set -o nope 2>/dev/null; echo rc=$?",
    // `${!x}` indirects, and through an unset name expands to nothing.
    "echo \"[${!x}]\"; x=y; y=v; echo ${!x}",
    // A negative substring length is an error (fatal, 127 at top level).
    "x=abc; echo \"${x:0:-1}\" 2>/dev/null; echo no",
    // Constants wrap in 64 bits; a negative exponent is an error; `--3` is 3.
    "echo $(( 9223372036854775808 )) $(( 99999999999999999999 )) $(( --3 )); echo $(( 2 ** -1 )) 2>/dev/null; echo no",
    // `local` with no operands lists the function's locals as `name=value`.
    "f() { local a=1 b; local; }; f",
    // `printf %b` reads a bare `\\NNN` as octal.
    "printf '%b\\n' 'A\\101\\x41'",
    // Arithmetic names in substrings, `$FUNCNAME`, `$\"…\"`, test's `-a FILE` / `-o OPT`
    // and no `-v`.
    "x=abcdef; i=2; echo ${x:i} ${x:i+1:i}; f() { echo $FUNCNAME; }; f; echo $\"hello\"; [ -a /etc/passwd ]; echo $?; [ -o noglob ]; echo $?; [ -v HOME ] 2>/dev/null; echo $?",
    // printf as in bash 5 (`\\c` literal, `\\E`, `%a`, `-n`), `%c` with no operand
    // prints nothing; `read -e` off a pipe is plain, `-N` does not exist.
    "printf 'a\\cb\\n'; printf '%s\\n' \"$(printf 'x\\Ey' | tr '\\033' _)\"; printf '%a\\n' 1.5; printf -n a 2>/dev/null; echo rc=$?; printf '[%c]\\n'; printf 'a\\n' | { read -e x; echo \"[$x]\"; }; printf 'a\\n' | { read -N 2 x 2>/dev/null; echo rc=$?; }",
    // Brace ranges as in bash 5; `;&` is a syntax error.
    "echo {Z..a} {0..02}; case abc in abc) echo 1;& xyz) echo 2;; esac 2>/dev/null; echo rc=$?",
    // Bare `type` / `unset` are no-ops; `command -v` succeeds when any name is
    // found; an unclosed `[` stays literal; no `nameref`.
    "type; echo rc=$?; unset; echo rc=$?; command -v echo nosuch; echo rc=$?; cd /; echo b[in; nameref r=x 2>/dev/null; echo rc=$?",
    // Blanks around a `test` integer; a `-X` word after `kill -l` is a signal
    // spec and the names are listed on one line; `kill` is 0 when any pid got
    // the signal; `alias -p` adds the prefix; a tilde prefix ends at `:`; a
    // multi-digit fd before `>`; `command` runs builtins; the declaration
    // commands are builtins.
    "test '1 ' -eq 1; echo $?; kill -l -x | head -c 7; echo; kill -0 $$ 999999 2>/dev/null; echo $?; alias x=y; alias -p; cd /; echo ~:x | head -c 3; echo; echo a 10>/dev/null; command : ; echo $?; type local | head -c 30; echo",
    // `cd ''` is a no-op; `pwd` and `times` ignore operands; `umask -p`;
    // `readonly -p` lists the named variables as `readonly NAME="value"`;
    // `unset NAME` drops a function; `]]`, `if` and `in` are keywords; bash's
    // `type` wording for aliases, with not-found on stderr; `type /path`
    // answers an existing path; a WINCH trap is delivered; the EXIT trap runs
    // when a non-interactive `exec` fails, and the shell then exits 0.
    "cd ''; echo rc=$?; pwd a >/dev/null; echo $?; umask -p | head -c 6; echo; times a >/dev/null; echo $?; readonly r=1; readonly -p | grep ' r='; f() { :; }; unset f; type f 2>/dev/null; echo rc=$?; command -v ']]'; echo $?; alias e=echo; type e; type nonexist 2>/dev/null; echo rc=$?; type if; type in; type nonexist ls 2>/dev/null | wc -l | tr -d ' '; type /etc 2>/dev/null; echo rc=$?; trap 'echo t' WINCH; kill -WINCH $$; echo yes; trap 'echo e' EXIT; exec nonexist 2>/dev/null; echo no",
    // A trap raised by `kill` on the shell sees `$?` = 0; `trap -p`; `$(trap)`
    // lists the parent's traps while `( trap )` does not; no `coproc`; `test
    // ! ! X` is "too many arguments" (2); `kill -l SIGHUP` and a number past
    // the table are invalid (1); an untrapped TERM runs the EXIT trap, and so
    // does `exit` inside a trap.
    "trap 'echo $?' USR1; false; kill -USR1 $$; trap 'echo t' TERM; trap -p; trap -p TERM; trap 'echo x' USR2; (trap) | grep -c USR2; echo [$(trap)] | grep -c USR2; command -v coproc; echo $?; test ! ! a; echo $?; test ! ! ! a; echo $?; test ! ! -n a; echo $?; kill -l SIGHUP 2>/dev/null; echo rc=$?; kill -l HUP; kill -l 64 2>/dev/null; echo rc=$?; kill -l 129; trap 'echo e' EXIT; trap 'echo T; exit 3' TERM; kill -TERM $$; echo no",
    // An untrapped HUP runs the EXIT trap before the shell dies (129).
    "trap 'echo e' EXIT; kill -HUP $$; echo no",
    // Setting an EXIT trap arms the terminating signals (bash's set_signal): an
    // untrapped ALRM then runs the trap (142); QUIT kills bash 3.2 without it;
    // an untrapped INT exits 130; the ignore an `&` job sets is not listed.
    "trap 'echo e' EXIT; kill -ALRM $$; echo no",
    "kill -INT $$; echo no",
    "trap 'echo e' EXIT; kill -QUIT $$; echo no",
    "trap 'echo x' USR2; (trap) & wait; echo done",
];

/// True when `path` is a bash build (`$BASH_VERSION` is set when it runs).
fn is_bash(path: &str) -> bool {
    std::process::Command::new(path)
        .args(["-c", "echo ${BASH_VERSION:+bash}"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "bash")
}

/// True when `path` resolves to dash (Debian/Ubuntu's `/bin/sh`).
fn resolves_to_dash(path: &str) -> bool {
    std::fs::canonicalize(path)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .is_some_and(|n| n == "dash")
}

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

/// The released zsh zshrs reports as its own version (5.9.2): `$ZSHRS_RELEASE_ZSH`,
/// then the `scripts/build_zsh_oracle.sh` install of the 5.9.2 tag, and only then a
/// system zsh from `ZSH`. Probes whose answer follows the option table (`setopt`
/// lists `restricted` in 5.9.2; upstream master dropped it, 54181) must compare
/// against the release, not the development tree.
fn release_zsh() -> Option<String> {
    if let Ok(p) = std::env::var("ZSHRS_RELEASE_ZSH") {
        if Path::new(&p).exists() {
            return Some(p);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let built = Path::new(&home).join(".cache/zshrs/zsh-5.9.2/bin/zsh");
        if built.exists() {
            return Some(built.to_string_lossy().into_owned());
        }
    }
    find_shell(ZSH)
}

/// Wall-clock limit for one script. A shell that hangs is a divergence, not a stuck CI job.
const SCRIPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Working directory every script runs in: an empty per-process scratch directory, so a
/// redirection a non-`[[` shell reads out of `[[ a < b ]]` (or any stray `> file`) can neither
/// find a file lying in the repository nor leave one behind.
fn scratch_cwd() -> &'static Path {
    static CWD: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    CWD.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("zshrs-parity-cwd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch cwd");
        dir
    })
}

/// What `run` reports for a script that exceeded [`SCRIPT_TIMEOUT`].
const TIMED_OUT: &str = "<<TIMED OUT>>";

/// (stdout, success). stderr is intentionally dropped — its text
/// legitimately differs across shells; only stdout + exit-sign are compared.
/// A script still running after [`SCRIPT_TIMEOUT`] is killed and reported as
/// `(TIMED_OUT, false)`.
fn run(bin: &str, args: &[&str], script: &str) -> (String, bool) {
    use std::io::Read;
    use std::process::Stdio;
    let mut full: Vec<&str> = args.to_vec();
    full.push("-c");
    full.push(script);
    let mut child = Command::new(bin)
        .args(&full)
        .current_dir(scratch_cwd())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    // Drain stdout on its own thread so a chatty script cannot fill the pipe and stall.
    let mut pipe = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        buf
    });
    let deadline = std::time::Instant::now() + SCRIPT_TIMEOUT;
    let status = loop {
        match child.try_wait().unwrap_or_else(|e| panic!("wait {bin}: {e}")) {
            Some(st) => break Some(st),
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    };
    let out = reader.join().unwrap_or_default();
    match status {
        Some(st) => (String::from_utf8_lossy(&out).into_owned(), st.success()),
        None => (TIMED_OUT.to_string(), false),
    }
}

/// Run one corpus script through a parity case: zshrs with the case flags,
/// and the reference binary (optionally prefixed with `emulate X` for the
/// cross-emulation legs, and `-f` when the reference is zsh).
fn run_case(case: &ParityCase, refbin: &str, script: &str) -> ((String, bool), (String, bool)) {
    // zshrs side: the case's flags, plus `-f` where it means NO_RCS. A drop-in
    // for a real shell (`--bash`, `--ksh`, `--sh`, …) is not told `-f`: in those
    // shells `-f` is `set -f` (noglob), so the zshrs side ran with globbing off
    // against references that glob — and a bare `-c` drop-in reads no startup
    // files anyway, so nothing here needs it.
    let ref_is_zsh = case.ref_emulate.is_some() || case.name == "zsh";
    let mut zargs: Vec<&str> = case.zshrs_flags.to_vec();
    if ref_is_zsh {
        zargs.push("-f");
    }
    let z = run(&zshrs_bin(), &zargs, script);

    // Reference side. Cross-emulation legs run zsh with `emulate X` prepended;
    // a bare zsh reference also takes `-f` (no rc) for determinism.
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

    // `/bin/sh` is bash 3.2 in POSIX mode, which still brace-expands
    // (`sh -c "printf '%s ' {a,b,c}"` → `a b c `); the drop-in follows it.
    let out = Command::new(zshrs_bin())
        .args(["--sh", "-f", "-c", "printf '%s ' {a,b,c}"])
        .output()
        .expect("spawn");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "a b c ",
        "--sh brace-expands like /bin/sh (bash 3.2)"
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
fn bash_brace_step_direction_comes_from_the_endpoints() {
    // bash walks from the LEFT endpoint and ignores the step's sign; a zero
    // step is 1. zsh instead reverses for a negative step and leaves a zero
    // step literal. Each expected string is /opt/homebrew/bin/bash output.
    let run = |mode: &str, script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args([mode, "-f", "-c", script])
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    };
    assert_eq!(run("--bash", "echo {f..a..2}"), "f d b");
    assert_eq!(run("--bash", "echo {f..a..-2}"), "f d b");
    assert_eq!(run("--bash", "echo x{a..e..-2}y"), "xay xcy xey");
    assert_eq!(run("--bash", "echo {6..1..-2}"), "6 4 2");
    assert_eq!(run("--bash", "echo {1..6..-2}"), "1 3 5");
    assert_eq!(run("--bash", "echo {a..e..0} {1..3..0}"), "a b c d e 1 2 3");
    // zsh semantics are unchanged.
    assert_eq!(run("--zsh", "echo {1..6..-2}"), "5 3 1");
    assert_eq!(run("--zsh", "echo {1..3..0}"), "1..3..0");
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

/// Single-quote `s` for the shell.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Combinatorial scripts every way must agree on: each family is a grid of operators
/// over operands, so one table row covers dozens of cases. Built once; every entry is
/// POSIX-portable (printf for output, quoted expansions, no stderr dependence).
fn grid_corpus() -> &'static [String] {
    static GRID: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    GRID.get_or_init(|| {
        let mut out: Vec<String> = Vec::new();
        // ${v#p} ${v##p} ${v%p} ${v%%p}: every operand against every pattern.
        let vals = ["abc", "a.b.c", "a b", "/x/y/z", "aaa", "a*b", "", "abcabc"];
        let pats = ["a", "a*", "*a", "?", "*", "*.", "*/", "b*c", "[ab]", "?*", "*c"];
        for v in vals {
            for op in ["#", "##", "%", "%%"] {
                for p in pats {
                    out.push(format!("v={}; printf '[%s]\\n' \"${{v{op}{p}}}\"", sq(v)));
                }
            }
        }
        // - :- + :+ = := # applied to an unset, an empty and a set variable.
        for pre in ["unset v;", "v=;", "v=x;"] {
            for form in ["${v-d}", "${v:-d}", "${v+p}", "${v:+p}", "${v=a}", "${v:=a}", "${#v}"] {
                out.push(format!("{pre} printf '[%s][%s]\\n' \"{form}\" \"${{v-unset}}\""));
            }
        }
        // Binary arithmetic operators over sign / zero / shift-width operands.
        for (a, b) in [(7, 3), (-7, 3), (0, 5), (2, 10), (255, 16), (1, 0)] {
            for op in ["+", "-", "*", "<<", ">>", "&", "|", "^", "<", ">", "<=", ">=", "==", "!=", "&&", "||"] {
                out.push(format!("printf '%s\\n' \"$(( {a} {op} {b} ))\""));
            }
            if b != 0 {
                for op in ["/", "%"] {
                    out.push(format!("printf '%s\\n' \"$(( {a} {op} {b} ))\""));
                }
            }
        }
        // case: every subject against every pattern shape. `[a-c]` against an upper-case
        // letter is left out — it depends on the locale's collation, not the shell.
        let subjects = ["abc", "a.b", "", "A", "5", "a b", "abc.txt", ".hid"];
        let case_pats = [
            "a*", "*c", "?b?", "[a-c]*", "[!a]*", "*", "\"\"", "a|b", "\"a b\"", "[[:digit:]]", "*.*", "*.txt", ".*",
            "[[:upper:]]", "?",
        ];
        for subj in subjects {
            for pat in case_pats {
                if subj == "A" && pat == "[a-c]*" {
                    continue;
                }
                out.push(format!("case {} in {pat}) printf match;; *) printf no;; esac; printf '\\n'", sq(subj)));
            }
        }
        // test: string, integer, unary and file operators.
        for (a, b) in [("a", "a"), ("a", "b"), ("", ""), ("a", ""), ("10", "9"), ("abc", "abd")] {
            for op in ["=", "!="] {
                out.push(format!("[ {} {op} {} ]; printf '%s\\n' $?", sq(a), sq(b)));
            }
        }
        for (a, b) in [(1, 2), (2, 1), (3, 3), (-1, 1), (10, 9)] {
            for op in ["-eq", "-ne", "-lt", "-le", "-gt", "-ge"] {
                out.push(format!("[ {a} {op} {b} ]; printf '%s\\n' $?"));
            }
        }
        for u in ["-z", "-n"] {
            for v in ["", "x", " "] {
                out.push(format!("[ {u} {} ]; printf '%s\\n' $?", sq(v)));
            }
        }
        for f in ["-e", "-d", "-f", "-r", "-w", "-x", "-s", "-L", "-h"] {
            for path in ["/", "/etc/passwd", "/nonexistent_zz", "/bin/sh"] {
                out.push(format!("[ {f} {path} ]; printf '%s\\n' $?"));
            }
        }
        // Field splitting: IFS shapes against separator layouts.
        for ifs in ["\":\"", "\" \"", "\",;\"", "\"\"", "\":\"\" \"", "' \\t\\n'"] {
            for v in ["a:b:c", "a b  c", ":a:", "a,b;c", "", "a::b", " a b "] {
                out.push(format!("IFS={ifs}; v={}; set -- $v; printf '%s|' \"$#\" \"$@\"; printf '\\n'", sq(v)));
            }
        }
        // printf conversions over words and numbers (`%%` with an argument is mksh-specific).
        for fmt in ["%s", "%5s", "%-5s|", "%.2s", "%d", "%5d", "%-5d|", "%05d", "%+d", "%x", "%X", "%o", "%c", "%b"] {
            for val in ["abc", "42", "-7", "0", "a b"] {
                out.push(format!("printf {} {}; printf '\\n'", sq(fmt), sq(val)));
            }
        }
        // Positional parameters, shift counts and counting loops.
        for n in 0..5 {
            let words: String = (0..n).map(|i| format!("a{i}")).collect::<Vec<_>>().join(" ");
            let words = if n == 0 { "--".to_string() } else { words };
            out.push(format!("set -- {words}; printf '%s\\n' \"$#\" \"${{1-u}}\" \"${{{}-u}}\"", n.max(1)));
        }
        for n in 0..4 {
            out.push(format!("set -- a b c; shift {n} 2>/dev/null; printf '%s\\n' \"$#\""));
            out.push(format!("i=0; while [ $i -lt {n} ]; do i=$((i+1)); done; printf '%s\\n' $i"));
            out.push(format!("i=0; until [ $i -ge {n} ]; do i=$((i+1)); done; printf '%s\\n' $i"));
        }
        // Exit statuses through subshells, functions, child shells and substitutions.
        for code in [0, 1, 2, 127, 255] {
            out.push(format!("(exit {code}); printf '%s\\n' $?"));
            out.push(format!("f() {{ return {code}; }}; f; printf '%s\\n' $?"));
            out.push(format!("sh -c 'exit {code}'; printf '%s\\n' $?"));
            out.push(format!("x=$(exit {code}); printf '%s\\n' $?"));
        }
        for conn in ["&&", "||"] {
            for a in ["true", "false"] {
                for b in ["true", "false"] {
                    out.push(format!("{a} {conn} {b}; printf '%s\\n' $?"));
                }
            }
        }
        for a in ["true", "false"] {
            for b in ["true", "false"] {
                out.push(format!("{a} | {b}; printf '%s\\n' $?"));
                out.push(format!("! {a} | {b}; printf '%s\\n' $?"));
            }
        }
        out
    })
}

/// Scripts kept in `tests/data/emulation_extra_cases.txt`, one per line (`<NL>` stands for
/// a newline inside a script). Each was measured identical across all nine ways: `"$@"` /
/// `"$*"` / `$@` / `$*` under several IFS values, nested parameter expansions,
/// `break N` / `continue N`, function scope and return status, redirections and here-documents,
/// the `set -e` matrix, EXIT and signal traps, `read` with IFS variants, arithmetic
/// assignment forms and subshell isolation.
fn extra_corpus() -> &'static [String] {
    static EXTRA: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    EXTRA.get_or_init(|| {
        include_str!("data/emulation_extra_cases.txt")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.replace("<NL>", "\n"))
            .collect()
    })
}

/// Dialect scripts from `tests/data/emulation_dialect_cases.txt`: `ways<TAB>script`, where
/// `ways` lists the ways (by `ParityCase::name`) the script was measured identical on —
/// arrays, `${v//p/r}` / `${v:o:l}` / `${v^^}`, `[[ ]]`, `(( ))` / `let`, brace expansion,
/// here-strings, process substitution, `printf -v`, `$'..'`, `typeset` / `declare`, `+=`.
/// A script a shell does not have is left out of that shell's list, never edited.
fn dialect_corpus() -> &'static [(Vec<String>, String)] {
    static DIALECT: std::sync::OnceLock<Vec<(Vec<String>, String)>> = std::sync::OnceLock::new();
    DIALECT.get_or_init(|| {
        include_str!("data/emulation_dialect_cases.txt")
            .lines()
            .filter_map(|l| {
                let (ways, script) = l.split_once('\t')?;
                Some((
                    ways.split(',').map(str::to_string).collect(),
                    script.replace("<NL>", "\n"),
                ))
            })
            .collect()
    })
}

/// The scripts one way runs, in corpus order, each at most once. Keeping the
/// selection in one function means the per-way report counts exactly what ran.
fn corpus_for(case: &ParityCase, refbin: &str) -> Vec<&'static str> {
    let empty: &'static [&'static str] = &[];
    let name = case.name;
    let real_shell = case.ref_emulate.is_none() && name != "zsh";
    let grid: Vec<&'static str> = grid_corpus()
        .iter()
        .chain(extra_corpus())
        .map(String::as_str)
        .chain(
            dialect_corpus()
                .iter()
                .filter(|(ways, _)| ways.iter().any(|w| w == name))
                .map(|(_, script)| script.as_str()),
        )
        .collect();
    let parts: [&'static [&'static str]; 15] = [
        PORTABLE_CORPUS,
        if case.extended { EXTENDED_CORPUS } else { empty },
        if name == "bash" { BASH_ONLY_CORPUS } else { empty },
        if name == "sh" && is_bash(refbin) { SH_BASH_CORPUS } else { empty },
        if name == "zsh" { ZSH_ONLY_CORPUS } else { empty },
        if name == "zsh" { ZSH_BAD_OPTION_CORPUS } else { empty },
        if matches!(name, "bash" | "ksh") { BASH_KSH_CORPUS } else { empty },
        if name == "ksh" { KSH_ONLY_CORPUS } else { empty },
        if real_shell { REAL_SHELL_CORPUS } else { empty },
        if matches!(name, "bash" | "ksh" | "mksh") { BASH_KORN_CORPUS } else { empty },
        if name == "bash" && shell_version_at_least(refbin, 5, 3) { BASH_53_CORPUS } else { empty },
        if matches!(name, "mksh" | "pdksh") { PDKSH_CORPUS } else { empty },
        if matches!(name, "dash" | "ash") { DASH_FAMILY_CORPUS } else { empty },
        if name == "dash" { DASH_ONLY_CORPUS } else { empty },
        if name != "zsh" { BARE_POSITIONAL_CORPUS } else { empty },
    ];
    // ksh93u+m releases before 1.0.10 (Ubuntu 24.04 ships 1.0.8) reject or format
    // these differently from the 1.0.10 line `--ksh` follows.
    let old_ksh = name == "ksh" && ksh_version(refbin).is_some_and(|v| v < (1, 0, 10));
    let mut seen = std::collections::HashSet::new();
    parts
        .iter()
        .flat_map(|p| p.iter().copied())
        .chain(grid)
        .filter(|script| !(old_ksh && KSH_1_0_10_ONLY.contains(script)))
        .filter(|script| seen.insert(*script))
        .collect()
}

/// First line of `<shell> --version` (stdout or stderr), or the file name when the
/// shell has no such flag (dash, ash). Printed in the report so a failure says which
/// reference binary and release it was measured against.
fn shell_banner(shell: &str) -> String {
    let fallback = || {
        Path::new(shell)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| shell.to_string())
    };
    let Ok(out) = Command::new(shell).arg("--version").stdin(std::process::Stdio::null()).output() else {
        return fallback();
    };
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    // A shell without `--version` prints a usage error; only a line carrying a release
    // number counts as a banner.
    match text
        .lines()
        .map(str::trim)
        .find(|l| l.chars().any(|c| c.is_ascii_digit()) && !l.to_lowercase().contains("illegal option") && !l.contains("Usage"))
    {
        Some(l) => l.chars().take(90).collect(),
        None => fallback(),
    }
}

/// Outcome of one way of the matrix.
struct WayReport {
    name: &'static str,
    refbin: String,
    banner: String,
    cases: usize,
    mismatches: Vec<String>,
}

/// Why a way did not run.
enum WaySkip {
    /// No reference binary installed.
    Missing,
    /// `/bin/sh` is dash: the `sh` way would repeat the `dash` way against a shell
    /// `--sh` is not meant to be (zshrs's `--sh` is POSIX sh, not dash's quirks).
    ShIsDash(String),
}

fn run_way(case: &ParityCase) -> Result<WayReport, WaySkip> {
    let refbin = find_shell(case.candidates).ok_or(WaySkip::Missing)?;
    if case.name == "sh" && resolves_to_dash(&refbin) {
        return Err(WaySkip::ShIsDash(refbin));
    }
    let corpus = corpus_for(case, &refbin);
    let mut mismatches = Vec::new();
    for script in &corpus {
        let ((r_out, r_ok), (z_out, z_ok)) = run_case(case, &refbin, script);
        if r_out != z_out || r_ok != z_ok {
            mismatches.push(format!(
                "  [{}] {script:?}\n    ref: ok={r_ok} out={r_out:?}\n    zrs: ok={z_ok} out={z_out:?}",
                case.name
            ));
        }
    }
    Ok(WayReport {
        name: case.name,
        banner: shell_banner(&refbin),
        refbin,
        cases: corpus.len(),
        mismatches,
    })
}

#[test]
fn emulation_parity_matrix() {
    let require = std::env::var("ZSHRS_REQUIRE_REF_SHELLS").is_ok();
    // Every way is independent (own reference process, own zshrs process), so they run
    // side by side: the matrix costs the slowest way, not the sum of all of them.
    let results: Vec<(&ParityCase, Result<WayReport, WaySkip>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = PARITY_CASES
            .iter()
            .map(|case| (case, scope.spawn(move || run_way(case))))
            .collect();
        handles
            .into_iter()
            .map(|(case, h)| (case, h.join().unwrap_or_else(|_| panic!("way `{}` panicked", case.name))))
            .collect()
    });

    let mut missing: Vec<&str> = Vec::new();
    let mut mismatches: Vec<String> = Vec::new();
    let mut tested = 0usize;
    eprintln!("{:<14} {:>6} {:>10}  reference", "way", "cases", "mismatches");
    for (case, res) in &results {
        match res {
            Ok(r) => {
                tested += 1;
                eprintln!(
                    "{:<14} {:>6} {:>10}  {} ({})",
                    r.name,
                    r.cases,
                    r.mismatches.len(),
                    r.refbin,
                    r.banner
                );
                mismatches.extend(r.mismatches.iter().cloned());
            }
            Err(WaySkip::Missing) => {
                // Optional ways (ash/mksh/pdksh) are best-effort: absence never counts
                // toward the enforced-missing list.
                if !case.optional {
                    missing.push(case.name);
                }
                eprintln!(
                    "{:<14} {:>6} {:>10}  skipped: reference not found{}",
                    case.name,
                    "-",
                    "-",
                    if case.optional { " (optional)" } else { "" }
                );
            }
            Err(WaySkip::ShIsDash(bin)) => {
                eprintln!("{:<14} {:>6} {:>10}  skipped: {bin} is dash (covered by the dash way)", case.name, "-", "-");
            }
        }
    }

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
    assert!(bash("set -o errtrace; set +o")
        .0
        .contains("set -o errtrace"));

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
    assert!(
        !names.contains(&"posix"),
        "posix is off by default: {opts:?}"
    );
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
    // Inside a function it reports the call line and succeeds. Measured on
    // bash 5.3: `bash -c 'f() { caller; }; f'` prints `1 NULL` (no source file
    // under -c); the frame's function name only appears for `caller N`.
    let (out, ok) = run("f() { caller; }; f");
    assert!(ok, "`caller` in a function must succeed");
    assert_eq!(out, "1 NULL\n");
    let (out, ok) = run("g() { f; }; f() { caller 0; }; g");
    assert!(ok, "`caller 0` two frames deep must succeed");
    assert!(out.starts_with("1 g "), "frame names the caller function: {out:?}");
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
        let out = Command::new(zshrs_bin())
            .args(&args)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    for mode in [["--sh"], ["--dash"], ["--ash"], ["--mksh"]] {
        assert_eq!(
            run(&mode, r#"echo "a\tb""#),
            "a\tb\n",
            "{mode:?} echo must interpret escapes like the real shell"
        );
    }
    // ksh93 (the system 93u+ 2012 and Homebrew's 93u+m 1.0.10 alike) keeps escapes
    // literal without -e and honours it; `-E` is not an option there, it is printed.
    assert_eq!(run(&["--ksh"], r#"echo "a\tb""#), "a\\tb\n");
    assert_eq!(run(&["--ksh"], r#"echo -e "a\tb""#), "a\tb\n");
    assert_eq!(run(&["--ksh"], r#"echo -E x"#), "-E x\n");
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
    // The reference is the 5.9.2 release: the `setopt` listing follows the option
    // table, which still has `restricted` there (upstream master dropped it, 54181).
    let Some(zsh) = release_zsh() else {
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
            "[[ abc =~ (b) ]]; print -r -- $?",           // single capture group
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
            run_zshrs(
                flags,
                r#"set -m; printf '%d\n' $?; set +m; printf '%d\n' $?"#
            ),
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
            run_zshrs(
                flags,
                r#"a=(x y z); a[5]=q; a=(m n); print -r -- "${#a[@]}""#
            )
            .0,
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
        run_zshrs(
            &["--bash"],
            r#"case x in x) printf a;;& x) printf b;; esac"#
        )
        .0,
        "ab",
        "--bash `;;&` must fall through to the next pattern test"
    );
    // A non-matching second pattern still stops the output at `a`.
    assert_eq!(
        run_zshrs(
            &["--bash"],
            r#"case x in x) printf a;;& y) printf b;; esac"#
        )
        .0,
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
        // mksh (R59) answers 2 for a read-only violation only; every other
        // personality keeps 1.
        let mksh_want = if body.contains("readonly") { "2\n" } else { "1\n" };
        assert_eq!(
            run_zshrs(&["--mksh"], &sub).0,
            mksh_want,
            "--mksh: `({body})` status"
        );
        for flags in [
            &["--zsh"][..],
            &["--bash"][..],
            &["--ksh"][..],
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
        run_zshrs(
            &["--dash"],
            r#"(exit 5; : "${nope:?}") 2>/dev/null; printf '%d\n' $?"#
        )
        .0,
        "5\n",
        "`exit 5` runs first, so no error is ever raised"
    );
    // …but an error raised BEFORE the exit wins, because exraise assigns
    // exitstatus at the raise. dash agrees: this prints 2.
    assert_eq!(
        run_zshrs(
            &["--dash"],
            r#"(: "${nope:?}"; exit 5) 2>/dev/null; printf '%d\n' $?"#
        )
        .0,
        "2\n"
    );
    // Ordinary non-zero statuses are untouched.
    assert_eq!(
        run_zshrs(&["--dash"], r#"(false); printf '%d\n' $?"#).0,
        "1\n"
    );
    assert_eq!(
        run_zshrs(
            &["--dash"],
            r#"nonexistent_cmd_zz 2>/dev/null; printf '%d\n' $?"#
        )
        .0,
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
            run_zshrs(
                flags,
                r#"true | false | true; print -r -- "[${PIPESTATUS[*]}]""#
            )
            .0,
            "[0 1 0]\n",
            "{flags:?}: PIPESTATUS must carry every stage"
        );
        assert_eq!(
            run_zshrs(
                flags,
                r#"(exit 3) | (exit 4); print -r -- "[${PIPESTATUS[*]}]""#
            )
            .0,
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
        run_zshrs(
            &["--ksh"],
            r#"true | false | true; print -r -- "[${PIPESTATUS[*]}]""#
        )
        .0,
        "[]\n",
        "--ksh (ksh93) has no PIPESTATUS"
    );
    // bash keeps it; zsh's own name is `$pipestatus`, and PIPESTATUS is an
    // ordinary (unset) parameter there.
    assert_eq!(
        run_zshrs(
            &["--bash"],
            r#"true | false | true; printf '[%s]\n' "${PIPESTATUS[*]}""#
        )
        .0,
        "[0 1 0]\n"
    );
    assert_eq!(
        run_zshrs(
            &["--zsh"],
            r#"true | false | true; print -r -- "[${PIPESTATUS[*]}]""#
        )
        .0,
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
            run_zshrs(
                flags,
                r#"x=0; y=${ x=5; print -n out; }; print -r -- "x=$x y=$y""#
            )
            .0,
            "x=5 y=out\n",
            "{flags:?}: a funsub shares the current shell environment"
        );
        assert_eq!(
            run_zshrs(
                flags,
                r#"x=0; y=$(x=5; print -n out); print -r -- "x=$x y=$y""#
            )
            .0,
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
            run_zshrs(
                flags,
                r#"x=0; y=${|x=5; REPLY=v;}; print -r -- "x=$x y=$y""#
            )
            .0,
            "x=5 y=v\n"
        );
        // … but REPLY itself is local to it: the outer value is neither
        // visible inside nor clobbered after.
        assert_eq!(
            run_zshrs(
                flags,
                r#"REPLY=outer; y=${|:;}; print -r -- "[$y][$REPLY]""#
            )
            .0,
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

/// Run `probes` through zshrs (`zflags` + `-f`) and the reference binary
/// `refbin` (prefixed with `ref_prefix`, e.g. `emulate ksh\n`) and collect
/// every stdout / exit-sign mismatch.
fn probe_mismatches(
    zflags: &[&str],
    refbin: &str,
    ref_args: &[&str],
    ref_prefix: &str,
    probes: &[&str],
) -> Vec<String> {
    let mut zargs: Vec<&str> = zflags.to_vec();
    zargs.push("-f");
    probes
        .iter()
        .filter_map(|p| {
            let z = run(&zshrs_bin(), &zargs, p);
            let r = run(refbin, ref_args, &format!("{ref_prefix}{p}"));
            (z != r).then(|| format!("  {p:?}\n    ref: {r:?}\n    zrs: {z:?}"))
        })
        .collect()
}

/// ksh93-only behaviors that bash and zsh do not share, so they cannot sit in
/// the cross-shell corpus: float formatting (`%g`), built-in math functions,
/// ksh's `printf %q` and `typeset -p` spellings, `read -a/-n/-N`, unpadded
/// brace ranges, sparse `unset a[i]`, and `-nt`/`-ot` with a missing file.
/// `(major, minor, patch)` of a ksh93u+m (`93u+m/1.0.10 2024-08-01`), or `None`
/// for any other ksh.
fn ksh_version(ksh: &str) -> Option<(u32, u32, u32)> {
    let out = std::process::Command::new(ksh).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    let rest = text.split("93u+m/").nth(1)?;
    let mut it = rest.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty());
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

#[test]
fn ksh93_specifics_match_ksh93() {
    let Some(ksh) = find_shell(&["/opt/homebrew/bin/ksh", "ksh", "/bin/ksh", "/usr/bin/ksh"]) else {
        eprintln!("skip: ksh not found");
        return;
    };
    // Float arithmetic output changed between ksh93u+m releases: 1.0.8 (Ubuntu
    // 24.04's package) prints 17 significant digits (`1.41421356237309515`),
    // 1.0.10 prints 15 (`1.4142135623731`), which is what `--ksh` matches. The
    // digits-sensitive probe only means something against 1.0.10 or newer.
    let float_digits_probe = "print $((7.0/2)) $((5.0)) $((2**0.5)) $((1.0/3)) $((1e20))";
    let new_float_format = ksh_version(&ksh).is_some_and(|v| v >= (1, 0, 10));
    let probes = [
        "float f=1.5; print $f",
        "typeset -E f=0.3333333333333; print $f",
        "typeset -E2 f=1; print $f",
        if new_float_format { float_digits_probe } else { "print $((7.0/2)) $((5.0))" },
        "print $((sqrt(16))) $((abs(-3))) $((int(3.7)))",
        "printf '%q|' 'a b' \"it's\" 'a;b' abc '' a=b '=x' 'a\"b'; print",
        "printf '%q\\n' \"$(printf 'a\\tb')\"",
        "x='a b'; typeset -p x",
        "x=1; typeset -p x",
        "typeset -xi a=5; typeset -p a",
        "typeset -rx a=5; typeset -p a",
        "typeset -a a=('a b' c); typeset -p a",
        "typeset -i16 a=255; typeset -p a",
        "typeset -F2 a=1; typeset -p a",
        "typeset -E2 a=1; typeset -p a",
        "typeset -Z3 a=7; typeset -p a",
        "typeset -L3 a=7; typeset -p a",
        "typeset -u a=x; typeset -p a",
        "read -a arr <<< 'a b c'; print ${arr[1]}",
        "read -n 2 a <<< abcd; print $a",
        "read -N 2 a <<< abcd; print $a",
        "echo {01..03} {1..03} {1..3}",
        "a=(x y z); unset a[1]; print ${#a[@]} ${!a[@]}",
        "[[ /etc/passwd -nt /nonexistent ]] && print nt",
        "[[ /nonexistent -ot /etc/passwd ]] && print ot",
        "[[ /nonexistent -nt /nonexistent2 ]] || print n",
        "trap 'print a' INT; trap -p",
        r"print -e 'a\tb'",
        r"print '\0101|\x41|\101|\e|\U00000041|\q|\E[0m|A|\00101'",
        r"print -r '\0101'",
        r"print 'a\cb'; print x",
        r"echo -e '\0101|\E|\x41|\e|\U00000041'",
        r"printf '%b\n' '\0101|\x41|\E|\e|\U00000041'",
        "typeset -A h; h[a b]=1; print \"${h[a b]}\"; print ${!h[@]}",
        "a[ 2 ]=x; print ${!a[@]}",
        "function f { trap 'print bye' EXIT; print in; }; f; print after",
        "f() { trap 'print bye' EXIT; print in; }; f; print after",
        "echo {a..e..2} {e..a..-2} {1..10..3} {10..1..-4} {10..1..4} {1..5..0}",
        "echo {a..e..a} {5..1..1} {a..e..-1}",
        "printf '%.0f %.0f %.0f %.0f %.1f %.0e\\n' 0.5 1.5 2.5 3.5 0.25 2.5",
        "f() { typeset x=1; }; f; print ${x-unset}",
        "function f { typeset x=1; }; f; print ${x-unset}",
        "f() { integer x=1; }; f; print ${x-unset}",
        "a[1+1]=x; print ${!a[@]}",
        "i=2; a[i]=x; print ${!a[@]}; a[i+1]=y; print ${!a[@]}",
        "whence -v nosuch; print $?",
        "whence -v if",
        "let; print $?",
        "trap 'print int' INT; trap -p INT",
        "trap 'print a' INT TERM; trap -p INT TERM HUP",
        "print ${KSH_VERSION+set}",
        "cd /usr; cd /bin; OLDPWD=/etc; cd - >/dev/null; pwd",
        // ERR: a failing command AND a function call that returns non-zero each
        // fire it (`return N` itself does not); a subshell starts with no traps.
        "trap 'print ERR' ERR; f() { false; }; f; print end",
        "trap 'print ERR' ERR; f() { return 3; }; f; print end",
        "trap 'print ERR' ERR; f() { g; }; g() { false; }; f; print end",
        "trap 'print ERR' ERR; f() { g; }; g() { return 2; }; f; print end",
        "trap 'print ERR' ERR; f() { (exit 4); }; f; print end",
        "trap 'print ERR' ERR; f() { false; print a; false; }; f; print end",
        "trap 'print ERR' ERR; f() { { false; }; }; f; print end",
        "trap 'print ERR' ERR; f() { false; }; f && print x; print end",
        "trap 'print ERR' ERR; f() { false; }; (f); print end",
        "trap 'print ERR' ERR; (false); print end",
        "trap 'print ERR' ERR; ( false; true ); print end",
        "trap 'print ERR' ERR; echo $(false); print end",
        // `typeset -p` of an unset name is silent and succeeds.
        "typeset -p nosuch; print $?",
        "x=1; [[ -R x ]] && print ref; typeset -n r=x; [[ -R r ]] && print ref",
        // `${!name[sub]}` names the element, with an indexed sub evaluated.
        "a=(x y); i=1; print ${!a[i]} ${!a[0]} ${!a[@]} ${!a[*]}",
        "typeset -A h; h[k]=v; print ${!h[k]} ${!h[@]}",
        // An empty substring offset is not accepted.
        "x=abc; print ${x::2}; print rc=$?",
        "x=abc; print ${x:1:1} ${x:1} ${x: -1}",
        // `~(options)` pattern prefixes.
        "[[ ab == ~(i)AB ]] && print y; print $?",
        "[[ ab == ~(i:AB) ]] && print y",
        "[[ aBc == a~(i)b* ]] && print y",
        "[[ aBC == a~(i)b~(-i)c ]] && print y; print n",
        "[[ abc == ~(i:AB)c ]] && print y",
        "[[ ABC == ~(i)[a-c]* ]] && print y",
        "[[ ABC == ~(i)@(abc|x) ]] && print y",
        "[[ abc != ~(i)ABC ]] && print y; print n",
        "[[ abc == ~(E)b ]] && print y",
        "[[ abc == ~(E)^abc$ ]] && print y",
        "[[ abc == ~(E)^ab$ ]] && print y; print n",
        "[[ ABC == ~(iE)a.c ]] && print y",
        "[[ abc == ~(F)b ]] && print y",
        "[[ abc == ~(F)a.c ]] && print y; print n",
        "[[ ABC == ~(Fi)abc ]] && print y",
        "[[ abc == ~(G)a\\(b\\)c ]] && print y",
        "[[ ab == ~(K)a* ]] && print y",
        "case ABC in ~(i)abc|x) print y;; esac",
        "p=abc; [[ ABC == ~(i)$p ]] && print y",
        // `select` that hits EOF ends with status 1; `type`/`whence -v` word an alias like the `alias` listing.
        "select x in a b; do print $x; false; done <<< 1; print rc=$?",
        "select x in a b; do print $x; done <<< 9; print rc=$?",
        "alias ll='ls -l'; type ll; whence -v ll",
        "alias a=b; type a",
        "alias a=\"it's\"; type a",
        "alias a=\"it's\"; alias a",
        "alias a='x=y' b='p*q' c='~'; alias a b c",
    ];
    let bad = probe_mismatches(&["--ksh"], &ksh, &[], "", &probes);
    assert!(bad.is_empty(), "--ksh diverged from ksh93:\n{}", bad.join("\n"));
}

/// The pdksh line (`--mksh` / `--pdksh`): last pipeline stage in a subshell,
/// 8-bit `return`, sparse `unset a[i]`, missing-file `-nt`/`-ot`, and `trap -p`
/// rejected. mksh stands in for pdksh (no pdksh binary is installed).
#[test]
fn pdksh_line_specifics_match_mksh() {
    let Some(mksh) = find_shell(&["mksh", "/bin/mksh", "/usr/bin/mksh"]) else {
        eprintln!("skip: mksh not found");
        return;
    };
    // mksh releases disagree on an unset readonly: older ones list `typeset -r k=''`,
    // newer ones `typeset -r k`. zshrs mirrors the former, so the probe runs only
    // against a mksh that prints it.
    let unset_readonly_lists_empty_value =
        run_code(&mksh, &[], "typeset -r k; typeset -p k").0 == "typeset -r k=''\n";
    let mut probes = vec![
        "echo x | read v; print \"[$v]\"",
        "f() { return 300; }; f; print $?",
        "h() { return -1; }; h; print $?",
        "a=(x y z); unset a[1]; print ${#a[@]} ${!a[@]}",
        "[[ /etc/passwd -nt /nonexistent ]] && print nt",
        "[[ /nonexistent -ot /etc/passwd ]] && print ot",
        "trap 'print a' INT; trap -p",
        "trap -x; print after",
        "trap -l; print after",
        "trap -ZZ INT; print after",
        "trap -- 'print a' INT; trap 'print b' NOSUCH; print after",
        "x=\"a b\"; echo ${x@Q}; x=abc; echo ${x@Q}; x=; echo ${x@Q}; x=\"it's\"; echo ${x@Q}",
        "x='a*'; echo ${x@Q}",
        "print ${KSH_VERSION+set}",
        "x=\"a b\"; typeset -p x; typeset -r y=1; typeset -p y; typeset -i z=3; typeset -p z",
        "typeset -x w=1; typeset -p w; typeset -u u=ab; typeset -p u; typeset -l lo=AB; typeset -p lo",
        "typeset -Z3 q=1; typeset -p q; typeset -R3 q2=1; typeset -p q2; typeset -L3 lj=abcdef; typeset -p lj",
        "typeset -r -x -i k=1; typeset -p k; typeset -rux a=ab; typeset -p a",
        "typeset -i16 h=255; typeset -p h; typeset -i2 g=5; typeset -p g",
        "for v in 'a:b' 'a=b' 'a#b' 'a*b' 'a\\b' \"a'b\"; do x=$v; typeset -p x; done",
        "set -A arr -- 'a b' c; typeset -p arr; typeset -a q; q[2]=a; typeset -p q",
        "x=$(printf 'a\\tb\\nc'); typeset -p x",
        // `typeset -p` of an unset name is silent and succeeds; `-U` is listed
        // last among the attribute words.
        "typeset -p nosuch; print $?",
        "typeset -U x=1; typeset -p x",
        "typeset -iU x=1; typeset -p x",
        "typeset -xU x=1; typeset -p x",
        "typeset -rU x=1; typeset -p x",
        "typeset -lU x=A; typeset -p x",
        "typeset -Z3 -U x=1; typeset -p x",
        // A `-c` string ignores `set -n` / `set -o noexec` and reports LINENO 0.
        "echo hi; set -n; echo no",
        "set -o noexec; echo hi",
        "echo $LINENO\necho $LINENO",
        "f() { echo $LINENO; }; f",
        "x=abc; echo ${x::2} ${x:1:1}",
        "a=(x y); i=1; print ${!a[i]} ${!a[0]}",
        // ERR: one fire per failing command; a function call adds none; a
        // subshell starts with no traps.
        "trap 'print ERR' ERR; f() { false; }; f; print end",
        "trap 'print ERR' ERR; f() { g; print in; }; g() { false; }; f; print end",
        "trap 'print ERR' ERR; f() { return 3; }; f; print end",
        "trap 'print ERR' ERR; (false); print end",
        "trap 'print ERR' ERR; echo $(false); print end",
        "select x in a b; do print $x; false; done <<< 1; print rc=$?",
        "alias ll='ls -l'; type ll; whence -v ll",
        "alias a=\"it's\"; type a",
    ];
    if unset_readonly_lists_empty_value {
        probes.push("typeset -r k; typeset -p k; typeset x1; typeset -p x1");
    }
    for flags in [&["--mksh"][..], &["--pdksh"][..]] {
        let bad = probe_mismatches(flags, &mksh, &[], "", &probes);
        assert!(bad.is_empty(), "{flags:?} diverged from mksh:\n{}", bad.join("\n"));
    }
}

/// zsh-style `--ksh --zsh` must reject what zsh rejects under `emulate ksh`:
/// the `printf '%(FMT)T'` directive is ksh93/bash-only and zsh 5.9.2 answers
/// "invalid directive" even in ksh emulation.
#[test]
fn ksh_zsh_style_rejects_printf_time_directive_like_zsh() {
    let Some(zsh) = find_shell(ZSH) else {
        eprintln!("skip: zsh not found");
        return;
    };
    let probes = ["printf '%(%Y)T\\n' 0; print $?"];
    let bad = probe_mismatches(&["--ksh", "--zsh"], &zsh, &["-f"], "emulate ksh\n", &probes);
    assert!(bad.is_empty(), "--ksh --zsh diverged from zsh:\n{}", bad.join("\n"));
}

/// `cd -` and `~-` resolve through the shell's internal `oldpwd` in zsh
/// (c:Src/builtin.c:909 `: oldpwd`, c:Src/subst.c:755), so assigning
/// `OLDPWD=…` does not redirect them. dash, bash, ksh93 and mksh read the
/// `$OLDPWD` variable itself, so the same assignment does. Both halves were
/// measured on the real shells.
#[test]
fn cd_dash_oldpwd_source_follows_the_emulated_shell() {
    let cd_minus = "cd /usr; cd /bin; OLDPWD=/etc; cd - >/dev/null; pwd";
    let tilde_minus = "cd /usr; cd /bin; OLDPWD=/etc; echo ~-";
    for flags in [&["--zsh"][..], &[][..]] {
        assert_eq!(run_zshrs(flags, cd_minus).0, "/usr\n", "{flags:?}: cd -");
        assert_eq!(run_zshrs(flags, tilde_minus).0, "/usr\n", "{flags:?}: ~-");
    }
    for flags in [
        &["--bash"][..],
        &["--dash"][..],
        &["--sh"][..],
        &["--ksh"][..],
        &["--mksh"][..],
    ] {
        assert_eq!(run_zshrs(flags, cd_minus).0, "/etc\n", "{flags:?}: cd -");
    }
    for flags in [&["--bash"][..], &["--ksh"][..], &["--mksh"][..]] {
        assert_eq!(run_zshrs(flags, tilde_minus).0, "/etc\n", "{flags:?}: ~-");
    }
    // The assignment is not required to be the only way OLDPWD changes:
    // a later cd re-seats the internal value in zsh.
    let (out, _) = run_zshrs(&["--zsh"], "cd /usr; OLDPWD=/etc; cd /bin; OLDPWD=/var; cd - >/dev/null; pwd");
    assert_eq!(out, "/usr\n");
}

/// Run `bin args -c script` with stdin from /dev/null; (stdout, exit code).
fn run_code(bin: &str, args: &[&str], script: &str) -> (String, i32) {
    let out = Command::new(bin)
        .args(args)
        .args(["-c", script])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// dash/ash specifics that the cross-shell corpus cannot hold because bash and
/// zsh disagree with dash: fatal syntax errors (status 2, also in `-c`, `eval`
/// and dot-sourced files), usage-error statuses, `kill -l NAME`, literal
/// `~+`/`~-`, `\0NNN` echo escapes and dash's strict arithmetic variables.
/// Compared on stdout AND exit status against the real dash; `--ash` is the
/// same shell and must agree.
#[test]
fn dash_specifics_match_dash() {
    let Some(dash) = find_shell(&["/opt/homebrew/bin/dash", "/bin/dash", "dash"]) else {
        eprintln!("skip: dash not found");
        return;
    };
    // dash releases disagree on `type -a`: older ones take `-a` for a command name
    // (`-a: not found`, 127), newer ones reject it as an illegal option (2), which is
    // what zshrs mirrors. The probe runs only against a dash that rejects it.
    let type_rejects_options = run_code(&dash, &[], "type -a echo").1 == 2;
    let mut probes = vec![
        "echo (",
        "echo a; if; echo b",
        "echo a; eval 'if'; echo after",
        "echo 'echo (' > \"${TMPDIR:-/tmp}/zshrs_dash_probe.sh\"; echo a; . \"${TMPDIR:-/tmp}/zshrs_dash_probe.sh\"; echo after",
        "kill -l INT",
        "kill -l 9",
        "echo ~+ ~-",
        "echo '\\101\\0101\\060'",
        "local",
        "f() { local; echo in; }; f",
        "getopts; echo $?",
        "getopts a; echo $?",
        "printf '%q\\n' a",
        "printf '%d\\n' 5; printf '%q\\n' a; echo after",
        "echo $((0x))",
        "echo $(( ))",
        "x=abc; echo $((x+1))",
        "x=1+2; echo $((x*2))",
        "x=; echo $((x+1))",
        "x=' 5 '; echo $((x+1))",
        "x=0x10; echo $((x+1))",
        "x=010; echo $((x+1))",
        "x=-3; echo $((x+1))",
        "echo $((010)) $((0x1f)) $((1<<3)) $((-8>>1)) $((1?2:3)) $((1&&0)) $((!5)) $((~5)) $((-5%3))",
        "shift abc; echo after",
        "break; echo $?",
        "[ a == a ]; echo $?",
        "type if; type :; type echo",
        "x=5; f() { local x; echo \"[$x]\"; }; f",
        "set +f; set -eu; echo $-",
        // builtins and specials dash does not have
        "let 1+1",
        "echo \"[$RANDOM][$SECONDS]\"; RANDOM=5; echo $RANDOM",
        "typeset x=1; echo $x",
        // $LINENO inside a function counts from the definition line as 1
        // option and directive rejections
        "command -x ls",
        "ulimit -z",
        "ulimit -u",
        "printf '%ld\\n' 3",
        "printf 'a\\cb\\n'",
        "echo -n -n a",
        "echo -nn a",
        "trap -x; echo no",
        "trap -; echo $?",
        "unset; echo $?",
        // grammar: one `;` after a command, none before it or after an operator
        "echo hi; ;",
        "echo hi && ; echo",
        "echo a | ; echo",
        "echo a &&",
        "; echo a",
        // export -p / readonly -p listings
        "export ZZA; export -p | grep ZZA",
        "readonly ZZC; readonly -p | grep ZZC",
        "export ZZA=\"it's\"; export -p | grep ZZA",
        // dash's own test grammar
        "[ \\( \\) ]; echo $?",
        "[ -v HOME ]; echo $?",
        "[ -t a ]; echo $?",
        "test ! -a x; echo $?",
        "test -f -a -f; echo $?",
        "[ ! ! 1 = 1 ]; echo $?",
        "[ a = a -o b = c -a c = d ]; echo $?",
    ];
    if type_rejects_options {
        probes.push("type -a echo");
    }
    // Some dash builds (Ubuntu's package) print nothing for `$LINENO` inside a function,
    // so that probe runs only against a dash that numbers function lines.
    if run_code(&dash, &[], "f(){ echo $LINENO; }; f").0.trim() != "" {
        probes.push("f(){ echo $LINENO; echo $LINENO; }; f");
    }
    // Missing-file `-nt` / `-ot` and an out-of-range integer in `[` are answered
    // differently by dash builds (macOS's and Ubuntu's both say 1 / 1 / 2). zshrs
    // mirrors the dash that answers 0 for each, so a probe runs only against a dash
    // that does.
    for probe in [
        "[ /bin/sh -nt /nonexistent ]; echo $?",
        "[ /nonexistent -ot /bin/sh ]; echo $?",
        "[ 99999999999999999999 -gt 1 ]; echo $?",
    ] {
        if run_code(&dash, &[], probe).0 == "0\n" {
            probes.push(probe);
        }
    }
    for flag in ["--dash", "--ash"] {
        let bad: Vec<String> = probes
            .iter()
            .filter_map(|p| {
                let z = run_code(&zshrs_bin(), &[flag, "-f"], p);
                let r = run_code(&dash, &[], p);
                (z != r).then(|| format!("  {p:?}\n    dash: {r:?}\n    zrs:  {z:?}"))
            })
            .collect();
        assert!(bad.is_empty(), "{flag} diverged from dash:\n{}", bad.join("\n"));
    }
}

/// Under KSH emulation an unbraced `$assoc[key]` does not subscript: `$assoc`
/// is `${assoc[0]}` (empty without a key "0") and `[key]` stays literal text
/// (Src/params.c:2342-2358, Src/subst.c:2800-2802). The segmented-word runtime
/// path used to read the first bucket value instead (`x$h[a]` printed `x1[a]`),
/// and a module special hash (`$options[a]`) printed its first scanned value.
/// An octal-looking variable is octal under OCTALZEROES.
#[test]
fn ksh_zsh_style_unbraced_assoc_subscript_and_octal_vars_match_zsh() {
    let Some(zsh) = find_shell(ZSH) else {
        eprintln!("skip: zsh not found");
        return;
    };
    let probes = [
        "typeset -A h=(a 1); echo x$h[a]; echo $h[a]x; echo x\"$h[a]\"; echo x$h\\[a]",
        "typeset -A h=(a 1); echo $h[a]$h[b]; echo ${h}$h[a]",
        "typeset -A h=(0 z); echo x$h[a]",
        "echo $options[a]x$options[b]",
        "setopt octalzeroes; x=010; echo $((x)) $((x + 1))",
    ];
    let bad = probe_mismatches(&["--ksh", "--zsh"], &zsh, &["-f"], "emulate ksh\n", &probes);
    assert!(bad.is_empty(), "--ksh --zsh diverged from zsh:\n{}", bad.join("\n"));
}

/// dash, ksh, mksh and /bin/sh read a `-c` string a line at a time, so a syntax
/// error on a later line leaves the earlier lines already run (stdout `first`, then
/// the diagnostic). zsh — and the zsh-style legs — parse the whole string first and
/// run nothing. zshrs's real-shell drop-ins used to behave like zsh.
#[test]
fn dash_c_string_runs_the_lines_before_a_syntax_error() {
    let script = "printf 'first\\n'\ncase x in ) printf m;; esac\nprintf 'last\\n'";
    for case in PARITY_CASES {
        if case.ref_emulate.is_some() || case.name == "zsh" {
            continue;
        }
        let Some(refbin) = find_shell(case.candidates) else { continue };
        let ((r_out, _), (z_out, _)) = run_case(case, &refbin, script);
        assert_eq!(r_out, "first\n", "{}: the reference shell's own behaviour changed", case.name);
        assert_eq!(z_out, r_out, "{}: lines before a syntax error must run", case.name);
    }
}
