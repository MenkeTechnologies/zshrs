//! Differential parity fuzzer: `zsh -fc <s>` vs `zshrs --zsh -f -c <s>`.
//!
//! Generates thousands of grammar-driven, deterministic-output shell snippets,
//! runs each through both shells, and reports every case where stdout OR exit
//! code diverge. Each case is produced from a per-index seed so any divergence
//! replays exactly: `parity-fuzz --seed <N> --once`.
//!
//! The generator is deliberately biased toward the historically weak areas
//! (parameter-expansion flags, arithmetic precedence, array/slice ops, assoc
//! iteration, string ops). Pure random bytes only produce mutual syntax errors
//! that agree on both shells and teach nothing.
//!
//! Determinism invariant: the generator NEVER emits a construct whose output is
//! nondeterministic for reasons unrelated to parity ($RANDOM, $$, dates,
//! filesystem glob order, unsorted assoc iteration). Assoc iteration is always
//! forced through an ordering flag. This keeps every reported divergence a real
//! parity gap, not a false positive.
//!
//! Build:  cargo build --features parity-fuzz --bin parity-fuzz
//! Run:    ./target/debug/parity-fuzz --count 5000

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

// ---------------------------------------------------------------------------
// Shell locations / invocation (mirrors tests/parity/*.rs contract exactly)
// ---------------------------------------------------------------------------

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
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

struct RunOut {
    stdout: String,
    exit: i32,
    timed_out: bool,
}

/// Spawn `cmd` and wait up to `timeout`, killing it if it overruns.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> RunOut {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            return RunOut {
                stdout: String::new(),
                exit: -999,
                timed_out: false,
            }
        }
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut buf = String::new();
                if let Some(mut out) = child.stdout.take() {
                    use std::io::Read;
                    let _ = out.read_to_string(&mut buf);
                }
                return RunOut {
                    stdout: buf,
                    exit: status.code().unwrap_or(-1),
                    timed_out: false,
                };
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return RunOut {
                        stdout: String::new(),
                        exit: -1,
                        timed_out: true,
                    };
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => {
                return RunOut {
                    stdout: String::new(),
                    exit: -998,
                    timed_out: false,
                }
            }
        }
    }
}

// Glob mode runs every generated pattern from a fixed fixture directory so
// filename generation has a known, deterministic fileset to match. Set once at
// startup; the runners below cd into it. None in the other modes.
static FIXTURE_CWD: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

fn run_zsh(script: &str, timeout: Duration) -> RunOut {
    let mut cmd = Command::new(zsh_path());
    cmd.args(["-f", "-c", script]);
    if let Some(dir) = FIXTURE_CWD.get() {
        cmd.current_dir(dir);
    }
    run_with_timeout(cmd, timeout)
}

fn run_zshrs(script: &str, bin: &Path, timeout: Duration) -> RunOut {
    let mut cmd = Command::new(bin);
    cmd.args(["--zsh", "-f", "-c", script]).env_remove("ZSHRS_CACHE");
    if let Some(dir) = FIXTURE_CWD.get() {
        cmd.current_dir(dir);
    }
    run_with_timeout(cmd, timeout)
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

/// Small, side-effect-free variable environment prepended to every script so
/// expansions have real values to chew on. Fixed names keep generation simple
/// and outputs deterministic.
const PREAMBLE: &str = concat!(
    "s=Hello_World; ",
    "t='a,b,c,d'; ",
    "empty=''; ",
    "n=42; ",
    "neg=-7; ",
    "path=/usr/local/bin/zsh; ",
    "spaces='  x y  '; ",
    "a=(one two three four five); ",
    "b=(two four six eight); ",
    "nums=(3 1 4 1 5 9 2 6); ",
    "lines=$'aa\\nbb\\ncc'; ",
    "typeset -A m; m=(k1 v1 k2 v2 k3 v3); ",
    "ptr=path; ",
    "typeset -F fl=3.5; ",
);

const SCALARS: &[&str] = &["s", "t", "empty", "path", "spaces"];
const INTVARS: &[&str] = &["n", "neg"];
const ARRAYS: &[&str] = &["a", "nums"];

/// Parameter-expansion flag letters that are safe + deterministic to combine.
const PE_FLAGS: &[&str] = &[
    "U", "L", "C", "q", "Q", "o", "O", "n", "u", "w", "W", "#", "V", "P", "e",
];

/// History-style word modifiers applied via `${var:MOD}` / `$var:MOD`.
const MODIFIERS: &[&str] = &["h", "t", "r", "e", "l", "u", "q", "Q", "gs/o/0", "s/l/L", "a"];

fn pick<'a, T>(rng: &mut StdRng, xs: &'a [T]) -> &'a T {
    &xs[rng.gen_range(0..xs.len())]
}

/// A scalar parameter expansion, possibly with flags / modifiers.
fn gen_scalar_pe(rng: &mut StdRng) -> String {
    let v = pick(rng, SCALARS);
    match rng.gen_range(0..12) {
        0 => format!("${{{v}}}"),
        1 => format!("${{#{v}}}"),
        2 => format!("${{{v}:-fallback}}"),
        3 => format!("${{{v}:+set}}"),
        4 => {
            let off = rng.gen_range(0..6);
            let len = rng.gen_range(1..6);
            format!("${{{v}:{off}:{len}}}")
        }
        5 => format!("${{{v}//o/0}}"),
        6 => format!("${{{v}/#H/J}}"),
        7 => format!("${{{v}/%d/D}}"),
        8 => format!("${{{v}:u}}"),
        9 => format!("${{{v}:l}}"),
        10 => {
            // random flag combo
            let k = rng.gen_range(1..=3);
            let mut flags = String::new();
            for _ in 0..k {
                flags.push_str(pick(rng, PE_FLAGS));
            }
            format!("${{({flags}){v}}}")
        }
        _ => format!("${{{v}##*_}}"),
    }
}

/// Left/right padding and split/join flags — dense parse surface.
fn gen_padding(rng: &mut StdRng) -> String {
    let v = pick(rng, SCALARS);
    let w = rng.gen_range(1..10);
    match rng.gen_range(0..6) {
        0 => format!("${{(l:{w}:){v}}}"),
        1 => format!("${{(r:{w}:){v}}}"),
        2 => format!("${{(l:{w}::0:){v}}}"),
        3 => format!("${{(r:{w}::-:){v}}}"),
        4 => format!("${{(l:{w}::x::y:){v}}}"),
        _ => format!("${{(r:{w}::.:){v}}}"),
    }
}

