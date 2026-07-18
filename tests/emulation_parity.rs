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
    ParityCase { name: "zsh",  zshrs_flags: &["--zsh"],  candidates: ZSH, ref_emulate: None, extended: true,  optional: false },
    ParityCase { name: "bash", zshrs_flags: &["--bash"], candidates: &["bash", "/bin/bash", "/usr/bin/bash", "/opt/homebrew/bin/bash"], ref_emulate: None, extended: true, optional: false },
    ParityCase { name: "ksh",  zshrs_flags: &["--ksh"],  candidates: &["ksh", "/bin/ksh", "/usr/bin/ksh"], ref_emulate: None, extended: true,  optional: false },
    ParityCase { name: "sh",   zshrs_flags: &["--sh"],   candidates: &["/bin/sh"], ref_emulate: None, extended: false, optional: false },
    ParityCase { name: "dash", zshrs_flags: &["--dash"], candidates: &["/bin/dash", "/usr/bin/dash"], ref_emulate: None, extended: false, optional: false },
    // ── zshrs --X --zsh (zsh-STYLE) vs real zsh doing `emulate X` ────────
    ParityCase { name: "sh/zsh-style",  zshrs_flags: &["--sh", "--zsh"],  candidates: ZSH, ref_emulate: Some("sh"),  extended: false, optional: false },
    ParityCase { name: "ksh/zsh-style", zshrs_flags: &["--ksh", "--zsh"], candidates: ZSH, ref_emulate: Some("ksh"), extended: true,  optional: false },
    // ── best-effort variants: ash ≈ dash, mksh ≈ ksh (POSIX base only) ───
    ParityCase { name: "mksh", zshrs_flags: &["--mksh"], candidates: &["mksh", "/bin/mksh", "/usr/bin/mksh"], ref_emulate: None, extended: false, optional: true },
    ParityCase { name: "ash",  zshrs_flags: &["--ash"],  candidates: &["ash", "/bin/ash", "/usr/bin/ash"], ref_emulate: None, extended: false, optional: true },
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
    "printf '%s\\n' \"${TERM+set}\"",                              // +alt (unquoted var may be unset→'')
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
    // NB: `local` is intentionally NOT here — ksh93 has no `local` builtin
    // (it uses `typeset`), so it is a legitimate ksh divergence, not a bug.
    // Bare `${a[N]}` single-index is also excluded — 1-based (zsh) vs 0-based
    // (bash/ksh) legitimately differs and would flag a non-bug.
];