/// String splitting into words via (s), (f), (z), (@).
fn gen_split(rng: &mut StdRng) -> String {
    match rng.gen_range(0..6) {
        0 => "${(s:,:)t}".to_string(),
        1 => "${(s:_:)s}".to_string(),
        2 => "\"${(@s:,:)t}\"".to_string(),
        3 => "${#${(s:,:)t}}".to_string(),
        4 => "${(j:/:)${(s:,:)t}}".to_string(),
        _ => "${(ws:,:)t}".to_string(),
    }
}

/// History-modifier chains on a path-ish scalar (:h:t:r:e:gs...).
fn gen_modchain(rng: &mut StdRng) -> String {
    let base = if rng.gen_bool(0.6) { "path" } else { "s" };
    let k = rng.gen_range(1..=3);
    let mut out = format!("${{{base}");
    for _ in 0..k {
        out.push(':');
        out.push_str(pick(rng, MODIFIERS));
    }
    out.push('}');
    out
}

/// Nested / indirect expansion — subscript flags and $(P).
fn gen_nested(rng: &mut StdRng) -> String {
    match rng.gen_range(0..7) {
        0 => "${a[(i)three]}".to_string(),
        1 => "${a[(I)two]}".to_string(),
        2 => "${a[(r)f*]}".to_string(),
        3 => "${a[(R)f*]}".to_string(),
        4 => "${nums[(r)5]}".to_string(),
        5 => "${(P)ptr}".to_string(),
        _ => "${#${(o)a}}".to_string(),
    }
}

/// An array parameter expansion.
fn gen_array_pe(rng: &mut StdRng) -> String {
    let v = pick(rng, ARRAYS);
    match rng.gen_range(0..11) {
        0 => format!("${{{v}[1]}}"),
        1 => {
            let i = rng.gen_range(1..4);
            let j = rng.gen_range(i..6);
            format!("${{{v}[{i},{j}]}}")
        }
        2 => format!("${{#{v}}}"),
        3 => format!("${{{v}[-1]}}"),
        4 => format!("${{(j:-:){v}}}"),
        5 => format!("${{(o){v}}}"),
        6 => format!("${{(O){v}}}"),
        7 => format!("${{(On){v}}}"),
        8 => format!("${{(on){v}}}"),
        9 => format!("${{{v}[(r)two]}}"),
        _ => format!("${{(u){v}}}"),
    }
}

/// An assoc expansion (always ordered so iteration is deterministic).
fn gen_assoc_pe(rng: &mut StdRng) -> String {
    match rng.gen_range(0..5) {
        0 => "${(ko)m}".to_string(),
        1 => "${(vo)m}".to_string(),
        2 => "${(kvo)m}".to_string(),
        3 => "${m[k2]}".to_string(),
        _ => "${(o)${(k)m}}".to_string(),
    }
}

/// An arithmetic expression printed via $(( )). Recursive with a depth cap.
fn gen_arith(rng: &mut StdRng, depth: u32) -> String {
    if depth == 0 || rng.gen_bool(0.35) {
        return match rng.gen_range(0..7) {
            0 => rng.gen_range(-20..40).to_string(),
            1 => pick(rng, INTVARS).to_string(),
            2 => pick(rng, INTVARS).to_string(),
            3 => format!("16#{:x}", rng.gen_range(1..255)),      // hex base
            4 => format!("2#{:b}", rng.gen_range(1..16)),        // binary base
            5 => format!("0x{:x}", rng.gen_range(1..255)),       // C-style hex
            _ => rng.gen_range(1..12).to_string(),
        };
    }
    let l = gen_arith(rng, depth - 1);
    let r = gen_arith(rng, depth - 1);
    let op = pick(
        rng,
        &["+", "-", "*", "/", "%", "**", "<<", ">>", "&", "|", "^", "<", ">", "==", "!=", "&&", "||"],
    );
    // Guard divide/mod against a zero right operand: force it nonzero via `| 1`.
    if *op == "/" || *op == "%" {
        return format!("({l}) {op} ((({r})) | 1)");
    }
    match rng.gen_range(0..3) {
        0 => format!("({l}) {op} ({r})"),
        1 => format!(
            "{l} {op} {r} {} {}",
            pick(rng, &["+", "-", "*"]),
            rng.gen_range(1..5)
        ),
        // ternary — exercises precedence around ?:
        _ => format!("({l}) ? ({r}) : {}", rng.gen_range(0..9)),
    }
}

/// Array set operations and pattern filters — deterministic surface that the
/// scalar/array/assoc generators don't reach: `:|` (difference), `:*`
/// (intersection), `:#pat` / `(M):#pat` (element filter). Uses PREAMBLE arrays
/// `a` and `b` (which share `two`/`four`) so the results are non-trivial.
fn gen_setops(rng: &mut StdRng) -> String {
    match rng.gen_range(0..8) {
        0 => "${a:|b}".to_string(),              // elements of a not in b
        1 => "${a:*b}".to_string(),              // intersection of a and b
        2 => "\"${(@)a:|b}\"".to_string(),
        3 => "${a:#t*}".to_string(),             // drop elements matching t*
        4 => "${(M)a:#*e}".to_string(),          // keep elements matching *e
        5 => "${nums:#[13]}".to_string(),        // drop bare 1 / 3
        6 => "\"${(j:,:)${a:|b}}\"".to_string(), // join the difference
        _ => "${#${a:#*e*}}".to_string(),        // count after filter
    }
}

/// `(f)` newline splitting and the quote-style flags (`qq`/`qqq`/`qqqq`/`q-`).
/// `lines` is a 3-line scalar; `spaces` has leading/trailing/embedded spaces.
fn gen_quoteflags(rng: &mut StdRng) -> String {
    match rng.gen_range(0..8) {
        0 => "\"${(f)lines}\"".to_string(),
        1 => "${#${(f)lines}}".to_string(),          // element count after split
        2 => "\"${(j:|:)${(f)lines}}\"".to_string(), // split then re-join
        3 => "\"${(qq)spaces}\"".to_string(),        // single-quote style
        4 => "\"${(q-)spaces}\"".to_string(),        // minimal quoting
        5 => "\"${(qqqq)s}\"".to_string(),           // $'...' style
        6 => "\"${(Ff)lines}\"".to_string(),         // join-with-newline of split
        _ => "\"${(@f)lines}\"".to_string(),
    }
}

/// Generate the raw expression list for a seed (before script assembly).
fn gen_parts(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let n = rng.gen_range(1..=3);
    let mut parts: Vec<String> = Vec::with_capacity(n);
    for _ in 0..n {
        let expr = match rng.gen_range(0..12) {
            0 => gen_scalar_pe(&mut rng),
            1 => gen_array_pe(&mut rng),
            2 => gen_assoc_pe(&mut rng),
            3 => format!("$(( {} ))", gen_arith(&mut rng, 3)),
            4 => gen_padding(&mut rng),
            5 => gen_split(&mut rng),
            6 => gen_modchain(&mut rng),
            7 => gen_nested(&mut rng),
            8 => gen_setops(&mut rng),
            9 => gen_quoteflags(&mut rng),
            10 => gen_scalar_pe(&mut rng),
            _ => gen_array_pe(&mut rng),
        };
        parts.push(expr);
    }
    parts
}

// ---------------------------------------------------------------------------
// Stateful program generator
//
// A "program" is a Vec<String> of statements executed in order. State-mutating
// statements (setopt, typeset attributes, IFS, positional params, array/assoc
// mutation, function scope) build up context; observation statements probe it.
// This is what surfaces context-dependent parity gaps: a construct that behaves
// correctly from a clean slate but diverges once some option/attribute is set.
//
// All values are alnum/space only — never a glob metachar — so that even with
// GLOB_SUBST / SH_WORD_SPLIT / unquoted probes the output stays deterministic
// (no filesystem globbing) across both shells.
// ---------------------------------------------------------------------------

/// Output-affecting options that are deterministic without touching the
/// filesystem. Each flips real expansion / assignment / word-split semantics.
const OPTIONS: &[&str] = &[
    "KSH_ARRAYS",
    "SH_WORD_SPLIT",
    "RC_EXPAND_PARAM",
    "KSH_TYPESET",
    "TYPESET_SILENT",
    "WARN_CREATE_GLOBAL",
    "MAGIC_EQUAL_SUBST",
    "KSH_ZERO_SUBSCRIPT",
    "EXTENDED_GLOB",
    "NUMERIC_GLOB_SORT",
    "NO_UNSET",
    "IGNORE_BRACES",
    "BRACE_CCL",
    "MULTIBYTE",
    "CASE_GLOB",
];

const WORDS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "foo", "bar", "x1", "x2", "y3", "aa", "bb", "7", "42",
];

/// A state-mutating statement.
fn gen_mutation(rng: &mut StdRng) -> String {
    match rng.gen_range(0..17) {
        0 => {
            let neg = if rng.gen_bool(0.35) { "un" } else { "" };
            format!("{neg}setopt {}", pick(rng, OPTIONS))
        }
        // Quote the arithmetic RHS: unquoted it has spaces/parens and parses
        // as an array assignment + glob, which is malformed input (a generator
        // artifact), not a parity gap. Quoted, `-i` evaluates it as arithmetic
        // — the genuine test. The unquoted assignment-word glob behaviour is
        // covered separately by the function arm's `$1*2`.
        1 => format!("typeset -i iv=\"{}\"", gen_arith(rng, 2)),
        2 => format!("typeset -F 3 fv=$(( {} ))", gen_arith(rng, 2)),
        3 => format!("typeset -Z {} zv={}", rng.gen_range(3..7), rng.gen_range(1..999)),
        4 => format!("typeset -{} uv={}", pick(rng, &["u", "l"]), pick(rng, WORDS)),
        5 => {
            let (a, b, c) = (pick(rng, WORDS), pick(rng, WORDS), pick(rng, WORDS));
            format!("typeset -aU arr; arr=({a} {a} {b} {b} {c})")
        }
        6 => format!("IFS={}", pick(rng, &[":", ",", "-", "|", "."])),
        7 => {
            let k = rng.gen_range(1..4);
            let ws: Vec<&str> = (0..k).map(|_| *pick(rng, WORDS)).collect();
            format!("set -- {}", ws.join(" "))
        }
        8 => format!("arr+=({})", pick(rng, WORDS)),
        9 => format!("arr[{}]={}", rng.gen_range(1..5), pick(rng, WORDS)),
        10 => format!("as[{}]={}", pick(rng, &["k1", "k2", "k3", "kx"]), pick(rng, WORDS)),
        11 => format!("unset {}", pick(rng, &["v", "iv", "arr", "uv", "zv"])),
        12 => format!("typeset -r rv={}", rng.gen_range(1..99)),
        13 => format!("v+={}", pick(rng, WORDS)),
        14 => format!(
            "f() {{ typeset -i lv=$1*2; print -r -- $lv }}; f {}",
            rng.gen_range(1..20)
        ),
        15 => format!("(( c {} ))", pick(rng, &["++", "--", "+= 3", "*= 2"])),
        _ => format!("v={}", pick(rng, WORDS)),
    }
}

/// A conditional / pattern-match probe: `[[ ]]` tests and `case`, printing the
/// branch taken or `$?` so the output is deterministic. Exercises the pattern
/// matcher (glob patterns, character classes, `<a-b>` ranges, alternation) and
/// the numeric/string test operators — surface the PE/arith generators miss.
/// The extended-glob variants (`(abc)#`) only match when a prior mutation ran
/// `setopt EXTENDED_GLOB`, so they probe the option-dependent matcher path too.
fn gen_cond(rng: &mut StdRng) -> String {
    match rng.gen_range(0..14) {
        0 => "[[ $v == a* ]]; print -r -- $?".to_string(),
        1 => "[[ $v == *b* ]]; print -r -- $?".to_string(),
        2 => "[[ abc == a?c ]]; print -r -- $?".to_string(),
        3 => "[[ $v != x* ]]; print -r -- $?".to_string(),
        4 => "[[ -z ${arr[99]} ]]; print -r -- $?".to_string(),
        5 => format!(
            "[[ {} -gt {} ]]; print -r -- $?",
            rng.gen_range(0..10),
            rng.gen_range(0..10)
        ),
        6 => "case $v in (a*) print A;; (*) print B;; esac".to_string(),
        7 => "case ${arr[1]} in (x[0-9]) print X;; (*) print O;; esac".to_string(),
        8 => "[[ foobar == f*r && $v == a* ]]; print -r -- $?".to_string(),
        9 => "[[ hello == (hel*|wor*) ]]; print -r -- $?".to_string(),
        10 => "[[ 12345 == <1-99999> ]]; print -r -- $?".to_string(),
        11 => "[[ file.txt == *.(txt|md) ]]; print -r -- $?".to_string(),
        12 => "[[ aXb == a[[:upper:]]b ]]; print -r -- $?".to_string(),
        _ => "[[ abcabc == (abc)# ]]; print -r -- $?".to_string(),
    }
}