fn find_shell(candidates: &[&str]) -> Option<String> {
    for c in candidates {
        if c.starts_with('/') {
            if Path::new(c).exists() {
                return Some((*c).to_string());
            }
        } else if let Ok(out) = Command::new("sh").args(["-c", &format!("command -v {c}")]).output() {
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
    let out = Command::new(bin).args(&full).output().unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.success())
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
        (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.success())
    };
    // ash ≡ dash: strict-POSIX rejections + posix-faithful splitting.
    for script in [
        "echo $((2**10))",                          // dash arith: `**` rejected
        "[[ 1 = 1 ]] && echo y",                    // `[[` not reserved
        "IFS=:; v=a:b:; set -- $v; printf %s \"$#\"", // trailing-empty drop → 2
        "printf '%d' A",                            // strtoimax printf → exit 1
    ] {
        assert_eq!(probe("--ash", script), probe("--dash", script), "--ash vs --dash: {script}");
    }
    // mksh ≡ ksh: same emulation base (ksharrays etc.).
    for script in [
        "a=(x y z); printf '%s' \"${a[0]}\"",       // 0-indexed arrays
        "print -r -- ${options[ksharrays]}",
        "print -r -- ${options[shwordsplit]}",
    ] {
        assert_eq!(probe("--mksh", script), probe("--ksh", script), "--mksh vs --ksh: {script}");
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
        ("printf '%s ' {a,b,c}", "a b c "),                   // brace expansion on
        ("printf '%s ' {1..4}", "1 2 3 4 "),                  // brace range
        ("IFS=:; v=a:b:; set -- $v; printf %s \"$#\"", "2"),  // trailing-empty drop
    ];
    for (script, want) in cases {
        let out = Command::new(zshrs_bin())
            .args(["--bash", "-f", "-c", script])
            .output()
            .expect("spawn");
        assert_eq!(String::from_utf8_lossy(&out.stdout), *want, "--bash: {script}");
    }
    // printf %d numeric contract (bash errors on non-numeric, like dash).
    let out = Command::new(zshrs_bin())
        .args(["--bash", "-f", "-c", "printf '%d' A"])
        .output()
        .expect("spawn");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "0");
    assert!(!out.status.success(), "--bash printf %d A should exit non-zero");

    // POSIX sh must NOT brace-expand (regression guard for the gate).
    let out = Command::new(zshrs_bin())
        .args(["--sh", "-f", "-c", "printf '%s ' {a,b,c}"])
        .output()
        .expect("spawn");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "{a,b,c} ", "--sh must not brace-expand");
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
        (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.success())
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
        ("v=abcDEF; printf '%s-%s' \"${v^^}\" \"${v,,}\"", "ABCDEF-abcdef"),
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
        assert!(!out.status.success(), "{mode}: ${{!y}} must not do indirect");
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
    assert_eq!(bash("read -a arr <<< 'x y z'; printf '%s' \"${arr[1]}\""), "y");
    assert_eq!(bash("read -a arr <<< 'one two three'; printf '%s' \"${#arr[@]}\""), "3");
    // ${!arr[@]} indices (3 separate args → joined with a space here).
    assert_eq!(bash("a=(x y z); printf '%s ' \"${!a[@]}\""), "0 1 2 ");
    assert_eq!(
        bash("a=(p q r); for i in \"${!a[@]}\"; do printf '%s:%s ' \"$i\" \"${a[$i]}\"; done"),
        "0:p 1:q 2:r "
    );

    // BASH_REMATCH must stay unset under --zsh (uses $match instead).
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "[[ ab =~ (a) ]]; printf '[%s]' \"${BASH_REMATCH:-unset}\""])
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
    assert_eq!(bash("mapfile -t L <<< $'a\\nb\\nc'; printf '%s' \"${#L[@]}\""), "3");
    assert_eq!(bash("mapfile -t L <<< $'a\\nb\\nc'; printf '%s' \"${L[1]}\""), "b");
    // readarray is an alias.
    assert_eq!(bash("readarray -t L <<< $'x\\ny'; printf '%s' \"${#L[@]}\""), "2");
    // Without -t the trailing delimiter is kept in each element.
    assert_eq!(bash("mapfile L <<< $'x\\ny'; printf '[%s]' \"${L[@]}\""), "[x\n][y\n]");
    // -s skip + -n count.
    assert_eq!(bash("mapfile -t -s 1 -n 2 L <<< $'a\\nb\\nc\\nd'; printf '%s' \"${L[*]}\""), "b c");
    // -d custom delimiter.
    assert_eq!(bash("mapfile -d : -t L <<< 'a:b:c'; printf '%s' \"${L[1]}\""), "b");
    // Default array name is MAPFILE.
    assert_eq!(bash("mapfile -t <<< $'p\\nq'; printf '%s' \"${MAPFILE[0]}\""), "p");

    // Gated to non-zsh: `mapfile` is "command not found" under --zsh.
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "mapfile x"])
        .output()
        .expect("spawn");
    assert!(!out.status.success(), "--zsh: mapfile must be command-not-found");
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
    assert_eq!(bash(r#"a=(x y z); a[5]=q; printf '%s' "${a[*]}""#), "x y z q");
    assert_eq!(bash(r#"a=(x y z); a[5]=q; printf '%s' "${!a[*]}""#), "0 1 2 5");
    // Custom IFS applies to the star-join over live elements only.
    assert_eq!(bash(r#"a=(x y z); a[5]=q; IFS=,; printf '%s' "${a[*]}""#), "x,y,z,q");
    // `unset a[i]` is 0-based and leaves a hole.
    assert_eq!(bash(r#"a=(x y z); unset a[1]; printf '%s' "${a[@]}""#), "xz");
    assert_eq!(bash(r#"a=(x y z); unset a[1]; printf '%s' "${!a[@]}|${#a[@]}""#), "02|2");
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
    assert_eq!(bash(r#"a=([2]=two [5]=five); printf '%s' "${a[*]}""#), "two five");
    assert_eq!(bash(r#"a=([2]=two [5]=five); printf '%s' "${!a[*]}""#), "2 5");
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
    assert_eq!(bash(r#"v=hello_world; printf '%s' "${v^^[hw]}""#), "Hello_World");
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
    assert!(!out.status.success(), "--zsh: ${{v@U}} must be bad substitution");
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
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
    assert_eq!(bash("shopt -s nocasematch; [[ HELLO == hello ]] && echo ci"), "ci");
    assert_eq!(bash("shopt -s nocasematch; [[ Hello == h* ]] && echo m"), "m");
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
    assert_eq!(zsh(r#"read -n foo <<< "hi there"; echo "[$foo]""#), "[hi there]");
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
        bash(r#"declare -A m=([a]=1 [b]=2 [c]=3); s=0; for k in "${!m[@]}"; do s=$((s+m[$k])); done; echo $s"#),
        "6"
    );
    // The key SET is correct (sorted for determinism).
    assert_eq!(
        bash(r#"declare -A m=([x]=1 [y]=2 [z]=3); for k in "${!m[@]}"; do echo "$k"; done | sort | tr -d '\n'"#),
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
    assert_eq!(bash("declare -rx e=hi; declare -p e"), r#"declare -rx e="hi""#);
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
    assert_eq!(bash(r#"v="a\"b\$c"; declare -p v"#), r#"declare -- v="a\"b\$c""#);

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
    assert_eq!(bash("shopt -s extglob; [[ abc == @(abc|xyz) ]] && echo m"), "m");
    assert_eq!(bash("shopt -s extglob; [[ aaa == +(a) ]] && echo p"), "p");
    assert_eq!(bash("shopt -s extglob; [[ color == colo?(u)r ]] && echo o"), "o");
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
    assert_eq!(bash(r#"true | false | true; echo "${PIPESTATUS[@]}""#), "0 1 0");
    assert_eq!(bash(r#"true | false | true; echo "${PIPESTATUS[*]}""#), "0 1 0");
    assert_eq!(bash(r#"true | false; echo "${PIPESTATUS[0]}${PIPESTATUS[1]}""#), "01");
    assert_eq!(bash(r#"true | false | true; echo "${#PIPESTATUS[@]}""#), "3");
    // FUNCNAME — call stack, innermost (current) first; nested frames.
    assert_eq!(bash(r#"f() { echo "${FUNCNAME[0]}"; }; f"#), "f");
    assert_eq!(bash(r#"g(){ f(){ echo "${FUNCNAME[@]}"; }; f; }; g"#), "f g");
    // BASH_VERSINFO — 6-element array, first element numeric & >= 4.
    assert_eq!(bash(r#"echo "${#BASH_VERSINFO[@]}""#), "6");
    assert_eq!(bash(r#"[[ ${BASH_VERSINFO[0]} =~ ^[0-9]+$ ]] && echo num"#), "num");
    assert_eq!(bash(r#"[[ ${BASH_VERSINFO[0]} -ge 4 ]] && echo modern"#), "modern");
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
    assert_eq!(zsh(r#"true|false; echo "[${PIPESTATUS[0]}][$BASH_VERSION]""#), "[][]");
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
        let emu = case.ref_emulate.map(|e| format!(" [emulate {e}]")).unwrap_or_default();
        eprintln!(
            "testing {} : zshrs {} vs {}{} (extended={})",
            case.name, case.zshrs_flags.join(" "), refbin, emu, case.extended
        );

        // The portable corpus runs for every case; the extended corpus only
        // for cases whose reference has arrays / [[ / (( )) / brace expansion.
        let corpus = PORTABLE_CORPUS
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

    eprintln!("emulation parity: tested {tested} way(s), {} missing", missing.len());

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
    assert!(tested > 0, "no reference shells available at all — cannot verify parity");
}