/// An observation statement — always emits to stdout, deterministically.
/// `${(t)var}` is weighted in because it reports a parameter's full type +
/// attribute set (e.g. `integer-local`, `array-unique`, `scalar-readonly`),
/// which is the most direct probe of whether typeset state was modelled right.
fn gen_observation(rng: &mut StdRng) -> String {
    // 1-in-5: conditional / pattern-match probe (state-sensitive matcher path).
    if rng.gen_bool(0.20) {
        return gen_cond(rng);
    }
    // 1-in-4: unquoted list probe to exercise word-splitting under options.
    if rng.gen_bool(0.25) {
        let u = pick(rng, &["$v", "${arr[@]}", "$*", "$@", "$uv"]);
        return format!("print -rl -- {u}");
    }
    let probe = match rng.gen_range(0..18) {
        0 => "\"$v\"",
        1 => "\"${(t)v}\"",
        2 => "\"${(t)arr}\"",
        3 => "\"${(t)as}\"",
        4 => "\"${#arr}\"",
        5 => "\"${arr[1]}\"",
        6 => "\"${arr[@]}\"",
        7 => "\"$#\"",
        8 => "\"$*\"",
        9 => "\"$@\"",
        10 => "\"${(t)iv}\"",
        11 => "\"$iv\"",
        12 => "\"$(( iv + 1 ))\"",
        13 => "\"${(ko)as}\"",
        14 => "\"${as[k1]}\"",
        15 => "\"${v:-UNSET}\"",
        16 => "\"$c\"",
        _ => "\"$zv\"",
    };
    format!("print -r -- {probe}")
}

/// A NESTED-SCOPE statement: functions with `local`/`typeset -g`, subshells,
/// nested loops that accumulate state, and anonymous functions. These probe
/// the hardest state-dependent behaviour — scope save/restore, global leakage,
/// and subshell isolation — where a construct's output depends on which frame
/// created/modified a parameter. Every form prints deterministically so the
/// differential comparison is exact.
fn gen_nested_scope(rng: &mut StdRng) -> String {
    match rng.gen_range(0..16) {
        // local shadows an outer scalar, restored on function return
        0 => "sc=outer; f() { local sc=inner; print -r -- $sc }; f; print -r -- $sc".to_string(),
        // nested functions: inner local must not leak to the middle frame
        1 => "g() { local x=2; print -r -- $x }; f() { local x=1; g; print -r -- $x }; f".to_string(),
        // typeset -g from inside a function creates/updates a GLOBAL
        2 => "unset gv; f() { typeset -g gv=fromfunc }; f; print -r -- ${gv-UNSET}".to_string(),
        // local array shadows outer array
        3 => "ar=(o1 o2); f() { local ar=(i1 i2 i3); print -r -- ${#ar} }; f; print -r -- ${#ar}"
            .to_string(),
        // subshell mutation is isolated from the parent
        4 => "arr=(x1 x2 y3); ( arr+=(z); print -r -- ${#arr} ); print -r -- ${#arr}".to_string(),
        // nested for-loops accumulating into an array
        5 => "acc=(); for a in p q; do for b in 1 2; do acc+=($a$b); done; done; print -r -- $acc"
            .to_string(),
        // anonymous function scope — `local` inside does not leak
        6 => "v=keep; () { local v=temp; print -r -- $v }; print -r -- $v".to_string(),
        // while-loop with arithmetic state
        7 => "i=0; s=0; while (( i < 4 )); do (( s += i )); (( i++ )); done; print -r -- $s"
            .to_string(),
        // function-local integer attribute
        8 => "f() { integer n=6*7; print -r -- $n }; f; print -r -- ${n-UNSET}".to_string(),
        // nested command substitution reading loop-built state
        9 => "parts=(a b c); joined=${(j:-:)parts}; print -r -- $joined".to_string(),
        // local -A assoc inside function
        10 => "f() { local -A m2=(a 1 b 2); print -r -- ${(kvo)m2} }; f".to_string(),
        // case inside a function driven by a parameter
        11 => "classify() { case $1 in ([0-9]) print digit;; ([a-z]) print lower;; (*) print other;; esac }; classify 5; classify q; classify %"
            .to_string(),
        // recursive countdown via a function
        12 => "cnt() { (( $1 <= 0 )) && return; print -rn -- $1; cnt $(( $1 - 1 )) }; cnt 3; print"
            .to_string(),
        // nested subshell with its own option scope: extendedglob set inside
        // the subshell must not leak to the parent's match after it exits
        13 => "( setopt extendedglob; [[ abcabc == (abc)# ]]; print -r -- in=$? ); [[ abcabc == (abc)# ]]; print -r -- out=$?"
            .to_string(),
        // append inside a for-loop over an assoc's ordered keys
        14 => "typeset -A h=(k1 1 k2 2 k3 3); out=(); for k in ${(ok)h}; do out+=($k=$h[$k]); done; print -r -- $out"
            .to_string(),
        // param scope: function modifies a global array element by index
        _ => "arr=(a b c); f() { arr[2]=MODIFIED }; f; print -r -- $arr".to_string(),
    }
}

/// Base state every stateful program starts from. Kept as separate statements
/// so minimization can drop whichever anchors a divergence doesn't depend on.
fn base_state() -> Vec<String> {
    vec![
        "v='a b c'".to_string(),
        "arr=(x1 x2 y3)".to_string(),
        "typeset -A as; as=(k1 v1 k2 v2)".to_string(),
        "integer c=0".to_string(),
    ]
}

/// Build a stateful program: base state, then mutation/observation steps.
/// A fraction of steps are NESTED-SCOPE constructs (functions / subshells /
/// nested loops) so state-dependent gaps that only manifest across a scope
/// boundary get exercised — the "nested state" the fuzzer exists to find.
fn gen_program(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = base_state();
    let steps = rng.gen_range(4..=12);
    for _ in 0..steps {
        // 1-in-4 step is a self-contained nested-scope probe.
        if rng.gen_bool(0.25) {
            stmts.push(gen_nested_scope(&mut rng));
            continue;
        }
        if rng.gen_bool(0.6) {
            stmts.push(gen_mutation(&mut rng));
        }
        stmts.push(gen_observation(&mut rng));
    }
    stmts
}

/// Flat-expression program (the `--mode expr` generator), expressed as a
/// statement list so it shares the same runner + minimizer as stateful mode.
fn expr_program(seed: u64) -> Vec<String> {
    let mut stmts = vec![PREAMBLE.trim_end().to_string()];
    for p in gen_parts(seed) {
        stmts.push(format!("print -r -- \"{p}\""));
    }
    stmts
}

/// Join a statement list into a runnable script.
fn build_program(stmts: &[String]) -> String {
    stmts.join("\n")
}

/// True iff zsh and zshrs disagree on stdout or exit for `script`. A zsh-side
/// timeout is treated as non-divergent (pathological case, not a parity gap).
fn diverges(script: &str, bin: &Path, timeout: Duration) -> bool {
    let z = run_zsh(script, timeout);
    if z.timed_out {
        return false;
    }
    let r = run_zshrs(script, bin, timeout);
    // Infra failures are NOT parity gaps: -999 = spawn failed (binary missing
    // — e.g. a concurrent `cargo build` deleted it mid-run), -998 = wait error,
    // timed_out = pathological. Treating any of these as a divergence floods
    // the report with false positives (every probe "diverges" because zshrs
    // produced no output). Skip them.
    if r.exit == -999 || r.exit == -998 || r.timed_out || z.exit == -999 || z.exit == -998 {
        return false;
    }
    z.stdout != r.stdout || z.exit != r.exit
}

/// Delta-debug a diverging statement list to a locally-minimal one: repeatedly
/// drop any single statement whose removal preserves the divergence, to a
/// fixpoint. Statements share state, so this (not per-statement independence)
/// is the only correct minimizer for stateful programs.
fn minimize(stmts: Vec<String>, bin: &Path, timeout: Duration) -> Vec<String> {
    let mut cur = stmts;
    loop {
        let mut removed = false;
        let mut i = 0;
        while i < cur.len() {
            let mut cand = cur.clone();
            cand.remove(i);
            if !cand.is_empty() && diverges(&build_program(&cand), bin, timeout) {
                cur = cand; // keep index i — it now points at the next statement
                removed = true;
            } else {
                i += 1;
            }
        }
        if !removed {
            break;
        }
    }
    cur
}

// ---------------------------------------------------------------------------
// Glob-qualifier generator
//
// Every pattern runs from a fixed fixture directory (setup_glob_fixture) with a
// known fileset: distinct sizes and mtimes so ordering qualifiers ((oL) size,
// (om) mtime, (on) name) are deterministic across both shells. Only stable
// order keys are used — never (oa)/(oc) (atime/ctime drift) or a bare no-sort.
// ---------------------------------------------------------------------------

const GLOB_BASE: &[&str] = &[
    "*", "*.txt", "*.log", "*.md", "[a-d]*", "?", "??*", "<->", "**/*", "*.??",
    "[[:alpha:]]*", "d*", "*.*", "[^.]*",
];

const GLOB_QUAL: &[&str] = &[
    "", "(.)", "(/)", "(@)", "(*)", "(.N)", "(on)", "(On)", "(oL)", "(OL)", "(om)", "(Om)",
    "(.on)", "(/on)", "(N)", "(D)", "(.D)", "(^/)", "(r)", "(x)", "(w)", "([1])", "([1,2])",
    "(om[1])", "(.oL[1,2])", "(*N)", "(@N)", "(.^@)", "(-.)",
];

/// One or two glob-pattern print statements, prefixed with `setopt extendedglob`
/// so `<->` numeric ranges and `^` negation parse.
fn gen_glob(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let n = rng.gen_range(1..=2);
    let mut stmts = vec!["setopt extendedglob".to_string()];
    for _ in 0..n {
        let base = pick(&mut rng, GLOB_BASE);
        let qual = pick(&mut rng, GLOB_QUAL);
        // print -rl -- one match per line; unquoted so filename generation runs.
        stmts.push(format!("print -rl -- {base}{qual}"));
    }
    stmts
}

// ---------------------------------------------------------------------------
// printf generator
//
// Deterministic, self-contained (no filesystem, no time): exercises format
// specifiers, flags, width/precision (including `*` from args), the zsh-
// specific %b/%q conversions, and argument recycling (a format is reused when
// more args than specifiers are supplied).
// ---------------------------------------------------------------------------

const PF_CONV: &[&str] = &[
    "d", "i", "o", "u", "x", "X", "e", "E", "f", "g", "G", "s", "c", "b", "q", "%",
];
const PF_FLAGS: &[&str] = &["", "-", "+", " ", "#", "0", "-0", "+ ", "0#", "-#"];
const PF_STR_ARGS: &[&str] = &[
    "abc", "hello", "", "12", "3.14", "-5", "a b", "x\\\\ty", "it_s", "star", "%d", "cafe",
    "0x1f", "007", "1e3",
];

/// One printf statement: a format string of 1-3 specifiers (with optional
/// literal separators) plus 0-5 arguments.
fn gen_printf(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let nspec = rng.gen_range(1..=3);
    let mut fmt = String::new();
    for _ in 0..nspec {
        if rng.gen_bool(0.4) {
            fmt.push_str(pick(&mut rng, &["x", "-", "[", "] ", ":", "->"]));
        }
        fmt.push('%');
        fmt.push_str(pick(&mut rng, PF_FLAGS));
        // width: literal or `*` (consumes an arg)
        if rng.gen_bool(0.5) {
            if rng.gen_bool(0.25) {
                fmt.push('*');
            } else {
                fmt.push_str(&rng.gen_range(0..12).to_string());
            }
        }
        // precision
        if rng.gen_bool(0.4) {
            fmt.push('.');
            if rng.gen_bool(0.25) {
                fmt.push('*');
            } else {
                fmt.push_str(&rng.gen_range(0..8).to_string());
            }
        }
        fmt.push_str(pick(&mut rng, PF_CONV));
    }
    let nargs = rng.gen_range(0..=5);
    let mut args: Vec<String> = Vec::with_capacity(nargs);
    for _ in 0..nargs {
        if rng.gen_bool(0.5) {
            args.push(rng.gen_range(-20..40).to_string());
        } else {
            // single-quote string args; the pool has no quotes to escape.
            args.push(format!("'{}'", pick(&mut rng, PF_STR_ARGS)));
        }
    }
    // A trailing `|` marks end-of-output so trailing-space diffs are visible.
    let fmt_q = fmt.replace('\'', "'\\''");
    vec![format!("printf '{fmt_q}|\\n' {}", args.join(" "))]
}

// ---------------------------------------------------------------------------
// here-doc / here-string generator
//
// Exercises delimiter quoting (`EOF` vs `'EOF'` vs `"EOF"` — controls whether
// the body is expanded), `<<` vs `<<-` (leading-tab strip), and parameter /
// command / arithmetic expansion inside the body. Deterministic: the body only
// references the fixed PREAMBLE vars and side-effect-free command subs.
// ---------------------------------------------------------------------------

/// Body fragments that probe expansion inside a here-document.
const HD_FRAGS: &[&str] = &[
    "plain text",
    "$s",
    "${s}",
    "${s:u}",
    "${(U)s}",
    "${nums}",
    "${(j:-:)nums}",
    "$(echo cmd)",
    "$((3 + 4))",
    "${undef:-def}",
    "tab\tinside",
    "back\\slash",
    "dollar \\$s",
    "quote \\` tick",
    "brace ${a[2]}",
    "$s and $n",
];

fn gen_heredoc(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    // A heredoc-specific preamble: the shared PREAMBLE sets `path=…`, which is
    // a special array tied to PATH in zsh — a scalar assignment clobbers PATH
    // and `cat` vanishes (exit 127). Use the same vars minus `path`/`ptr`, so
    // the external `cat` that reads the here-doc still resolves.
    let preamble = "s=Hello_World; t='a,b,c,d'; empty=''; n=42; neg=-7; \
                    spaces='  x y  '; a=(one two three four five); \
                    nums=(3 1 4 1 5 9 2 6); typeset -A m; m=(k1 v1 k2 v2 k3 v3);"
        .to_string();

    // 1-in-4: a here-STRING (`<<< word`) instead of a here-doc.
    if rng.gen_bool(0.25) {
        let frag = pick(&mut rng, HD_FRAGS);
        let stmt = match rng.gen_range(0..3) {
            0 => format!("cat <<< \"{frag}\""),
            1 => format!("cat <<< '{frag}'"),
            _ => format!("cat <<< {}", frag.split_whitespace().next().unwrap_or(frag)),
        };
        return vec![preamble, stmt];
    }

    // here-doc: choose delimiter quoting and the `<<`/`<<-` operator.
    let (op, indent) = if rng.gen_bool(0.4) {
        ("<<-", "\t") // `<<-` strips leading TABS from body + terminator
    } else {
        ("<<", "")
    };
    let delim = match rng.gen_range(0..3) {
        0 => "EOF".to_string(),      // unquoted → body is expanded
        1 => "'EOF'".to_string(),    // single-quoted → body is literal
        _ => "\"EOF\"".to_string(),  // double-quoted → body is literal
    };
    let nlines = rng.gen_range(1..=3);
    let mut lines = String::new();
    for _ in 0..nlines {
        let frag = pick(&mut rng, HD_FRAGS);
        lines.push_str(&format!("{indent}{frag}\n"));
    }
    // The closing delimiter is the bare word (EOF), tab-indented only for `<<-`.
    vec![preamble, format!("cat {op}{delim}\n{lines}{indent}EOF")]
}

/// Create (idempotently) the glob fixture directory and return its path. Files
/// have deliberately distinct sizes and staggered mtimes so size/mtime ordering
/// qualifiers produce a single deterministic order.
fn setup_glob_fixture() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("parity-fuzz")
        .join("glob-fixture");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::create_dir_all(dir.join("dir1")).unwrap();
    std::fs::create_dir_all(dir.join("dir2")).unwrap();

    // (name, size, mtime-offset-seconds). Sizes all distinct; mtimes all distinct.
    let files: &[(&str, usize, i64)] = &[
        ("a.txt", 5, 100),
        ("bb.log", 50, 300),
        ("ccc.txt", 200, 200),
        ("d.md", 1, 400),
        (".hidden", 2, 500),
        ("empty", 0, 600),
        ("123", 7, 700),
        ("45", 8, 800),
        ("dir1/nested.txt", 3, 900),
    ];
    let base_epoch: i64 = 1_600_000_000; // fixed anchor — no wall-clock reads
    for (name, size, off) in files {
        let p = dir.join(name);
        std::fs::write(&p, "x".repeat(*size)).unwrap();
        set_mtime(&p, base_epoch + off);
    }
    // Executable file for the (*) qualifier.
    {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("exec.sh");
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        set_mtime(&p, base_epoch + 50);
    }
    // Symlink for the (@) qualifier. Deliberately DANGLING with a target-name
    // length (42) that collides with no file size above: a valid symlink stats
    // to its target's size and would tie with that file, and equal sort keys
    // are qsort-UNSTABLE in zsh (undefined order) — a false positive, not a
    // gap. A dangling link uses lstat (size = target-string length), giving it
    // a unique, deterministic size so (oL)/(OL) ordering stays well-defined.
    let link = dir.join("link");
    let _ = std::fs::remove_file(&link);
    let dangling_target = "x".repeat(42);
    let _ = std::os::unix::fs::symlink(&dangling_target, &link);
    // Stagger directory mtimes too.
    set_mtime(&dir.join("dir1"), base_epoch + 1000);
    set_mtime(&dir.join("dir2"), base_epoch + 1100);
    dir
}

/// Set both atime and mtime of `path` to a fixed epoch second.
fn set_mtime(path: &Path, secs: i64) {
    let t = libc::timeval {
        tv_sec: secs as libc::time_t,
        tv_usec: 0,
    };
    let times = [t, t];
    if let Ok(c) = std::ffi::CString::new(path.to_string_lossy().as_bytes()) {
        unsafe {
            libc::utimes(c.as_ptr(), times.as_ptr());
        }
    }
}

// ---------------------------------------------------------------------------
// Main driver
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Stateful,
    Expr,
    Glob,
    Printf,
    Heredoc,
}

struct Args {
    count: u64,
    base_seed: u64,
    once: bool,
    timeout_ms: u64,
    out_path: PathBuf,
    max_report: usize,
    jobs: usize,
    mode: Mode,
    verify: usize,
    baseline: Option<PathBuf>,
}

/// Generate the statement list for a seed in the selected mode.
fn gen_case(seed: u64, mode: Mode) -> Vec<String> {
    match mode {
        Mode::Stateful => gen_program(seed),
        Mode::Expr => expr_program(seed),
        Mode::Glob => gen_glob(seed),
        Mode::Printf => gen_printf(seed),
        Mode::Heredoc => gen_heredoc(seed),
    }
}

fn parse_args() -> Args {
    let mut count = 2000u64;
    let mut base_seed = 1u64;
    let mut once = false;
    let mut timeout_ms = 5000u64;
    let mut max_report = 200usize;
    let mut mode = Mode::Stateful;
    let mut verify = 1usize;
    let mut baseline: Option<PathBuf> = None;
    let mut jobs = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    // Default under target/ (gitignored) so a regenerable fuzz artifact never
    // pollutes the curated tests/parity_corpus/ directory.
    let mut out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("parity-fuzz")
        .join("divergences.txt");

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--count" | "-c" => {
                i += 1;
                count = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(count);
            }
            "--seed" | "-s" => {
                i += 1;
                base_seed = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(base_seed);
            }
            "--once" => once = true,
            "--timeout-ms" => {
                i += 1;
                timeout_ms = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(timeout_ms);
            }
            "--out" | "-o" => {
                i += 1;
                if let Some(p) = argv.get(i) {
                    out_path = PathBuf::from(p);
                }
            }
            "--max-report" => {
                i += 1;
                max_report = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(max_report);
            }
            "--jobs" | "-j" => {
                i += 1;
                jobs = argv
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .filter(|&j| j >= 1)
                    .unwrap_or(jobs);
            }
            "--mode" | "-m" => {
                i += 1;
                match argv.get(i).map(|s| s.as_str()) {
                    Some("expr") => mode = Mode::Expr,
                    Some("stateful") => mode = Mode::Stateful,
                    Some("glob") => mode = Mode::Glob,
                    Some("printf") => mode = Mode::Printf,
                    Some("heredoc") => mode = Mode::Heredoc,
                    _ => {}
                }
            }
            "--expr" => mode = Mode::Expr,
            "--glob" => mode = Mode::Glob,
            "--printf" => mode = Mode::Printf,
            "--heredoc" => mode = Mode::Heredoc,
            "--verify" => {
                i += 1;
                verify = argv
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .filter(|&k| k >= 1)
                    .unwrap_or(verify);
            }
            "--baseline" => {
                i += 1;
                baseline = argv.get(i).map(PathBuf::from);
            }
            "--help" | "-h" => {
                eprintln!(
                    "parity-fuzz — differential zsh/zshrs parity fuzzer\n\
                     \n\
                     --count N        number of cases (default 2000)\n\
                     --seed N         base seed; case i uses seed+i (default 1)\n\
                     --mode M         'stateful' (default), 'expr', 'glob', 'printf', or 'heredoc'\n\
                     --once           run a single case (seed) and print both outputs\n\
                     --timeout-ms N   per-shell wall-clock timeout (default 5000)\n\
                     --out PATH       divergence corpus file\n\
                     --max-report N   stop after N divergences (default 200)\n\
                     --jobs N         parallel workers (default = CPU count)\n\
                     --verify K       require K consecutive divergences to report\n\
                                      a case (default 1; use 3 on CI to reject\n\
                                      load-contention flakiness)\n\
                     --baseline FILE  allowlist of known-gap signatures; only a\n\
                                      NEW divergence (not in FILE) fails the run\n\
                                      (exit 1). Prints new-signature lines to add.\n\
                     \n\
                     stateful mode builds a sequence of setopt/typeset/IFS/scope\n\
                     mutations interleaved with probes; glob mode runs generated\n\
                     glob-qualifier patterns from a fixed fixture directory. Each\n\
                     divergence is delta-debugged to a minimal reproducer."
                );
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }
    Args {
        count,
        base_seed,
        once,
        timeout_ms,
        out_path,
        max_report,
        jobs,
        mode,
        verify,
        baseline,
    }
}

/// Normalize a minimal reproducer program to a stable gap-class signature:
/// mask numeric literals, base numbers, hex, and the fixed vocabulary words so
/// that many instances of the same gap collapse to one signature. Used by the
/// --baseline allowlist so known gaps don't fail CI but new ones do.
fn signature(program: &str) -> String {
    // Take the last non-empty line (the culprit probe), drop the preamble.
    let body = program
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .next_back()
        .unwrap_or("")
        .to_string();
    let mut s = body;
    // Order matters: hex/base before bare digits.
    for (pat, rep) in [
        (r"0x[0-9a-fA-F]+", "HEX"),
        (r"\b[0-9]+#[0-9a-zA-Z]+", "BASE"),
        (r"\b[0-9]+\b", "N"),
    ] {
        s = regex_lite_replace(&s, pat, rep);
    }
    for w in [
        "Hello_World", "alpha", "beta", "gamma", "delta", "foo", "bar", "x1", "x2", "y3", "aa",
        "bb", "one", "two", "three", "four", "five", "k1", "k2", "k3", "v1", "v2", "v3",
    ] {
        s = s.replace(w, "W");
    }
    s
}

/// Tiny regex replace via the std-free `regex`-like fallback: the binary
/// already depends on `regex`, so use it.
fn regex_lite_replace(s: &str, pat: &str, rep: &str) -> String {
    match regex::Regex::new(pat) {
        Ok(re) => re.replace_all(s, rep).into_owned(),
        Err(_) => s.to_string(),
    }
}

fn main() {
    let args = parse_args();
    let bin = zshrs_bin();
    let timeout = Duration::from_millis(args.timeout_ms);

    if !bin.exists() {
        eprintln!("zshrs binary not found at {}; run `cargo build` first", bin.display());
        std::process::exit(2);
    }

    // Glob mode needs a fixture directory that every generated pattern globs.
    if args.mode == Mode::Glob {
        let dir = setup_glob_fixture();
        FIXTURE_CWD.set(dir).ok();
    }

    // --once: replay a single seed, minimize if it diverges, dump both sides.
    if args.once {
        let stmts = gen_case(args.base_seed, args.mode);
        let script = build_program(&stmts);
        let z = run_zsh(&script, timeout);
        let r = run_zshrs(&script, &bin, timeout);
        let diverged = !z.timed_out && (z.stdout != r.stdout || z.exit != r.exit);
        println!("seed   : {}", args.base_seed);
        println!(
            "mode   : {}",
            match args.mode {
                Mode::Stateful => "stateful",
                Mode::Expr => "expr",
                Mode::Glob => "glob",
                Mode::Printf => "printf",
                Mode::Heredoc => "heredoc",
            }
        );
        let (show, z, r) = if diverged && stmts.len() > 1 {
            let m = minimize(stmts, &bin, timeout);
            let ms = build_program(&m);
            let mz = run_zsh(&ms, timeout);
            let mr = run_zshrs(&ms, &bin, timeout);
            (ms, mz, mr)
        } else {
            (script, z, r)
        };
        println!("program:\n  {}", show.replace('\n', "\n  "));
        println!("--- zsh   exit={} timeout={} ---", z.exit, z.timed_out);
        print!("{}", z.stdout);
        println!("--- zshrs exit={} timeout={} ---", r.exit, r.timed_out);
        print!("{}", r.stdout);
        println!("--- {} ---", if diverged { "DIVERGE" } else { "match" });
        std::process::exit(if diverged { 1 } else { 0 });
    }

    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Mutex;

    // Shared work counter (next seed offset to claim), collected divergences,
    // and a stop flag tripped when --max-report is reached.
    let next = AtomicU64::new(0);
    let checked = AtomicU64::new(0);
    let timeouts = AtomicU64::new(0);
    let stop = AtomicBool::new(false);
    let divergences: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());
    let start = Instant::now();

    eprintln!("fuzzing {} cases across {} workers…", args.count, args.jobs);

    std::thread::scope(|scope| {
        for _ in 0..args.jobs {
            scope.spawn(|| {
                loop {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let idx = next.fetch_add(1, Ordering::Relaxed);
                    if idx >= args.count {
                        break;
                    }
                    let seed = args.base_seed.wrapping_add(idx);
                    let stmts = gen_case(seed, args.mode);
                    let script = build_program(&stmts);
                    let z = run_zsh(&script, timeout);
                    let r = run_zshrs(&script, &bin, timeout);
                    let done = checked.fetch_add(1, Ordering::Relaxed) + 1;
                    if z.timed_out || r.timed_out {
                        timeouts.fetch_add(1, Ordering::Relaxed);
                    }
                    // zsh-side timeout ⇒ pathological case; not a parity gap.
                    if !z.timed_out && (z.stdout != r.stdout || z.exit != r.exit) {
                        // Delta-debug to the minimal state + probe that repros.
                        let minimal = minimize(stmts, &bin, timeout);
                        let mscript = build_program(&minimal);
                        let mz = run_zsh(&mscript, timeout);
                        let mr = run_zshrs(&mscript, &bin, timeout);
                        // Re-verify: a REAL gap diverges every time; a transient
                        // (empty output from resource pressure under heavy
                        // parallelism) won't reproduce. Require `verify`
                        // consecutive divergences or discard as flaky. This is
                        // what makes a CI fuzz run non-flaky.
                        let mut confirmed = mz.stdout != mr.stdout || mz.exit != mr.exit;
                        for _ in 1..args.verify.max(1) {
                            if !confirmed {
                                break;
                            }
                            confirmed = diverges(&mscript, &bin, timeout);
                        }
                        if !confirmed {
                            continue; // flaky/transient — not a real gap
                        }
                        let rec = format!(
                            "==== seed {seed} ====\n\
                             program:\n  {}\n\
                             zsh   : exit={} timeout={}\n{}\n\
                             zshrs : exit={} timeout={}\n{}\n",
                            mscript.replace('\n', "\n  "),
                            mz.exit,
                            mz.timed_out,
                            mz.stdout.trim_end_matches('\n'),
                            mr.exit,
                            mr.timed_out,
                            mr.stdout.trim_end_matches('\n'),
                        );
                        let mut d = divergences.lock().unwrap();
                        d.push((seed, rec));
                        if d.len() >= args.max_report {
                            stop.store(true, Ordering::Relaxed);
                        }
                    }
                    if done % 500 == 0 {
                        let n = divergences.lock().unwrap().len();
                        eprintln!(
                            "  {done}/{} checked, {n} divergences, {:.0}/s",
                            args.count,
                            done as f64 / start.elapsed().as_secs_f64().max(0.001)
                        );
                    }
                }
            });
        }
    });

    let checked = checked.load(Ordering::Relaxed);
    let timeouts = timeouts.load(Ordering::Relaxed);
    // Deterministic report order regardless of thread scheduling.
    let mut divergences: Vec<(u64, String)> = divergences.into_inner().unwrap();
    divergences.sort_by_key(|(seed, _)| *seed);
    let divergences: Vec<String> = divergences.into_iter().map(|(_, r)| r).collect();

    let elapsed = start.elapsed();

    // Extract each record's program body and normalize to a gap-class signature.
    let sig_of = |rec: &str| -> String {
        let prog = rec
            .split("program:\n")
            .nth(1)
            .and_then(|s| s.split("\nzsh   :").next())
            .unwrap_or(rec);
        signature(prog)
    };

    // Apply the --baseline allowlist: a divergence whose signature is in the
    // baseline is a KNOWN gap (allowed); anything else is NEW → fails the run.
    let allowed: std::collections::HashSet<String> = match &args.baseline {
        Some(bp) => std::fs::read_to_string(bp)
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect(),
        None => std::collections::HashSet::new(),
    };
    let mut new_records: Vec<&String> = Vec::new();
    let mut new_sigs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut known = 0usize;
    for rec in &divergences {
        let sig = sig_of(rec);
        if args.baseline.is_some() && allowed.contains(&sig) {
            known += 1;
        } else {
            new_records.push(rec);
            new_sigs.insert(sig);
        }
    }

    println!(
        "\nfuzzed {checked} cases in {:.1}s ({:.0}/s)\n\
         divergences : {} ({} known / {} new)\n\
         timeouts    : {}",
        elapsed.as_secs_f64(),
        checked as f64 / elapsed.as_secs_f64().max(0.001),
        divergences.len(),
        known,
        new_records.len(),
        timeouts,
    );

    if !divergences.is_empty() {
        if let Some(parent) = args.out_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::File::create(&args.out_path) {
            for d in &divergences {
                let _ = writeln!(f, "{d}");
            }
            println!("wrote {} divergences to {}", divergences.len(), args.out_path.display());
        }
    }

    // With a baseline, only NEW (non-allowlisted) divergences fail the run.
    // Without a baseline, any divergence fails (interactive/triage use).
    if !new_records.is_empty() {
        println!("\n--- {} NEW gap signature(s) (add to baseline once triaged) ---", new_sigs.len());
        for s in &new_sigs {
            println!("{s}");
        }
        println!("\n--- first {} new divergence record(s) ---", new_records.len().min(5));
        for d in new_records.iter().take(5) {
            println!("{d}");
        }
        std::process::exit(1);
    }
    if known > 0 {
        println!("all {known} divergences are known (in baseline) — OK");
    }
}
