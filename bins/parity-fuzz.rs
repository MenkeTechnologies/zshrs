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
    stderr: String,
    exit: i32,
    timed_out: bool,
}

/// --stderr: also require the two shells' DIAGNOSTICS to agree, not just stdout
/// and exit status. Off by default: a message-text mismatch is a much softer gap
/// than a wrong value, and mixing the two would drown the hard gaps.
static CMP_STDERR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The two shells necessarily disagree on their own name in a diagnostic
/// (`zsh: no such file` vs `zshrs: no such file`), which is not a parity gap.
/// Normalize the leading shell-name tag off every line; everything after it —
/// the wording, the offending word, the line number — must match exactly.
fn norm_stderr(s: &str) -> String {
    s.lines()
        .map(|l| {
            l.strip_prefix("zshrs:")
                .or_else(|| l.strip_prefix("zsh:"))
                .unwrap_or(l)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The divergence predicate. stdout + exit always; stderr only under --stderr.
fn differs(z: &RunOut, r: &RunOut) -> bool {
    if z.stdout != r.stdout || z.exit != r.exit {
        return true;
    }
    if CMP_STDERR.load(std::sync::atomic::Ordering::Relaxed) {
        return norm_stderr(&z.stderr) != norm_stderr(&r.stderr);
    }
    false
}

/// Spawn `cmd` and wait up to `timeout`, killing it if it overruns.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> RunOut {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            return RunOut {
                stdout: String::new(),
                stderr: String::new(),
                exit: -999,
                timed_out: false,
            }
        }
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                use std::io::Read;
                let mut buf = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut buf);
                }
                let mut ebuf = String::new();
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut ebuf);
                }
                return RunOut {
                    stdout: buf,
                    stderr: ebuf,
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
                        stderr: String::new(),
                        exit: -1,
                        timed_out: true,
                    };
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => {
                return RunOut {
                    stdout: String::new(),
                    stderr: String::new(),
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
    // Pre-quoted strings for the (Q) unquote generator: a mix of single-,
    // double-, and backslash-quoting with escapes that are context-sensitive
    // (`\t` stays literal inside `"…"` but is stripped bare). The literal
    // backslash before `t`/`n` is the exact surface that exposed the (Q)
    // double-quote unescape bug.
    r#"qd='"a\tb\nc"'; "#,
    r#"qs="'a b' c"; "#,
    r#"qmix='"x y" \z '"'"'p q'"'"''; "#,
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
    let w2 = rng.gen_range(1..10);
    match rng.gen_range(0..9) {
        0 => format!("${{(l:{w}:){v}}}"),
        1 => format!("${{(r:{w}:){v}}}"),
        2 => format!("${{(l:{w}::0:){v}}}"),
        3 => format!("${{(r:{w}::-:){v}}}"),
        4 => format!("${{(l:{w}::x::y:){v}}}"),
        5 => format!("${{(r:{w}::.:){v}}}"),
        // combined l+r in one flag group: split value at len/2, left-pad the
        // first half, right-pad the second half (subst.c:949-1109).
        6 => format!("${{(l:{w}::-:r:{w2}::+:){v}}}"),
        7 => format!("${{(r:{w}::>:l:{w2}::<:){v}}}"),
        _ => format!("${{(l:{w}::AB:r:{w2}::CD:){v}}}"),
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
        // `<<`/`>>` omitted: a negative or >=64 shift amount is undefined
        // behavior in C (zsh's math backend), so those cases diverge on UB, not
        // on a real parity bug — keep the corpus focused on defined semantics.
        &["+", "-", "*", "/", "%", "**", "&", "|", "^", "<", ">", "==", "!=", "&&", "||"],
    );
    // Guard divide/mod against a zero right operand: force it nonzero via `| 1`.
    if *op == "/" || *op == "%" {
        return format!("({l}) {op} ((({r})) | 1)");
    }
    // `&&`/`||` short-circuit: zsh math.c:1459 declares `int tst` and assigns it
    // the 64-bit left operand (bop()), truncating to 32 bits for the truthiness
    // test. So `L && <computed>` yields 0 whenever L's low 32 bits are zero (any
    // multiple of 2^32, e.g. `16**9`): tst==0 spuriously sets noeval, the RHS is
    // evaluated under noeval and op() pushes 0, and the full-precision DAND
    // (math.c:1322) then ANDs against 0. zshrs computes it correctly and does NOT
    // replicate the bug. Coerce both operands to 0/1 via `!= 0` (full-precision,
    // math.c:1316) so the short-circuit test never sees a 2^32-multiple; the
    // logical value is identical, keeping the corpus on defined semantics.
    if *op == "&&" || *op == "||" {
        return format!("(({l}) != 0) {op} (({r}) != 0)");
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
    match rng.gen_range(0..14) {
        0 => "\"${(f)lines}\"".to_string(),
        1 => "${#${(f)lines}}".to_string(),          // element count after split
        2 => "\"${(j:|:)${(f)lines}}\"".to_string(), // split then re-join
        3 => "\"${(qq)spaces}\"".to_string(),        // single-quote style
        4 => "\"${(q-)spaces}\"".to_string(),        // minimal quoting
        5 => "\"${(qqqq)s}\"".to_string(),           // $'...' style
        6 => "\"${(Ff)lines}\"".to_string(),         // join-with-newline of split
        // (Q) unquote — strips ONE quoting level honoring context: `\t`
        // stays literal inside `"…"`, drops bare. Round-trip (q then Q) too.
        7 => "\"${(Q)qd}\"".to_string(),
        8 => "\"${(Q)qs}\"".to_string(),
        9 => "\"${(Q)qmix}\"".to_string(),
        10 => "${#${(Q)qd}}".to_string(),            // length after unquote
        11 => "\"${(Q)${(qq)spaces}}\"".to_string(), // q then Q round-trip
        12 => "\"${(j:/:)${(Q)${(z)qmix}}}\"".to_string(), // z-split, unquote, re-join
        _ => "\"${(@f)lines}\"".to_string(),
    }
}

/// Brace expansion — comma lists, numeric/alpha ranges, zero-pad, steps,
/// cross-products, empty elements, nesting. Deterministic (no glob chars).
fn gen_brace(rng: &mut StdRng) -> String {
    match rng.gen_range(0..14) {
        0 => "{a,b,c}".to_string(),
        1 => "pre{x,y,z}post".to_string(),
        2 => "{1..5}".to_string(),
        3 => "{5..1}".to_string(),        // descending
        4 => "{01..05}".to_string(),      // zero-padded width
        5 => "{a..e}".to_string(),        // alpha range
        6 => "{e..a}".to_string(),        // descending alpha
        7 => "{1..10..3}".to_string(),    // step
        8 => "{10..1..2}".to_string(),    // descending step
        9 => "{a,b}{1,2}".to_string(),    // cross-product
        10 => "{a,,c}".to_string(),       // empty middle element
        11 => "x{a,b{1,2},c}y".to_string(), // nested
        12 => "{-3..3}".to_string(),      // negative range
        _ => {
            let lo = rng.gen_range(0..5);
            let hi = rng.gen_range(lo..lo + 6);
            format!("v{{{lo}..{hi}}}")
        }
    }
}

/// Subscript search/reverse/flag combos — `(i)/(I)/(r)/(R)/(e)/(n)/(w)/(b)`.
fn gen_subflags(rng: &mut StdRng) -> String {
    match rng.gen_range(0..14) {
        0 => "${a[(i)three]}".to_string(),   // index of first match (name)
        1 => "${a[(I)t*]}".to_string(),      // index of last match
        2 => "${a[(r)f*]}".to_string(),      // first matching value
        3 => "${a[(R)f*]}".to_string(),      // last matching value
        4 => "${a[(e)two]}".to_string(),     // exact-string subscript
        5 => "${a[(ie)two]}".to_string(),    // index + exact
        6 => "${nums[(r)5]}".to_string(),
        7 => "${nums[(i)4]}".to_string(),
        8 => "${a[(rb:2:)o*]}".to_string(),  // reverse search from offset 2
        9 => "${a[(Rb:2:)o*]}".to_string(),
        10 => "${a[(w)1]}".to_string(),      // word subscript
        11 => "${(k)a[(r)two]}".to_string(),
        12 => "\"${a[(R)*e]}\"".to_string(),
        _ => "${a[(rb:3:)*]}".to_string(),    // forward search from offset 3
    }
}

/// Generate the raw expression list for a seed (before script assembly).
fn gen_parts(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let n = rng.gen_range(1..=3);
    let mut parts: Vec<String> = Vec::with_capacity(n);
    for _ in 0..n {
        let expr = match rng.gen_range(0..15) {
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
            10 => gen_brace(&mut rng),
            11 => gen_subflags(&mut rng),
            12 => gen_scalar_pe(&mut rng),
            13 => gen_array_pe(&mut rng),
            _ => gen_subflags(&mut rng),
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
    match rng.gen_range(0..17) {
        // Case-mismatch matches: under `setopt nocaseglob` (a mutation option),
        // conditional/case/`:#` pattern matching must stay case-SENSITIVE — only
        // filename globbing goes case-insensitive. These pin that boundary.
        14 => "[[ ABC == abc ]]; print -r -- $?".to_string(),
        15 => "[[ ABC == a*c ]]; print -r -- $?".to_string(),
        16 => "a=(ABC Abc); print -rl -- ${a:#a*c}".to_string(),
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
        // Brace expansion only happens UNQUOTED and on the command line, so
        // a `{a,b}` part is emitted bare; everything else stays double-quoted
        // (parameter expansion works inside quotes and avoids stray word-split
        // / globbing). A bare brace part with no glob chars is deterministic.
        if p.starts_with('{') || p.contains("}{") || (p.contains('{') && p.contains("..")) {
            stmts.push(format!("print -r -- {p}"));
        } else {
            stmts.push(format!("print -r -- \"{p}\""));
        }
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
    differs(&z, &r)
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
// subscript generator
//
// Array / assoc subscripting is a dense corner of zsh: search subscripts
// (`(r)`, `(R)`, `(i)`, `(I)`), exact/reverse variants, `(n:N:)` "Nth match",
// `(b:N:)` "start search at N", negative indices, and slices whose bounds run
// off both ends of the array. Every form composes with the outer expansion
// flags, and the out-of-range rules differ between element and slice syntax.
//
// Deterministic: fixed arrays, no globbing (subscript patterns never touch the
// filesystem), assoc reads are always through a single key or an ordering flag.
// ---------------------------------------------------------------------------

const SUB_STATE: &str = concat!(
    "a=(one two three four five); ",
    "nums=(3 1 4 1 5 9 2 6); ",
    "dup=(x y x y x); ",
    "typeset -A m; m=(k1 v1 k2 v2 k3 v3); ",
);

/// Subscript patterns used by the search forms. Kept free of `/` and quoting
/// metachars so they splice into `${a[(r)PAT]}` without further escaping.
const SUB_PATS: &[&str] = &[
    "t*", "*e", "one", "x", "y", "five", "z*", "[a-f]*", "?????", "*o*", "1", "5",
];

fn gen_subscript(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec![SUB_STATE.trim_end().to_string()];
    let n = rng.gen_range(2..=5);
    for _ in 0..n {
        let arr = pick(&mut rng, &["a", "nums", "dup"]);
        let expr = match rng.gen_range(0..9) {
            // Plain element, including out-of-range and negative indices.
            0 => {
                let i: i32 = rng.gen_range(-7..=7);
                format!("${{{arr}[{i}]}}")
            }
            // Slice, with bounds that may invert or run off either end.
            1 => {
                let i: i32 = rng.gen_range(-7..=7);
                let j: i32 = rng.gen_range(-7..=7);
                format!("${{{arr}[{i},{j}]}}")
            }
            // Search subscripts: (r) value, (R) reverse value, (i) index,
            // (I) reverse index. A no-match has a defined answer for each.
            2 => {
                let f = pick(&mut rng, &["r", "R", "i", "I"]);
                let p = pick(&mut rng, SUB_PATS);
                format!("${{{arr}[({f}){p}]}}")
            }
            // (e) forces the subscript to be an exact string, not a pattern.
            3 => {
                let p = pick(&mut rng, SUB_PATS);
                format!("${{{arr}[(e){p}]}}")
            }
            // (n:N:) — the Nth match rather than the first.
            4 => {
                let k = rng.gen_range(1..=3);
                let f = pick(&mut rng, &["r", "R", "i", "I"]);
                let p = pick(&mut rng, SUB_PATS);
                format!("${{{arr}[(n:{k}:{f}){p}]}}")
            }
            // (b:N:) — begin the search at offset N.
            5 => {
                let k = rng.gen_range(1..=4);
                let p = pick(&mut rng, SUB_PATS);
                format!("${{{arr}[(b:{k}:i){p}]}}")
            }
            // Search subscript composed with an outer flag.
            6 => {
                let of = pick(&mut rng, &["", "U", "L", "o", "O", "n", "#"]);
                let f = pick(&mut rng, &["r", "R"]);
                let p = pick(&mut rng, SUB_PATS);
                if of.is_empty() {
                    format!("${{{arr}[({f}){p}]}}")
                } else {
                    format!("${{({of}){arr}[({f}){p}]}}")
                }
            }
            // Assoc: key lookup, and the (k)/(v) reverse-lookup forms.
            7 => {
                let which = rng.gen_range(0..4);
                match which {
                    0 => format!("${{m[{}]}}", pick(&mut rng, &["k1", "k2", "k3", "nokey"])),
                    1 => format!("${{(k)m[(R){}]}}", pick(&mut rng, &["v1", "v2", "v9", "v*"])),
                    2 => format!("${{(v)m[(I){}]}}", pick(&mut rng, &["k1", "k*", "z*"])),
                    _ => "${(kv)m[(I)k*]}".to_string(),
                }
            }
            // Count / length of a subscripted result.
            _ => {
                let p = pick(&mut rng, SUB_PATS);
                let f = pick(&mut rng, &["r", "R", "i", "I"]);
                format!("${{#{arr}[({f}){p}]}}")
            }
        };
        // Quoted: keeps an empty/no-match result observable as an empty line
        // instead of vanishing through word-splitting.
        stmts.push(format!("print -r -- \"[{expr}]\""));
    }
    stmts
}

// ---------------------------------------------------------------------------
// pattern generator
//
// `[[ x = pat ]]` / `case` matching under EXTENDED_GLOB. Exercises the pattern
// compiler directly (no filesystem): closures (`#`, `##`), alternation,
// negation (`^`), exclusion (`~`), numeric ranges (`<->`), character classes,
// counted closures (`(#cN,M)`), case-insensitive `(#i)`, and the backreference
// forms `(#b)` / `(#m)` whose side effects land in $match/$mbegin/$mend and
// $MATCH/$MBEGIN/$MEND.
//
// Deterministic: the subject and pattern are fixed strings, and the match
// result plus every backref variable is printed.
// ---------------------------------------------------------------------------

const PAT_SUBJECTS: &[&str] = &[
    "abc123", "FooBar", "aaa", "", "a.b.c", "2024-01-31", "foo.tar.gz", "x_y-z", "999", "a",
    "abcabc", "Hello_World",
];

/// Pattern fragments that compose into a whole pattern.
const PAT_ATOMS: &[&str] = &[
    "*",
    "?",
    "a",
    "abc",
    "[a-z]",
    "[[:digit:]]",
    "[[:alpha:]]",
    "[^0-9]",
    "(a|b|foo)",
    "<1-100>",
    "<->",
    "[a-z]#",
    "[a-z]##",
    "(ab)#",
    "(#c2,3)a",
    ".",
    "_",
    "-",
];

fn gen_pattern(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec!["setopt extendedglob".to_string()];
    let n = rng.gen_range(2..=5);
    for _ in 0..n {
        let subj = pick(&mut rng, PAT_SUBJECTS);

        // Build the pattern body from 1-3 atoms.
        let parts = rng.gen_range(1..=3);
        let mut pat = String::new();
        for _ in 0..parts {
            pat.push_str(pick(&mut rng, PAT_ATOMS));
        }

        match rng.gen_range(0..6) {
            // Plain match.
            0 => stmts.push(format!(
                "[[ \"{subj}\" = {pat} ]] && print -r -- Y || print -r -- N"
            )),
            // Case-insensitive.
            1 => stmts.push(format!(
                "[[ \"{subj}\" = (#i){pat} ]] && print -r -- Y || print -r -- N"
            )),
            // Negated pattern.
            2 => stmts.push(format!(
                "[[ \"{subj}\" = ^{pat} ]] && print -r -- Y || print -r -- N"
            )),
            // Exclusion: matches `pat` but not the second pattern.
            3 => {
                let ex = pick(&mut rng, PAT_ATOMS);
                stmts.push(format!(
                    "[[ \"{subj}\" = {pat}~{ex} ]] && print -r -- Y || print -r -- N"
                ));
            }
            // (#m) — whole-match side effects.
            4 => stmts.push(format!(
                "if [[ \"{subj}\" = (#m){pat} ]]; then print -r -- \"M=$MATCH B=$MBEGIN E=$MEND\"; else print -r -- N; fi"
            )),
            // (#b) — group backreferences into $match/$mbegin/$mend.
            _ => {
                let inner = pick(&mut rng, PAT_ATOMS);
                stmts.push(format!(
                    "if [[ \"{subj}\" = (#b)({inner})* ]]; then print -r -- \"1=$match[1] b=$mbegin[1] e=$mend[1]\"; else print -r -- N; fi"
                ));
            }
        }
    }
    stmts
}

// ---------------------------------------------------------------------------
// typeset generator
//
// Parameter *attributes* rather than expansions: integer bases (`-i N` prints
// as `base#digits`), float formats (`-F`/`-E` with precision), zero-padding
// (`-Z`), left/right justification (`-L`/`-R` with a fill width), case forcing
// (`-l`/`-u`), and how each survives a later arithmetic assignment or append.
//
// Deterministic: fixed values, and every result is printed inside brackets so
// justification/padding whitespace is visible.
// ---------------------------------------------------------------------------

const TS_INT_VALS: &[&str] = &["0", "1", "7", "42", "255", "-7", "1000", "65535"];
const TS_STR_VALS: &[&str] = &["ab", "AbC", "hello", "x", "", "MiXeD", "12"];
const TS_FLT_VALS: &[&str] = &["0", "1.5", "3.14159", "-2.5", "100", "0.001"];

fn gen_typeset(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    let n = rng.gen_range(2..=4);
    for i in 0..n {
        let v = format!("v{i}");
        match rng.gen_range(0..6) {
            // Integer with an output base: `typeset -i 16 x=255` -> `16#ff`.
            0 => {
                let base = pick(&mut rng, &["2", "8", "16", "36", "10"]);
                let val = pick(&mut rng, TS_INT_VALS);
                stmts.push(format!("typeset -i {base} {v}={val}"));
                stmts.push(format!("print -r -- \"[${v}]\""));
                // An arithmetic update must keep the base attribute.
                stmts.push(format!("(( {v} = {v} + 1 ))"));
                stmts.push(format!("print -r -- \"[${v}]\""));
            }
            // Fixed-point float with precision.
            1 => {
                let prec = rng.gen_range(0..=6);
                let val = pick(&mut rng, TS_FLT_VALS);
                stmts.push(format!("typeset -F {prec} {v}={val}"));
                stmts.push(format!("print -r -- \"[${v}]\""));
            }
            // Scientific float with precision.
            2 => {
                let prec = rng.gen_range(0..=6);
                let val = pick(&mut rng, TS_FLT_VALS);
                stmts.push(format!("typeset -E {prec} {v}={val}"));
                stmts.push(format!("print -r -- \"[${v}]\""));
            }
            // Zero-padded to a width.
            3 => {
                let w = rng.gen_range(1..=8);
                let val = pick(&mut rng, TS_INT_VALS);
                stmts.push(format!("typeset -Z {w} {v}={val}"));
                stmts.push(format!("print -r -- \"[${v}]\""));
            }
            // Left / right justified to a width (padding is observable).
            4 => {
                let w = rng.gen_range(1..=8);
                let just = pick(&mut rng, &["L", "R"]);
                let val = pick(&mut rng, TS_STR_VALS);
                stmts.push(format!("typeset -{just} {w} {v}={val}"));
                stmts.push(format!("print -r -- \"[${v}]\""));
            }
            // Case forcing, and whether it survives reassignment.
            _ => {
                let case = pick(&mut rng, &["l", "u"]);
                let val = pick(&mut rng, TS_STR_VALS);
                stmts.push(format!("typeset -{case} {v}={val}"));
                stmts.push(format!("print -r -- \"[${v}]\""));
                stmts.push(format!("{v}={}", pick(&mut rng, TS_STR_VALS)));
                stmts.push(format!("print -r -- \"[${v}]\""));
            }
        }
        // Type introspection must agree on the attribute set.
        stmts.push(format!("print -r -- \"t=${{(t){v}}}\""));
    }
    stmts
}

// ---------------------------------------------------------------------------
// zutil generator
//
// `zsh/zutil` is the substrate compsys sits on: zstyle's most-specific-match
// ordering, zparseopts' option grammar, zformat's field expansion. Bugs here
// are invisible in isolation and catastrophic under a real completion system,
// which is exactly the shape a fuzzer is good at.
//
// Deterministic: fixed contexts/patterns, and every lookup is printed with its
// return status (a zstyle miss is a status, not just an empty value).
// ---------------------------------------------------------------------------

/// zstyle context patterns, ordered from broad to narrow so the generator can
/// install several that all match one lookup and force the weight comparison.
const ZS_PATS: &[&str] = &[
    "*",
    ":completion:*",
    ":completion:*:default",
    ":completion:*:*:cd:*",
    ":completion:complete:*",
    ":c*:*",
    ":completion:complete:cd:*:*",
    "*:cd:*",
];

/// Contexts to look up against the installed patterns.
const ZS_CTX: &[&str] = &[
    ":completion:complete:cd:0:default",
    ":completion:complete:ls:0",
    ":completion:default",
    ":other:thing",
    "",
];

fn gen_zutil(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec!["zmodload zsh/zutil".to_string()];

    match rng.gen_range(0..3) {
        // ---- zstyle: install N patterns, then look up. The answer depends
        // entirely on setstypat's weight ordering (Src/Modules/zutil.c:344).
        0 => {
            let n = rng.gen_range(1..=4);
            for i in 0..n {
                let pat = pick(&mut rng, ZS_PATS);
                stmts.push(format!("zstyle '{pat}' sty v{i}"));
            }
            for _ in 0..rng.gen_range(1..=3) {
                let ctx = pick(&mut rng, ZS_CTX);
                // -s (scalar), -a (array), -t (boolean test), -g (dump), and
                // the raw lookup status all read the same table differently.
                match rng.gen_range(0..4) {
                    0 => stmts.push(format!(
                        "zstyle -s '{ctx}' sty r; print -r -- \"s=[$r] rc=$?\""
                    )),
                    1 => stmts.push(format!(
                        "zstyle -a '{ctx}' sty arr; print -r -- \"a=[${{arr[*]}}] rc=$?\""
                    )),
                    2 => stmts.push(format!(
                        "zstyle -t '{ctx}' sty; print -r -- \"t=$?\""
                    )),
                    _ => stmts.push(format!(
                        "zstyle -m '{ctx}' sty 'v*'; print -r -- \"m=$?\""
                    )),
                }
            }
            // -d deletes; a later lookup must miss.
            if rng.gen_bool(0.3) {
                let pat = pick(&mut rng, ZS_PATS);
                stmts.push(format!("zstyle -d '{pat}' sty"));
                let ctx = pick(&mut rng, ZS_CTX);
                stmts.push(format!(
                    "zstyle -s '{ctx}' sty r2; print -r -- \"after_del=[$r2] rc=$?\""
                ));
            }
        }
        // ---- zparseopts: option grammar (`:` takes an arg, `+` accumulates),
        // -D (delete from argv), -E (keep going past non-options), -K (keep
        // existing array values), -a vs =NAME output forms.
        1 => {
            let args: Vec<&str> = (0..rng.gen_range(1..=5))
                .map(|_| {
                    *pick(
                        &mut rng,
                        &[
                            "-a", "-b", "val", "-ab", "-b", "x", "--", "-c", "plain", "-bval",
                            "-a", "extra",
                        ],
                    )
                })
                .collect();
            stmts.push(format!("set -- {}", args.join(" ")));

            let mut flags = String::new();
            if rng.gen_bool(0.5) {
                flags.push_str("-D ");
            }
            if rng.gen_bool(0.4) {
                flags.push_str("-E ");
            }
            let spec = pick(
                &mut rng,
                &[
                    "a=A b:=B",
                    "a+=A b:=B",
                    "a=A b=B c=C",
                    "a=A b::=B",
                    "-a -b:",
                ],
            );
            // `zparseopts` returns non-zero on an unrecognised option; print it.
            stmts.push(format!("zparseopts {flags}{spec}; print -r -- \"rc=$?\""));
            stmts.push(r#"print -r -- "A=(${A[*]}) B=(${B[*]}) C=(${C[*]})""#.to_string());
            stmts.push(r#"print -r -- "argv=(${(j: :)@})""#.to_string());
        }
        // ---- zformat: %-field substitution, width/justification, and the
        // ternary `%(c.true.false)` form.
        _ => {
            let fmt = pick(
                &mut rng,
                &[
                    "%d-%s",
                    "%10d|%-10s|",
                    "%(d.yes.no)",
                    "%d%%%s",
                    "%-5d[%5s]",
                    "%D%S",
                    "%(x.T.F)-%d",
                ],
            );
            let specs = pick(
                &mut rng,
                &[
                    "d:1 s:two",
                    "d:ab s:",
                    "d: s:xyz",
                    "d:long_value s:x",
                    "x:1 d:2 s:3",
                ],
            );
            stmts.push(format!("zformat -f out '{fmt}' {specs}"));
            stmts.push(r#"print -r -- "[$out]""#.to_string());
        }
    }
    stmts
}

// ---------------------------------------------------------------------------
// func generator
//
// Function scoping is where a shell's parameter model shows its seams:
// `local` shadowing and restore-on-return, `typeset -g` reaching past the
// local scope, arrays/assocs declared local, `$0`/`$#`/`$@` inside a function,
// nested calls, and what `return` leaves in `$?`.
// ---------------------------------------------------------------------------

fn gen_func(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec!["v=global; typeset -a arr=(g1 g2); typeset -A h=(k gv)".to_string()];

    let body = match rng.gen_range(0..7) {
        // local shadows, and the global is restored on return.
        0 => "local v=inner; print -r -- \"in=$v\"".to_string(),
        // typeset -g writes through the local scope to the global.
        1 => "local v=inner; typeset -g v=clobbered; print -r -- \"in=$v\"".to_string(),
        // local array shadowing.
        2 => "local -a arr=(l1 l2 l3); print -r -- \"in=(${arr[*]}) n=${#arr}\"".to_string(),
        // local assoc shadowing.
        3 => "local -A h=(k lv j lw); print -r -- \"in=${h[k]},${h[j]}\"".to_string(),
        // positional params + $# inside a function.
        4 => "print -r -- \"n=$# args=($*) one=$1 last=${@[-1]}\"".to_string(),
        // return propagates to $?; code after return must not run.
        5 => "print -r -- before; return 3; print -r -- AFTER_RETURN".to_string(),
        // nested call sees the caller's local (dynamic scoping).
        _ => "local v=outer_local; inner_fn".to_string(),
    };

    stmts.push("inner_fn() { print -r -- \"nested_sees=$v\" }".to_string());
    stmts.push(format!("f() {{ {body} }}"));

    let call_args = pick(&mut rng, &["", "a", "a b", "a b c", "'x y' z"]);
    stmts.push(format!("f {call_args}; print -r -- \"rc=$?\""));
    // After the call the globals must be exactly as they were, unless
    // `typeset -g` deliberately reached through.
    stmts.push(r#"print -r -- "after v=$v arr=(${arr[*]}) h=${h[k]}""#.to_string());
    stmts
}

// ---------------------------------------------------------------------------
// redir generator
//
// Redirections, MULTIOS (zsh tees a duplicated fd rather than overwriting),
// fd juggling (`exec {fd}>`, `>&-`), here-strings, and append-vs-truncate.
//
// Deterministic: everything writes into $PWD (the per-run fixture dir is not
// used by this mode; files are created and read back inside the script itself,
// then removed, so no state leaks between cases).
// ---------------------------------------------------------------------------

fn gen_redir(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    // A unique dir per case keeps parallel workers from colliding, and the
    // name is derived from the seed (not a PID/timestamp) so a replay of the
    // same seed produces byte-identical output.
    let mut stmts = vec![
        format!("d=${{TMPDIR:-/tmp}}/pf_redir_{seed}"),
        "command rm -rf $d; command mkdir -p $d; cd $d".to_string(),
    ];

    match rng.gen_range(0..6) {
        // MULTIOS: two `>` on one command tee to BOTH files.
        0 => {
            stmts.push("setopt multios".to_string());
            stmts.push("print -r -- teed > f1 > f2".to_string());
            stmts.push("print -r -- \"f1=$(<f1) f2=$(<f2)\"".to_string());
        }
        // NO_MULTIOS: the last redirect wins, the first file is empty.
        1 => {
            stmts.push("unsetopt multios".to_string());
            stmts.push("print -r -- once > f1 > f2".to_string());
            stmts.push("print -r -- \"f1=[$(<f1)] f2=[$(<f2)]\"".to_string());
        }
        // Append vs truncate.
        2 => {
            stmts.push("print -r -- one > f".to_string());
            let op = pick(&mut rng, &[">", ">>"]);
            stmts.push(format!("print -r -- two {op} f"));
            stmts.push("print -r -- \"f=($(<f))\"; wc -l < f".to_string());
        }
        // Named fd + close.
        3 => {
            stmts.push("exec {u}> f".to_string());
            stmts.push("print -r -- viafd >&$u".to_string());
            stmts.push("exec {u}>&-".to_string());
            stmts.push("print -r -- \"f=$(<f)\"".to_string());
        }
        // Here-string / here-doc into a command, and stderr merging.
        4 => {
            let src = pick(
                &mut rng,
                &["<<< 'here string'", "<<< $'a\\nb'", "<<< \"\""],
            );
            stmts.push(format!("cat {src} > f"));
            stmts.push("print -r -- \"lines=$(wc -l < f) body=[$(<f)]\"".to_string());
        }
        // stdout/stderr ordering and 2>&1 duplication.
        _ => {
            let form = pick(
                &mut rng,
                &[
                    "{ print -r -- OUT; print -ru2 -- ERR; } > f 2>&1",
                    "{ print -r -- OUT; print -ru2 -- ERR; } 2>&1 > f",
                    "{ print -r -- OUT; print -ru2 -- ERR; } >f 2>f2",
                ],
            );
            stmts.push(form.to_string());
            stmts.push("print -r -- \"f=[$(<f)]\"".to_string());
            stmts.push("[[ -e f2 ]] && print -r -- \"f2=[$(<f2)]\"".to_string());
        }
    }
    stmts.push("cd /; command rm -rf $d".to_string());
    stmts
}

// ---------------------------------------------------------------------------
// nest generator
//
// The flat `expr` mode emits one expansion at a time. Real zsh code (and the
// completion system especially) *composes* them: `${${(f)${(P)v}}[2]}`,
// `${${x#a}%b}`, `${(j:,:)${(@s: :)str}}`. Composition is where the bugs were
// in `pattern` mode too — an operator that is correct alone can still be wrong
// about what it hands to the next one (word-vs-scalar, array-vs-string, when
// the flags of the OUTER expansion apply).
//
// So: build the expansion RECURSIVELY. An inner node is itself a full
// expansion, and the outer one applies flags / subscripts / substitutions to
// whatever it produced.
// ---------------------------------------------------------------------------

/// Base variables the recursion bottoms out on. Kept in the nest preamble.
const NEST_BASE: &str = concat!(
    "s=Hello_World; ",
    "t=a:b:c:d; ",
    "path=/usr/local/bin/zsh; ",
    "lines=$'aa\\nbb\\ncc'; ",
    "a=(one two three four); ",
    "nums=(3 1 4 1 5); ",
    "typeset -A m; m=(k1 v1 k2 v2); ",
    "ptr=s; ",
    "empty=''; ",
);

/// Outer operators applied to an already-built inner expansion. Each is a
/// (prefix, suffix) pair spliced around the inner text.
fn nest_wrap(rng: &mut StdRng, inner: &str) -> String {
    match rng.gen_range(0..14) {
        // Flag-only wrappers — these are the ones whose array/scalar contract
        // is easy to get wrong one level down.
        0 => format!("${{(U){inner}}}"),
        1 => format!("${{(L){inner}}}"),
        2 => format!("${{(o){inner}}}"),
        3 => format!("${{(O){inner}}}"),
        4 => format!("${{(n){inner}}}"),
        5 => format!("${{(u){inner}}}"),
        6 => format!("${{(q){inner}}}"),
        7 => format!("${{(j:-:){inner}}}"),
        8 => format!("${{(s.:.){inner}}}"),
        9 => format!("${{(f){inner}}}"),
        // Length of the inner result.
        10 => format!("${{#{inner}}}"),
        // Substitution applied to the inner result.
        11 => format!("${{{inner}//o/0}}"),
        // Strip applied to the inner result.
        12 => format!("${{{inner}#*_}}"),
        // Subscript the inner result.
        _ => {
            let i = rng.gen_range(1..=3);
            format!("${{{inner}[{i}]}}")
        }
    }
}

/// Build an expansion of the given depth. Depth 0 is a bare variable
/// reference; each level up wraps the previous one.
fn nest_build(rng: &mut StdRng, depth: u32) -> String {
    if depth == 0 {
        // A leaf is a plain name (the `${...}` is added by the wrapper), or a
        // full expansion when it is also the whole thing.
        let v = pick(rng, &["s", "t", "path", "lines", "a", "nums", "m", "empty"]);
        return v.to_string();
    }
    let inner = nest_build(rng, depth - 1);
    // At depth 1 the inner is a bare name, so the wrapper produces `${(U)s}`.
    // Above that the inner is already a `${...}`, so the wrapper nests it:
    // `${(o)${(U)s}}` — which is exactly the shape we want to exercise.
    nest_wrap(rng, &inner)
}

fn gen_nest(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec![NEST_BASE.trim_end().to_string()];
    for _ in 0..rng.gen_range(2..=5) {
        let depth = rng.gen_range(1..=3);
        let e = nest_build(&mut rng, depth);
        // Print both quoted (one word, joins with $IFS) and `-l` (one element
        // per line) — the two disagree exactly when the array/scalar contract
        // of a nested flag is wrong, which is the bug class this mode hunts.
        if rng.gen_bool(0.5) {
            stmts.push(format!("print -r -- \"[{e}]\""));
        } else {
            stmts.push(format!("print -rl -- {e}"));
        }
    }
    stmts
}

// ---------------------------------------------------------------------------
// arith generator
//
// `expr` mode embeds arithmetic only as `$(( ))` operands. The math evaluator
// has a whole surface of its own that nothing reaches: the OUTPUT RADIX
// specifiers (`[#16]` prints `16#ff`, `[##16]` prints bare `ff`), input base
// literals (`16#ff`, `2#1010`, `0xff`), in-expression ASSIGNMENT operators
// (`+=`, `**=`, `++`), the comma operator, and the integer/float type rules
// (`7/2` is 3 but `7.0/2` is 3.5; `**` is right-associative and binds tighter
// than unary minus, so `-2**2` is -4).
//
// Deterministic: fixed operands, no $RANDOM/time. Shift amounts are masked to
// 0..63 — a negative or oversized shift is C-level UB in zsh's math backend, so
// it would diverge on UB rather than on a real parity gap.
// ---------------------------------------------------------------------------

const AR_STATE: &str = "integer i=7; integer j=-3; float f=2.5; float g=0.5; integer big=1000000";

/// A leaf operand: literal, variable, or a based/hex literal.
fn ar_leaf(rng: &mut StdRng) -> String {
    match rng.gen_range(0..9) {
        0 => rng.gen_range(-20..40).to_string(),
        1 => "i".to_string(),
        2 => "j".to_string(),
        3 => "big".to_string(),
        4 => format!("16#{:x}", rng.gen_range(1..255)),
        5 => format!("2#{:b}", rng.gen_range(1..64)),
        6 => format!("8#{:o}", rng.gen_range(1..64)),
        7 => format!("0x{:x}", rng.gen_range(1..255)),
        // 36#zz — the top of the supported input-base range.
        _ => format!("36#{}", pick(rng, &["z", "zz", "10", "a9"])),
    }
}

/// A float-typed leaf — forces the whole expression into float arithmetic,
/// which has its own division/printing rules.
fn ar_fleaf(rng: &mut StdRng) -> String {
    match rng.gen_range(0..5) {
        0 => "f".to_string(),
        1 => "g".to_string(),
        2 => format!("{}.{}", rng.gen_range(0..9), rng.gen_range(1..99)),
        3 => format!("{}e{}", rng.gen_range(1..9), rng.gen_range(1..4)),
        _ => format!("{}.0", rng.gen_range(1..20)),
    }
}

/// An integer arithmetic expression, recursive with a depth cap.
fn ar_expr(rng: &mut StdRng, depth: u32) -> String {
    if depth == 0 || rng.gen_bool(0.3) {
        return ar_leaf(rng);
    }
    let l = ar_expr(rng, depth - 1);
    let r = ar_expr(rng, depth - 1);
    match rng.gen_range(0..10) {
        // Divide / modulo: force a nonzero divisor via `| 1`.
        0 => {
            let op = pick(rng, &["/", "%"]);
            format!("({l}) {op} ((({r})) | 1)")
        }
        // Shift: mask the amount to 0..63 to stay inside defined behaviour.
        1 => {
            let op = pick(rng, &["<<", ">>"]);
            format!("({l}) {op} ((({r})) & 63)")
        }
        // `**` is RIGHT-associative and binds tighter than unary minus.
        // Cap the exponent so the result stays inside int64 (no UB overflow).
        2 => format!("({l}) ** ((({r})) & 3)"),
        // Unary forms: negation, bitwise complement, logical not.
        3 => format!("{}({l})", pick(rng, &["-", "~", "!"])),
        // Ternary, including a nested one in the true-branch.
        4 => format!("(({l})) ? ({r}) : {}", rng.gen_range(0..9)),
        // Comparison — the result is a 0/1 int.
        5 => {
            let op = pick(rng, &["<", ">", "<=", ">=", "==", "!="]);
            format!("({l}) {op} ({r})")
        }
        // `&&`/`||`: coerce both sides to 0/1 first. zsh's math.c:1459 declares
        // the short-circuit test as `int tst` and assigns it the 64-bit left
        // operand, truncating to 32 bits — so `L && x` is spuriously 0 whenever
        // L's low 32 bits are zero (any multiple of 2^32). zshrs computes it at
        // full precision and does not replicate the bug; `!= 0` keeps the corpus
        // on defined semantics without changing the logical value.
        6 => {
            let op = pick(rng, &["&&", "||"]);
            format!("(({l}) != 0) {op} (({r}) != 0)")
        }
        7 => {
            let op = pick(rng, &["+", "-", "*", "&", "|", "^"]);
            format!("({l}) {op} ({r})")
        }
        // Comma operator: evaluate left for effect, yield right.
        8 => format!("({l}), ({r})"),
        _ => {
            let op = pick(rng, &["+", "-", "*"]);
            format!("{l} {op} {r}")
        }
    }
}

fn gen_arith_mode(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec![AR_STATE.to_string()];
    for _ in 0..rng.gen_range(2..=5) {
        match rng.gen_range(0..8) {
            // Plain integer expression.
            0 => {
                let e = ar_expr(&mut rng, 3);
                stmts.push(format!("print -r -- \"$(( {e} ))\""));
            }
            // OUTPUT RADIX: `[#16]` keeps the `16#` prefix, `[##16]` strips it.
            // The radix applies to the whole expression's printed form.
            1 => {
                let base = pick(&mut rng, &["2", "8", "16", "36", "10"]);
                let hashes = if rng.gen_bool(0.5) { "#" } else { "##" };
                let e = ar_expr(&mut rng, 2);
                stmts.push(format!("print -r -- \"$(( [{hashes}{base}] {e} ))\""));
            }
            // Float expression — division does NOT truncate, and the printed
            // precision is the parity question.
            2 => {
                let l = ar_fleaf(&mut rng);
                let r = ar_fleaf(&mut rng);
                let op = pick(&mut rng, &["+", "-", "*", "/"]);
                stmts.push(format!("print -r -- \"$(( {l} {op} {r} ))\""));
            }
            // Mixed int/float: one float operand promotes the whole expression.
            3 => {
                let l = ar_leaf(&mut rng);
                let r = ar_fleaf(&mut rng);
                let op = pick(&mut rng, &["+", "*", "/", "-"]);
                stmts.push(format!("print -r -- \"$(( {l} {op} {r} ))\""));
            }
            // In-expression ASSIGNMENT: the variable keeps the new value, and
            // the expression yields it. `i` is `integer`, so a float RHS
            // truncates on assignment — a distinct rule from plain evaluation.
            4 => {
                let op = pick(&mut rng, &["=", "+=", "-=", "*=", "|=", "&=", "^=", "**="]);
                let v = pick(&mut rng, &["i", "j"]);
                let e = ar_expr(&mut rng, 1);
                // `**=` with a large/negative exponent is not meaningful; mask it.
                let rhs = if *op == "**=" {
                    format!("((({e})) & 3)")
                } else {
                    format!("({e})")
                };
                stmts.push(format!("print -r -- \"$(( {v} {op} {rhs} ))\""));
                stmts.push(format!("print -r -- \"[${v}]\""));
            }
            // Pre/post increment and decrement — the value yielded differs
            // between the two forms, and the variable is mutated either way.
            5 => {
                let v = pick(&mut rng, &["i", "j"]);
                let form = pick(&mut rng, &["++", "--"]);
                let e = if rng.gen_bool(0.5) {
                    format!("{v}{form}") // post: yields the OLD value
                } else {
                    format!("{form}{v}") // pre: yields the NEW value
                };
                stmts.push(format!("print -r -- \"$(( {e} ))\""));
                stmts.push(format!("print -r -- \"[${v}]\""));
            }
            // `(( ))` as a COMMAND: exit status is 0 iff the value is nonzero
            // (inverted relative to C truthiness).
            6 => {
                let e = ar_expr(&mut rng, 2);
                stmts.push(format!("(( {e} )); print -r -- \"rc=$?\""));
            }
            // Arithmetic in an array subscript and in a `for ((;;))` header —
            // the same evaluator reached through two different callers.
            _ => {
                let e = ar_expr(&mut rng, 1);
                stmts.push(format!(
                    "arr=(a b c d e); print -r -- \"[${{arr[(({e}) % 5) + 1]}}]\""
                ));
            }
        }
    }
    stmts
}

// ---------------------------------------------------------------------------
// match generator
//
// The substring-match FLAGS on `${var#pat}` / `${var/pat/rep}`: `(S)` search
// for the shortest match anywhere (not anchored), `(I:n:)` take the n'th match,
// `(B)`/`(E)` report the match's begin/end offset instead of the text, `(M)`
// yield the matched text itself, `(N)` yield the match LENGTH, and the `(#b)` /
// `(#m)` backreference forms that expose `$match`/`$MATCH` inside a replacement.
//
// This is the machinery compsys leans on hardest and the generators above never
// reach: `expr` only ever emits an unflagged `${v//o/0}`.
//
// Deterministic: fixed subjects, patterns without filesystem contact.
// ---------------------------------------------------------------------------

const MT_STATE: &str = "s=abcabcabc; p=/usr/local/bin/zsh; w='foo bar foo baz'; n=a1b2c3";

/// (subject-var, pattern) pairs that actually match in interesting ways —
/// several have MULTIPLE matches so (I:n:) / (S) have something to select.
const MT_SUBJ: &[(&str, &str)] = &[
    ("s", "abc"),
    ("s", "a*c"),
    ("s", "b"),
    ("s", "*b"),
    ("w", "foo"),
    ("w", "ba?"),
    ("w", "*o"),
    ("p", "*/"),
    ("p", "/*"),
    ("p", "usr"),
    ("n", "[0-9]"),
    ("n", "[a-z]"),
];

fn gen_match(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec!["setopt extendedglob".to_string(), MT_STATE.to_string()];
    for _ in 0..rng.gen_range(2..=5) {
        let (v, pat) = *pick(&mut rng, MT_SUBJ);
        let expr = match rng.gen_range(0..10) {
            // Anchored strip with a report flag: (M) matched text, (B)/(E)
            // offsets, (N) match length. Reported instead of the remainder.
            0 => {
                let f = pick(&mut rng, &["M", "B", "E", "N", "MB", "BE"]);
                let op = pick(&mut rng, &["#", "##", "%", "%%"]);
                format!("${{({f}){v}{op}{pat}}}")
            }
            // (S) — search anywhere, not anchored to the head/tail.
            1 => {
                let op = pick(&mut rng, &["#", "##", "%", "%%"]);
                format!("${{(S){v}{op}{pat}}}")
            }
            // (S) combined with a report flag.
            2 => {
                let f = pick(&mut rng, &["M", "B", "E", "N"]);
                let op = pick(&mut rng, &["#", "##"]);
                format!("${{(S{f}){v}{op}{pat}}}")
            }
            // (I:n:) — select the n'th match rather than the first.
            3 => {
                let k = rng.gen_range(1..=3);
                let f = pick(&mut rng, &["", "M", "B", "E"]);
                format!("${{(SI:{k}:{f}){v}#{pat}}}")
            }
            // Substitution with the same flags: (S) shortest-anywhere replace.
            4 => format!("${{{v}/{pat}/X}}"),
            5 => format!("${{{v}//{pat}/X}}"),
            // Anchored substitution: `/#` head-anchored, `/%` tail-anchored.
            6 => {
                let anch = pick(&mut rng, &["#", "%"]);
                format!("${{{v}/{anch}{pat}/X}}")
            }
            // (#m) — the matched text is available as $MATCH inside the
            // replacement, so the replacement can transform it.
            7 => format!("${{{v}//(#m){pat}/[${{MATCH:u}}]}}"),
            // (#b) — parenthesised groups land in $match[1..].
            8 => format!("${{{v}//(#b)({pat})/<${{match[1]}}>}}"),
            // Replacement referencing the match length, and an empty
            // replacement (deletion).
            _ => {
                if rng.gen_bool(0.5) {
                    format!("${{{v}//{pat}/}}")
                } else {
                    format!("${{(M){v}//{pat}/X}}")
                }
            }
        };
        stmts.push(format!("print -r -- \"[{expr}]\""));
    }
    stmts
}

// ---------------------------------------------------------------------------
// regex generator
//
// `[[ str =~ ere ]]` is a separate matcher from zsh's glob patterns (POSIX ERE
// via the system regex, not pattern.c), and it has SIDE EFFECTS the glob
// matcher doesn't: `$MATCH` / `$match` / `$mbegin` / `$mend` on success — or,
// under `setopt BASH_REMATCH`, the `$BASH_REMATCH` array instead. Nothing else
// in the harness touches it.
//
// Deterministic: fixed subjects/patterns, and every backref variable is printed.
// ---------------------------------------------------------------------------

const RX_SUBJ: &[&str] = &[
    "abc123", "2024-01-31", "foo.tar.gz", "Hello_World", "", "aaa", "a1b2", "x-y-z", "999",
];

const RX_PAT: &[&str] = &[
    "^[a-z]+",
    "[0-9]+$",
    "^[a-z]+[0-9]+$",
    "([a-z]+)([0-9]+)",
    "([0-9]{4})-([0-9]{2})-([0-9]{2})",
    "a{2,3}",
    "(foo|bar)",
    "^$",
    ".",
    "[[:upper:]]",
    "z*",
    "(a)(b)?",
    "\\.",
];

fn gen_regex(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    // BASH_REMATCH re-routes the capture output into a different array with a
    // different indexing convention ([0] = whole match) — both paths matter.
    let bash_rematch = rng.gen_bool(0.3);
    if bash_rematch {
        stmts.push("setopt BASH_REMATCH".to_string());
    }
    for _ in 0..rng.gen_range(2..=4) {
        let subj = pick(&mut rng, RX_SUBJ);
        let pat = pick(&mut rng, RX_PAT);
        stmts.push(format!(
            "if [[ \"{subj}\" =~ {pat} ]]; then print -r -- \"rc=0\"; else print -r -- \"rc=$?\"; fi"
        ));
        if bash_rematch {
            stmts.push(
                r#"print -r -- "BR=(${(j:,:)BASH_REMATCH}) n=${#BASH_REMATCH}""#.to_string(),
            );
        } else {
            stmts.push(
                r#"print -r -- "M=[$MATCH] m=(${(j:,:)match}) b=(${(j:,:)mbegin}) e=(${(j:,:)mend})""#
                    .to_string(),
            );
        }
    }
    stmts
}

// ---------------------------------------------------------------------------
// builtin generator
//
// The builtins that produce output but aren't reachable from any other mode:
// `print` flag matrix (-l/-r/-n/-N/-o/-O/-i/-m/-f/-P), `echo` escape handling,
// `read` (-r/-A/-d/-k/-E/-e) fed from a here-string, `getopts` option parsing,
// `let`, `eval`, `[[ -o opt ]]` option introspection, and `typeset -p` output.
//
// Deterministic: no tty reads (every `read` is fed from a here-string), no
// filesystem, no time. `print -P` is restricted to prompt escapes with no
// environment dependence (no %~, %M, %n, %T).
// ---------------------------------------------------------------------------

const BI_WORDS: &[&str] = &["delta", "alpha", "Charlie", "bravo", "echo2", "a b", "", "42"];

fn gen_builtin(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();

    match rng.gen_range(0..7) {
        // ---- print: the flag matrix, over a fixed word list.
        0 => {
            let n = rng.gen_range(1..=4);
            let words: Vec<String> = (0..n)
                .map(|_| format!("'{}'", pick(&mut rng, BI_WORDS)))
                .collect();
            let flag = pick(
                &mut rng,
                &[
                    "-r", "-l", "-rl", "-n", "-rn", "-N", "-o", "-O", "-oi", "-Oi", "-lo", "-ln",
                ],
            );
            stmts.push(format!("print {flag} -- {}", words.join(" ")));
            stmts.push("print -r -- END".to_string());
        }
        // ---- print -m: only the args matching a pattern are printed.
        1 => {
            let pat = pick(&mut rng, &["a*", "*a*", "[A-Z]*", "?", "*2"]);
            stmts.push(format!(
                "print -rlm -- '{pat}' alpha bravo Charlie delta a2 x; print -r -- END"
            ));
        }
        // ---- echo: escape handling differs from print, and -e/-E flip it.
        2 => {
            let flag = pick(&mut rng, &["", "-n", "-e", "-E", "-ne"]);
            let body = pick(
                &mut rng,
                &[
                    r"a\tb",
                    r"a\nb",
                    r"x\\y",
                    r"\e[1m",
                    r"c\cd",
                    r"\0101",
                    r"no_escapes",
                    r"a\x41b",
                ],
            );
            stmts.push(format!("echo {flag} '{body}'; print -r -- END"));
        }
        // ---- read: fed from a here-string so it never blocks on a tty.
        3 => {
            let input = pick(
                &mut rng,
                &["one two three", "  lead and trail  ", "a:b:c", r"esc\ttab", "single"],
            );
            match rng.gen_range(0..5) {
                // Plain read: splits on IFS, last var gets the remainder.
                0 => {
                    stmts.push(format!("read a b c <<< '{input}'"));
                    stmts.push(r#"print -r -- "a=[$a] b=[$b] c=[$c]""#.to_string());
                }
                // -r: backslashes stay literal.
                1 => {
                    stmts.push(format!("read -r line <<< '{input}'"));
                    stmts.push(r#"print -r -- "[$line]""#.to_string());
                }
                // -A: the whole line splits into an array.
                2 => {
                    stmts.push(format!("read -A arr <<< '{input}'"));
                    stmts.push(r#"print -r -- "n=${#arr} [${(j:|:)arr}]""#.to_string());
                }
                // -d: a custom delimiter ends the read.
                3 => {
                    let d = pick(&mut rng, &[":", " ", "t"]);
                    stmts.push(format!("read -d '{d}' x <<< '{input}'; print -r -- \"[$x] rc=$?\""));
                }
                // -k: read exactly N characters.
                _ => {
                    let k = rng.gen_range(1..=4);
                    stmts.push(format!("read -k {k} x <<< '{input}'; print -r -- \"[$x]\""));
                }
            }
            // A read past EOF must fail with a nonzero status and leave the
            // variable empty — a distinct code path from a successful read.
            stmts.push("read eofv < /dev/null; print -r -- \"eof_rc=$? [$eofv]\"".to_string());
        }
        // ---- getopts: the option-parsing loop, including a bad option and a
        // missing required argument (both have defined, distinct behaviour).
        4 => {
            let args: Vec<&str> = (0..rng.gen_range(1..=4))
                .map(|_| *pick(&mut rng, &["-a", "-b", "val", "-c", "-ab", "-bval", "--", "x", "-z"]))
                .collect();
            stmts.push(format!("set -- {}", args.join(" ")));
            stmts.push(
                "while getopts ab:c opt; do print -r -- \"opt=$opt arg=[$OPTARG]\"; done"
                    .to_string(),
            );
            stmts.push(r#"print -r -- "rc=$? ind=$OPTIND rest=(${(j:,:)@[OPTIND,-1]})""#.to_string());
        }
        // ---- let / eval: arithmetic-as-command and re-parsed text.
        5 => {
            let e = ar_expr(&mut rng, 2);
            stmts.push("integer i=7 j=-3 big=1000000".to_string());
            stmts.push(format!("let \"x = {e}\"; print -r -- \"x=$x rc=$?\""));
            let ev = pick(
                &mut rng,
                &[
                    r#"eval 'print -r -- evaled'"#,
                    r#"v=inner; eval 'print -r -- "$v"'"#,
                    r#"eval 'a=(1 2 3)'; print -r -- "${#a}""#,
                    r#"q="print -r -- fromvar"; eval $q"#,
                    r#"eval 'exit 4'"#,
                ],
            );
            stmts.push(format!("( {ev} ); print -r -- \"rc=$?\""));
        }
        // ---- option introspection: `[[ -o opt ]]` must track setopt state,
        // including the `no`-prefixed spelling and the alias spellings.
        _ => {
            let opt = pick(
                &mut rng,
                &["extendedglob", "ksharrays", "shwordsplit", "nounset", "multios", "aliases"],
            );
            let set = rng.gen_bool(0.5);
            stmts.push(format!("{}setopt {opt}", if set { "" } else { "un" }));
            stmts.push(format!("[[ -o {opt} ]]; print -r -- \"o=$?\""));
            stmts.push(format!("[[ -o no{opt} ]]; print -r -- \"no=$?\""));
            // typeset -p round-trips a parameter's full declaration; the exact
            // rendering (quoting, attribute order) is the parity question.
            stmts.push("typeset -i tv=42; typeset -p tv".to_string());
            stmts.push("typeset -a ta=(x 'a b'); typeset -p ta".to_string());
        }
    }
    stmts
}

// ---------------------------------------------------------------------------
// cmdsub generator
//
// Command substitution has rules that no other mode probes: ALL trailing
// newlines are stripped (not just one), an unquoted result word-splits on $IFS
// while a quoted one does not, `$(<file)` is a distinct (fork-free) code path
// from `$(cat file)`, and a substitution's exit status only survives when it is
// the whole command.
//
// Deterministic: a per-seed temp dir (name derived from the seed, not a
// pid/timestamp), and the substituted commands are fixed built-ins.
// ---------------------------------------------------------------------------

fn gen_cmdsub(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec![
        format!("d=${{TMPDIR:-/tmp}}/pf_cs_{seed}"),
        "command rm -rf $d; command mkdir -p $d; cd $d".to_string(),
        "printf 'l1\\nl2\\nl3\\n' > f; printf 'trail\\n\\n\\n' > g; printf 'noeol' > h".to_string(),
        "IFS=$' \\t\\n'".to_string(),
    ];
    for _ in 0..rng.gen_range(2..=4) {
        let stmt = match rng.gen_range(0..12) {
            // Trailing newlines are ALL stripped, quoted or not.
            0 => r#"print -r -- "[$(printf 'a\n\n\n')]""#.to_string(),
            // `$(<file)` — the fork-free read path.
            1 => r#"print -r -- "[$(<f)]""#.to_string(),
            2 => r#"print -r -- "[$(<g)]""#.to_string(),
            // A file with no trailing newline.
            3 => r#"print -r -- "[$(<h)]""#.to_string(),
            // Quoted vs unquoted: the unquoted form word-splits.
            4 => r#"print -rl -- $(<f); print -r -- END"#.to_string(),
            5 => r#"print -rl -- "$(<f)"; print -r -- END"#.to_string(),
            // Nested substitution.
            6 => r#"print -r -- "[$(echo $(echo inner))]""#.to_string(),
            // Backtick form must agree with `$( )`.
            7 => "print -r -- \"[`echo tick`]\"".to_string(),
            // Splitting under a custom IFS.
            8 => {
                let ifs = pick(&mut rng, &[":", ",", "x"]);
                format!("(IFS={ifs}; print -rl -- $(print -r -- 'a{ifs}b{ifs}c'); print -r -- END)")
            }
            // Substitution feeding an array assignment.
            9 => r#"arr=( $(<f) ); print -r -- "n=${#arr} [${(j:|:)arr}]""#.to_string(),
            // Exit status of the substituted command propagates only when the
            // substitution is the entire command.
            10 => r#"$(exit 3); print -r -- "rc=$?"; x=$(exit 5); print -r -- "assign_rc=$?""#
                .to_string(),
            // Substitution inside arithmetic, and an expansion applied to the
            // substitution's result.
            _ => r#"print -r -- "[$(( $(print -r -- 3) + 4 ))] [${$(<f)[2]}] [${(U)$(<h)}]""#
                .to_string(),
        };
        stmts.push(stmt);
    }
    stmts.push("cd /; command rm -rf $d".to_string());
    stmts
}

// ---------------------------------------------------------------------------
// loop generator
//
// Control flow: `for` (list and C-style), `while`/`until`, `repeat`, the
// `break N` / `continue N` multi-level forms, and the zsh `case` fallthrough
// terminators `;&` (fall into the next branch unconditionally) and `;|` (retry
// the remaining patterns). Every body prints, so the exact iteration order and
// early-exit point are observable.
// ---------------------------------------------------------------------------

fn gen_loop(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec!["items=(p q r); nums=(1 2 3 4)".to_string()];
    for _ in 0..rng.gen_range(1..=3) {
        let stmt = match rng.gen_range(0..10) {
            // Nested loops with a multi-level break: `break 2` leaves BOTH.
            0 => {
                let lvl = rng.gen_range(1..=2);
                format!(
                    "for x in $items; do for y in $nums; do (( y == 2 )) && break {lvl}; print -r -- \"$x$y\"; done; done; print -r -- END"
                )
            }
            // Multi-level continue.
            1 => {
                let lvl = rng.gen_range(1..=2);
                format!(
                    "for x in $items; do for y in $nums; do (( y == 2 )) && continue {lvl}; print -r -- \"$x$y\"; done; done; print -r -- END"
                )
            }
            // C-style for.
            2 => {
                let n = rng.gen_range(0..=4);
                format!("for (( k = 0; k < {n}; k++ )); do print -r -- \"k=$k\"; done; print -r -- \"after=$k\"")
            }
            // while with an arithmetic condition.
            3 => "i=0; while (( i < 3 )); do print -r -- \"w=$i\"; (( i++ )); done".to_string(),
            // until — the inverted condition.
            4 => "i=0; until (( i >= 3 )); do print -r -- \"u=$i\"; (( i++ )); done".to_string(),
            // repeat N — a fixed count with no loop variable.
            5 => {
                let n = rng.gen_range(0..=4);
                format!("repeat {n}; do print -r -- rep; done; print -r -- END")
            }
            // `;&` — unconditional fallthrough into the NEXT branch's body.
            6 => {
                let subj = pick(&mut rng, &["a", "b", "c", "z"]);
                format!(
                    "case {subj} in (a) print -r -- A ;& (b) print -r -- B ;& (c) print -r -- C ;; (*) print -r -- OTHER ;; esac"
                )
            }
            // `;|` — re-test the REMAINING patterns after a match.
            7 => {
                let subj = pick(&mut rng, &["ab", "a", "b", "zz"]);
                format!(
                    "case {subj} in (a*) print -r -- A ;| (*b) print -r -- B ;| (??) print -r -- TWO ;; (*) print -r -- OTHER ;; esac"
                )
            }
            // for over a parameter expansion, with the loop var leaking after.
            8 => "for x in ${(o)items} ${nums[2,3]}; do print -rn -- \"$x.\"; done; print; print -r -- \"leak=$x\""
                .to_string(),
            // A loop whose body redefines the list it iterates: the list is
            // snapshotted at loop entry.
            _ => "l=(1 2 3); for e in $l; do l+=(x); print -rn -- \"$e\"; done; print; print -r -- \"n=${#l}\""
                .to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// split generator
//
// Word splitting is the one rule every other mode carefully avoids so its
// output stays stable — which means nothing here was being tested. This mode
// attacks it head-on: IFS (whitespace vs non-whitespace separators, empty,
// multi-char, unset), the SH_WORD_SPLIT / RC_EXPAND_PARAM / GLOB_SUBST /
// KSH_ARRAYS options, the explicit ${=x} (split) and ${~x} (glob) flags, and
// the $* / "$*" / "$@" / ${arr[*]} join rules that use IFS[1].
//
// Determinism: values hold no glob metacharacter that could match a real file,
// so even GLOB_SUBST / ${~x} either match nothing (NOMATCH error, deterministic)
// or expand to themselves. Nothing here touches the filesystem's contents.
// ---------------------------------------------------------------------------

/// Options that change splitting/joining/expansion of a *word*.
const SPLIT_OPTS: &[&str] = &[
    "SH_WORD_SPLIT",
    "RC_EXPAND_PARAM",
    "GLOB_SUBST",
    "KSH_ARRAYS",
    "NULL_GLOB",
    "NO_NOMATCH",
    "CSH_NULL_GLOB",
    "SH_NULLCMD",
];

/// IFS settings that exercise the whitespace/non-whitespace split rule: a run of
/// whitespace separators collapses to one break, a non-whitespace separator does
/// not (so `a::b` under IFS=: yields an EMPTY middle field).
const IFS_VALS: &[&str] = &[
    "$' \\t\\n'", // default
    "':'",
    "':,'",
    "' :'", // mixed whitespace + non-whitespace
    "''",   // empty: no splitting at all
    "$'\\n'",
    "'x'",
];

fn gen_split_mode(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();

    // A value whose split depends entirely on IFS: leading/trailing/doubled
    // separators are where the whitespace-vs-not rule actually bites.
    let val = pick(
        &mut rng,
        &[
            "a b  c",
            "a:b::c",
            " a:b ",
            ":a:b:",
            "a, b ,c",
            "one",
            "",
            "a\tb\nc",
            "xaxbx",
        ],
    );
    stmts.push(format!("v=$'{val}'"));
    stmts.push("arr=(p q r); e=(); empty=''".to_string());
    if rng.gen_bool(0.7) {
        stmts.push(format!("IFS={}", pick(&mut rng, IFS_VALS)));
    }
    if rng.gen_bool(0.6) {
        let neg = if rng.gen_bool(0.3) { "un" } else { "" };
        stmts.push(format!("{neg}setopt {}", pick(&mut rng, SPLIT_OPTS)));
    }

    for _ in 0..rng.gen_range(2..=4) {
        let probe = match rng.gen_range(0..16) {
            // Unquoted scalar: splits only under SH_WORD_SPLIT.
            0 => "print -rl -- $v; print -r -- END",
            // Quoted: never splits, whatever the options.
            1 => r#"print -rl -- "$v"; print -r -- END"#,
            // ${=v}: force splitting regardless of SH_WORD_SPLIT.
            2 => "print -rl -- ${=v}; print -r -- END",
            // ${=v} inside quotes still splits (the flag beats the quoting).
            3 => r#"print -rl -- "${=v}"; print -r -- END"#,
            // Counting the fields is a sharper probe than printing them.
            4 => "a=( ${=v} ); print -r -- \"n=${#a} [${(j:|:)a}]\"",
            // Explicit (s) split flag — independent of IFS.
            5 => r#"print -r -- "[${(s.:.)v}]" "[${(ps.:.)v}]""#,
            // (f) splits on newlines only; (z) does shell-word splitting.
            6 => r#"print -r -- "n=${#${(f)v}} z=${#${(z)v}}""#,
            // $* / "$*" join with IFS[1]; "$@" never joins.
            7 => "set -- a b c; print -rl -- \"$*\"; print -rl -- \"$@\"; print -r -- END",
            8 => "set -- a b c; print -rl -- $*; print -r -- END",
            // ${arr[*]} joins on IFS[1]; ${arr[@]} does not. With an EMPTY IFS
            // the join is a bare concatenation.
            9 => r#"print -r -- "[${arr[*]}] [${arr[@]}]""#,
            // RC_EXPAND_PARAM: `x${arr}y` distributes the prefix/suffix over
            // every element instead of only the first/last.
            10 => "print -rl -- pre${arr}post; print -r -- END",
            // Empty array / empty scalar in a word: does it produce an empty
            // word or no word at all?
            11 => "print -rl -- x$e y; print -r -- \"n=$#\"; set -- $e; print -r -- \"after=$#\"",
            12 => r#"a=( "$empty" $empty ); print -r -- "n=${#a}""#,
            // ${~v}: force glob expansion of the VALUE. The pattern matches no
            // real file, so NOMATCH / NULL_GLOB / CSH_NULL_GLOB decide the
            // outcome (error, removal, or literal) — all deterministic.
            13 => "p='zzq*'; print -rl -- ${~p}; print -r -- \"rc=$?\"",
            // GLOB_SUBST does the same implicitly, for an unquoted expansion.
            14 => "p='zzq*'; print -rl -- $p; print -r -- \"rc=$?\"",
            // Assignment never word-splits, even under SH_WORD_SPLIT — the RHS
            // of a scalar assignment is one word.
            _ => r#"w=$v; print -r -- "[$w]"; a=($v); print -r -- "n=${#a}""#,
        };
        stmts.push(probe.to_string());
    }
    stmts
}

// ---------------------------------------------------------------------------
// trap generator
//
// Traps are pure control-flow state: which handler runs, in what order, with
// what $?, and whether a handler set in a function/subshell survives the frame.
// zsh has two spellings (`trap '…' SIG` and the `TRAPSIG()` function form) with
// DIFFERENT semantics — the function form runs in its own frame with its own $?
// and can `return`. Plus `always` blocks and TRY_BLOCK_ERROR.
//
// Determinism: only self-sent signals (kill -SIG $$), no timers, no children.
// ---------------------------------------------------------------------------

fn gen_trap(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();

    for _ in 0..rng.gen_range(1..=2) {
        let stmt = match rng.gen_range(0..14) {
            // EXIT trap fires at shell exit, and sees the exit status in $?.
            0 => "trap 'print -r -- \"exit-trap rc=$?\"' EXIT; print -r -- body; exit 3".to_string(),
            // EXIT trap in a subshell fires when the SUBSHELL exits, not later.
            1 => "trap 'print -r -- outer' EXIT; ( trap 'print -r -- inner' EXIT; print -r -- sub ); print -r -- after".to_string(),
            // A trap set inside a function is GLOBAL unless LOCAL_TRAPS is set.
            2 => {
                let lt = if rng.gen_bool(0.5) { "setopt local_traps; " } else { "" };
                format!("trap 'print -r -- outer-usr1' USR1; f() {{ {lt}trap 'print -r -- inner-usr1' USR1; kill -USR1 $$ }}; f; kill -USR1 $$")
            }
            // ERR trap fires on every failing command (not inside a condition).
            3 => "trap 'print -r -- \"err rc=$?\"' ERR; false; print -r -- mid; if false; then :; fi; true; print -r -- done".to_string(),
            // TRAPZERR() — the function spelling of ERR.
            4 => "TRAPZERR() { print -r -- \"zerr rc=$?\" }; false; (exit 7); print -r -- end".to_string(),
            // `trap - SIG` removes the handler; `trap ''` ignores the signal.
            5 => {
                let which = pick(&mut rng, &["-", "''"]);
                format!("trap 'print -r -- caught' USR2; kill -USR2 $$; trap {which} USR2; kill -USR2 $$; print -r -- survived")
            }
            // `trap` with no args LISTS the installed handlers.
            6 => "trap 'print -r -- a' USR1; trap 'print -r -- b' EXIT; trap".to_string(),
            // always block: runs whether the try block succeeded or failed.
            7 => {
                let body = pick(&mut rng, &["true", "false", "print -r -- t; false", "return 2"]);
                format!("f() {{ {{ {body} }} always {{ print -r -- \"always rc=$?\" }}; print -r -- \"after rc=$?\" }}; f; print -r -- \"outer rc=$?\"")
            }
            // TRY_BLOCK_ERROR: set inside `always` when the try block was
            // aborted by an error; zeroing it swallows the error.
            8 => {
                let clear = if rng.gen_bool(0.5) { "TRY_BLOCK_ERROR=0; " } else { "" };
                format!("f() {{ {{ typeset -r ro=1; ro=2 }} always {{ print -r -- \"tbe=$TRY_BLOCK_ERROR\"; {clear}}} ; print -r -- reached }}; f; print -r -- \"rc=$?\"")
            }
            // The function form of a signal trap runs in its own frame: a
            // `local` inside it must not leak, and `return` exits only the trap.
            9 => "g=outer; TRAPUSR1() { local g=inner; print -r -- \"in=$g\"; return }; kill -USR1 $$; print -r -- \"out=$g\"".to_string(),
            // A trap fired while a function is running resumes the function.
            10 => "TRAPUSR1() { print -r -- trap }; f() { print -r -- pre; kill -USR1 $$; print -r -- post }; f".to_string(),
            // DEBUG trap runs BEFORE each command; EXIT ordering vs it matters.
            11 => "trap 'print -r -- \"dbg\"' DEBUG; print -r -- one; print -r -- two; trap - DEBUG; print -r -- three".to_string(),
            // errexit + trap: the EXIT trap still runs when errexit kills us.
            12 => "setopt err_exit; trap 'print -r -- \"exiting rc=$?\"' EXIT; print -r -- before; false; print -r -- unreachable".to_string(),
            // An `always` block runs even when the try block `return`s, and the
            // return value survives it.
            _ => "f() { { print -r -- try; return 5 } always { print -r -- fin } }; f; print -r -- \"rc=$?\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// pipeline generator
//
// Exit-status plumbing: $? through pipes/negation/&&/||, $pipestatus (and the
// ksh-spelled $PIPESTATUS), PIPE_FAIL, ERR_EXIT and its interaction with
// conditions, and the "each pipeline stage is a subshell" rule (so a `read` or
// an assignment in the last stage does NOT survive — zsh forks the last stage
// too, unlike ksh).
// ---------------------------------------------------------------------------

fn gen_pipeline(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();

    if rng.gen_bool(0.4) {
        let neg = if rng.gen_bool(0.3) { "un" } else { "" };
        stmts.push(format!("{neg}setopt {}", pick(&mut rng, &["pipe_fail", "err_exit", "err_return"])));
    }

    for _ in 0..rng.gen_range(1..=3) {
        let stmt = match rng.gen_range(0..14) {
            // $? of a pipeline is the LAST stage's status (unless PIPE_FAIL).
            0 => {
                let a = pick(&mut rng, &["true", "false", "(exit 3)"]);
                let b = pick(&mut rng, &["true", "false", "(exit 4)"]);
                format!("{a} | {b}; print -r -- \"rc=$? ps=(${{(j:,:)pipestatus}})\"")
            }
            // Three stages: pipestatus must have one entry per stage, in order.
            1 => "(exit 1) | (exit 0) | (exit 2); print -r -- \"rc=$? ps=(${(j:,:)pipestatus}) n=${#pipestatus}\"".to_string(),
            // The ksh spelling is an alias of the same array.
            2 => "false | true; print -r -- \"ps=${(j:,:)pipestatus} PS=${(j:,:)PIPESTATUS}\"".to_string(),
            // `!` negates the pipeline status (0<->1, never other values).
            3 => {
                let p = pick(&mut rng, &["true", "false", "(exit 7)"]);
                format!("! {p}; print -r -- \"rc=$?\"")
            }
            // && / || short-circuit chains, and their combined status.
            4 => {
                let c = pick(
                    &mut rng,
                    &[
                        "true && print -r -- A || print -r -- B",
                        "false && print -r -- A || print -r -- B",
                        "false || false || print -r -- C",
                        "true && false && print -r -- D",
                        "(exit 3) || print -r -- E",
                    ],
                );
                format!("{c}; print -r -- \"rc=$?\"")
            }
            // Every stage — INCLUDING the last — runs in a subshell: `x` does
            // not survive the pipeline.
            5 => "x=before; print -r -- new | read x; print -r -- \"x=$x\"".to_string(),
            6 => "n=0; print -rl -- a b c | while read l; do (( n++ )); done; print -r -- \"n=$n\"".to_string(),
            // A pipeline reading from a builtin producer into a builtin consumer.
            7 => "print -rl -- c a b | sort | print -rl -- $(cat); print -r -- END".to_string(),
            // The status of a compound as a pipeline stage.
            8 => "{ print -r -- x; false } | cat; print -r -- \"rc=$? ps=${(j:,:)pipestatus}\"".to_string(),
            // ERR_EXIT does NOT fire for a command in a condition context, but
            // DOES for a bare failing command.
            9 => "if false; then :; fi; while false; do :; done; false || true; print -r -- alive; false; print -r -- unreachable".to_string(),
            // ERR_RETURN inside a function returns instead of exiting.
            10 => "f() { setopt local_options err_return; print -r -- in; false; print -r -- never }; f; print -r -- \"rc=$? still-here\"".to_string(),
            // Exit status of an empty/`:`-only compound, and of an assignment
            // whose RHS is a failing substitution.
            11 => "(:); print -r -- \"rc=$?\"; v=$(exit 6); print -r -- \"assign=$?\"; v=$(exit 6) print -r -- prefix; print -r -- \"pre_rc=$?\"".to_string(),
            // `command` / `builtin` prefixes must not change the status.
            12 => "builtin false; print -r -- \"b=$?\"; command false; print -r -- \"c=$?\"".to_string(),
            // Background + wait: `wait` yields the job's status. Deterministic
            // (one job, fixed exit code).
            _ => "(exit 9) & wait $!; print -r -- \"wait=$?\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// prompt generator
//
// `print -P` runs the full prompt expander (Src/prompt.c promptexpand) — the
// %-escape vocabulary, the visual attributes (%B/%U/%S/%F/%K) that emit real
// terminal escapes, the ternary `%(x.t.f)` conditions, and the truncation forms
// `%N<…<` / `%N>…>` which are their own little algorithm.
//
// Determinism: only escapes whose value is identical in both shells (attributes,
// literals, ternaries on $?, truncation). Never %D/%T (time), %! (history), %N
// (script name — argv[0] genuinely differs), %i (line number).
// ---------------------------------------------------------------------------

const PROMPT_ATOMS: &[&str] = &[
    "%B bold %b",
    "%U under %u",
    "%S standout %s",
    "%F{red}red%f",
    "%F{4}blue%f",
    "%K{green}bg%k",
    "%F{red}%Kboth%k%f",
    "%%",
    "%)",
    "%{raw%}lit",
    "a%Bb%bc",
    "%(?.ok.bad)",
    "%(?.%F{green}Y%f.%F{red}N%f)",
    "%(1j.jobs.nojobs)",
    "%(#.root.user)",
    "%10(l.wide.narrow)",
    "%5<...<abcdefghij",
    "%5>...>abcdefghij",
    "%3<<abcdefg",
    "%20<..<short",
    "%c",
    "%2d",
    "%-1d",
];

fn gen_prompt(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(1..=3) {
        // A prior command fixes $? so the %(?..) ternaries are deterministic.
        if rng.gen_bool(0.4) {
            stmts.push(if rng.gen_bool(0.5) { "true".into() } else { "false".into() });
        }
        let n = rng.gen_range(1..=3);
        let body: Vec<&str> = (0..n).map(|_| *pick(&mut rng, PROMPT_ATOMS)).collect();
        let s = body.join("|");
        match rng.gen_range(0..4) {
            // print -P: the prompt expander on an explicit string.
            0 => stmts.push(format!("print -P -- '{s}'")),
            // -n: no trailing newline, so trailing-escape handling is visible.
            1 => stmts.push(format!("print -Pn -- '{s}'; print -r -- '|END'")),
            // ${(%)…} — the same expander as a parameter flag.
            2 => stmts.push(format!("p='{s}'; print -r -- \"${{(%)p}}\"")),
            // Expansion inside the prompt string: %-escapes AND ${…} together
            // (PROMPT_SUBST off ⇒ the ${…} is expanded by the double quotes,
            // not by the prompt expander).
            _ => stmts.push(format!("v=VAL; print -P -- \"{s}:$v\"")),
        }
    }
    stmts
}

// ---------------------------------------------------------------------------
// modifier generator
//
// History-style modifiers on parameters (Src/hist.c) — `:h :t :r :e :l :u :s :&
// :g :q :Q :f :F :A :a :P` — plus the array set-ops that share the `:` prefix
// (`:|` diff, `:*` intersect, `:^` zip, `:#` filter). These are what zpwr and
// compsys lean on for path munging; they have no other coverage.
//
// Determinism: the paths are literal strings, never globbed. `:a` / `:A` do
// touch the cwd, but both shells run with the same cwd, and the fixed relative
// paths below do not exist, so :A degrades to :a identically.
// ---------------------------------------------------------------------------

const MOD_PATHS: &[&str] = &[
    "/a/b/c.txt",
    "/a/b/",
    "dir/file.tar.gz",
    "noext",
    ".hidden",
    "/",
    "a.b.c",
    "./rel/x.rs",
    "../up/y.md",
    "trailing/",
    "sp ace/f.c",
];

const MODS: &[&str] = &[
    ":h", ":t", ":r", ":e", ":l", ":u", ":q", ":Q", ":h:t", ":t:r", ":r:e", ":h:h", ":a", ":s/a/Z/",
    ":gs/a/Z/", ":s|/|_|", ":gs|/|_|", ":s/x/Y/:&", ":fs/a//", ":t:u", ":r:l",
];

fn gen_modifier(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();

    match rng.gen_range(0..6) {
        // Scalar + a modifier chain.
        0 | 1 => {
            for _ in 0..rng.gen_range(1..=3) {
                let p = pick(&mut rng, MOD_PATHS);
                let m = pick(&mut rng, MODS);
                stmts.push(format!("f='{p}'; print -r -- \"[${{f{m}}}]\""));
            }
        }
        // Array: a modifier applies to EVERY element.
        2 => {
            let m = pick(&mut rng, MODS);
            stmts.push(
                "a=(/x/one.txt two/three.tar.gz /four/ five)".to_string(),
            );
            stmts.push(format!("print -rl -- \"${{a{m}}}\"; print -r -- END"));
            stmts.push(format!("print -rl -- ${{^a{m}}}; print -r -- END"));
        }
        // The `${name:#pat}` filter and its (M) inverse, on an array.
        3 => {
            let pat = pick(&mut rng, &["*.txt", "a*", "?", "*x*", "[0-9]*"]);
            stmts.push("a=(a.txt bx c1 2d ax.txt)".to_string());
            stmts.push(format!("print -rl -- ${{a:#{pat}}}; print -r -- END"));
            stmts.push(format!("print -rl -- ${{(M)a:#{pat}}}; print -r -- END"));
        }
        // Array set-ops: difference / intersection / zip.
        4 => {
            stmts.push("a=(x y z y); b=(y w)".to_string());
            let op = pick(&mut rng, &[":|b", ":*b", ":^b", ":^^b"]);
            stmts.push(format!("print -rl -- ${{a{op}}}; print -r -- END"));
            stmts.push(format!("print -r -- \"[${{(j:,:)a{op}}}]\""));
        }
        // Modifiers combined with a parameter flag, and applied to $0/$PWD-ish
        // values through a nested expansion.
        _ => {
            let m = pick(&mut rng, MODS);
            stmts.push("f='/usr/local/lib/libz.so.1'".to_string());
            stmts.push(format!("print -r -- \"[${{(U)f{m}}}]\""));
            stmts.push(format!("print -r -- \"[${{${{f{m}}}:-EMPTY}}]\""));
            stmts.push(format!("a=(/p/q.c /r/s.h); print -r -- \"[${{(j:,:)a{m}}}]\""));
        }
    }
    stmts
}

// ---------------------------------------------------------------------------
// mathfunc generator
//
// `zmodload zsh/mathfunc` (Src/Modules/mathfunc.c) plus `functions -M`, the
// user-defined math-function mechanism (Src/builtin.c bin_functions / math.c
// callmathfunc). Both extend the arithmetic evaluator's namespace at runtime —
// a path no arith-mode case reaches, since arith mode only uses operators.
//
// Determinism: no rand48/rand. Float results print through the shell's own
// float formatter, which is the parity question, so they are NOT rounded away.
// ---------------------------------------------------------------------------

/// Single-argument mathfunc calls with exactly representable or stable results.
const MF_1ARG: &[&str] = &[
    "sqrt(4)", "sqrt(2)", "abs(-3)", "fabs(-2.5)", "ceil(1.2)", "floor(1.8)", "rint(2.5)",
    "int(3.9)", "float(3)", "log(1)", "exp(0)", "log10(100)", "sin(0)", "cos(0)", "asin(1)",
    "atan(1)", "sinh(0)", "tanh(0)", "erf(0)", "j0(0)",
];

fn gen_mathfunc(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec!["zmodload zsh/mathfunc".to_string()];

    match rng.gen_range(0..6) {
        // Bare calls, printed raw: the float formatter is part of the parity.
        0 | 1 => {
            for _ in 0..rng.gen_range(1..=3) {
                let e = pick(&mut rng, MF_1ARG);
                stmts.push(format!("print -r -- \"$(( {e} ))\""));
            }
        }
        // Two-arg funcs, and a func inside a larger expression (so the
        // int/float contagion rule applies to the result).
        2 => {
            let e = pick(
                &mut rng,
                &[
                    "atan2(1,1)",
                    "fmod(7,3)",
                    "copysign(3,-1)",
                    "ldexp(1,4)",
                    "hypot(3,4)",
                    "atan(1,1)",
                    "1 + sqrt(9)",
                    "int(sqrt(2)) + 1",
                    "3 * ceil(0.1)",
                ],
            );
            stmts.push(format!("print -r -- \"$(( {e} ))\""));
            stmts.push(format!("integer i=$(( {e} )); print -r -- \"i=$i\""));
            stmts.push(format!("typeset -F 4 f=$(( {e} )); print -r -- \"f=$f\""));
        }
        // functions -M: a user math function, called from arithmetic.
        3 => {
            stmts.push("_cube() { (( REPLY = $1 * $1 * $1 )) }".to_string());
            stmts.push("functions -M cube 1 1 _cube".to_string());
            let n = rng.gen_range(0..6);
            stmts.push(format!("print -r -- \"$(( cube({n}) ))\""));
            stmts.push(format!("print -r -- \"$(( cube({n}) + cube(2) ))\""));
            // Wrong arity is an error, and must be reported the same way.
            stmts.push("print -r -- \"$(( cube(1,2) ))\"; print -r -- \"rc=$?\"".to_string());
        }
        // functions -M with a variable argument count and a string-arg variant.
        4 => {
            stmts.push("_sum() { local s=0 x; for x in \"$@\"; do (( s += x )); done; (( REPLY = s )) }".to_string());
            stmts.push("functions -M msum 1 4 _sum".to_string());
            stmts.push("print -r -- \"$(( msum(1) )) $(( msum(1,2) )) $(( msum(1,2,3,4) ))\"".to_string());
            stmts.push("_len() { (( REPLY = ${#1} )) }; functions -M slen 1 1 _len".to_string());
            stmts.push("print -r -- \"$(( slen(12345) ))\"".to_string());
            // `functions -M` with no body listed, and removal with +M.
            stmts.push("functions +M msum; print -r -- \"$(( msum(1,2) ))\"; print -r -- \"rc=$?\"".to_string());
        }
        // An undefined math function, and a mathfunc call before zmodload —
        // both are errors with a defined status, not a crash.
        _ => {
            stmts.push("print -r -- \"$(( nosuchfn(1) ))\"; print -r -- \"rc=$?\"".to_string());
            stmts.push("print -r -- \"$(( sqrt(-1) ))\"; print -r -- \"rc=$?\"".to_string());
            stmts.push("float x=$(( sqrt(2) )); print -r -- \"x=$x\"".to_string());
            stmts.push("typeset -F 6 y=$(( exp(1) )); print -r -- \"y=$y\"".to_string());
        }
    }
    stmts
}

// ---------------------------------------------------------------------------
// emulate generator
//
// `emulate sh|ksh|zsh` (Src/options.c bin_emulate) flips a whole BLOCK of
// options at once, and `emulate -L` scopes that to the enclosing function. The
// resulting option set is what decides word splitting, array base, `$0`, the
// `[[ ]]` vs `[` grammar, and the function-scope rules — so a single wrong
// option in the emulation table silently changes the meaning of every script
// that runs under it. Nothing else in the suite exercises the table.
// ---------------------------------------------------------------------------

fn gen_emulate(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let emu = pick(&mut rng, &["sh", "ksh", "zsh"]);
    let mut stmts: Vec<String> = Vec::new();

    match rng.gen_range(0..6) {
        // What the emulation actually SET: probe the options it is defined to
        // flip. This catches a wrong entry in the emulation table directly.
        0 => {
            stmts.push(format!("emulate {emu}"));
            for o in ["shwordsplit", "ksharrays", "globsubst", "bareglobqual", "rcexpandparam", "functionargzero", "nomatch", "banghist"] {
                stmts.push(format!("[[ -o {o} ]]; print -r -- \"{o}=$?\""));
            }
        }
        // The splitting/array semantics those options imply, observed directly.
        1 => {
            stmts.push(format!("emulate {emu}"));
            stmts.push("v='a b c'; arr=(x y z)".to_string());
            stmts.push("print -rl -- $v; print -r -- END".to_string());
            stmts.push(r#"print -r -- "[${arr[0]}] [${arr[1]}] n=${#arr}""#.to_string());
        }
        // `emulate -L` is LOCAL to the function: the option state must be
        // restored on return.
        2 => {
            stmts.push("setopt extendedglob".to_string());
            stmts.push(format!(
                "f() {{ emulate -L {emu}; [[ -o extendedglob ]]; print -r -- \"in=$?\"; [[ -o shwordsplit ]]; print -r -- \"split=$?\" }}"
            ));
            stmts.push("f; [[ -o extendedglob ]]; print -r -- \"out=$?\"".to_string());
            stmts.push("[[ -o shwordsplit ]]; print -r -- \"outsplit=$?\"".to_string());
        }
        // `emulate -c` / the `emulate sh -c 'script'` one-shot form.
        // NB: never probe `$0` here — its VALUE is the shell's own argv[0]
        // (a different path for zsh vs zshrs), which is a binary-name
        // artifact, not a parity gap. FUNCTION_ARGZERO is exercised via its
        // OPTION STATE below instead.
        3 => {
            let body = pick(
                &mut rng,
                &[
                    "v='a b'; print -rl -- $v",
                    "a=(1 2 3); print -r -- ${a[1]}",
                    "print -r -- ${#*}",
                    "x=5; print -r -- $((x*2))",
                ],
            );
            stmts.push(format!("emulate {emu} -c '{body}'"));
            stmts.push("print -r -- END".to_string());
        }
        // FUNCTION_ARGZERO: emulation flips it; probe the OPTION STATE (not the
        // value of $0, which is the binary's own path and differs by name).
        4 => {
            stmts.push(format!("emulate {emu}"));
            stmts.push("[[ -o functionargzero ]]; print -r -- \"faz=$?\"".to_string());
            stmts.push("f() { print -r -- \"nargs=$#\" }; f a b".to_string());
        }
        // Emulation + a construct whose parse depends on the option set.
        _ => {
            stmts.push(format!("emulate -L {emu}"));
            let probe = pick(
                &mut rng,
                &[
                    "[[ abc == a* ]]; print -r -- $?",
                    "[ abc = abc ]; print -r -- $?",
                    "print -r -- $(( 3 / 2 ))",
                    "x=5; print -r -- $((x++)) $x",
                    "print -r -- ${undefined-def}",
                    "typeset -A h=(k v); print -r -- $h[k]",
                ],
            );
            stmts.push(probe.to_string());
        }
    }
    stmts
}

// ---------------------------------------------------------------------------
// dirstack generator
//
// cd / pushd / popd / dirs and the directory stack (Src/builtins.c bin_cd,
// Src/hashnameddir.c) — `cd -`, `cd old new`, `pushd +N/-N`, PUSHD_MINUS,
// AUTO_PUSHD, PUSHD_IGNORE_DUPS, CDPATH, `~name` named directories, and the
// $PWD/$OLDPWD bookkeeping each of them must leave behind.
//
// SAFETY: the cleanup `rm` target is a LITERAL temp path that is NEVER
// reassigned and is guarded by a `*/pf_ds_*` glob, and the script `exit`s
// before any probe if the initial `cd` into the temp dir fails. This is
// deliberate: an earlier version derived the rm target from `$PWD` after a
// `cd`, so a `cd` bug in the shell-under-test (exactly what this mode hunts)
// redirected `rm -rf` at the fuzzer's own cwd. The rm target must never depend
// on shell-under-test behaviour.
// ---------------------------------------------------------------------------

fn gen_dirstack(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec![
        // `d` is the rm target: a literal, never reassigned from $PWD.
        format!("d=${{TMPDIR:-/tmp}}/pf_ds_{seed}"),
        "command rm -rf -- \"$d\"; command mkdir -p -- \"$d\"/a/b \"$d\"/c \"$d\"/d/e".to_string(),
        // If we cannot enter the fixture, bail BEFORE any probe or rm runs.
        "cd -- \"$d\" || exit 0".to_string(),
        // `base` is the RESOLVED cwd, used only to strip a machine path off
        // output — never as an rm target.
        "base=$PWD".to_string(),
        "p() { print -r -- \"${PWD#$base}|${OLDPWD#$base}\" }".to_string(),
    ];

    if rng.gen_bool(0.5) {
        let neg = if rng.gen_bool(0.3) { "un" } else { "" };
        stmts.push(format!(
            "{neg}setopt {}",
            pick(&mut rng, &["auto_pushd", "pushd_minus", "pushd_ignore_dups", "pushd_silent", "cdable_vars", "chase_links"])
        ));
    }

    for _ in 0..rng.gen_range(2..=5) {
        let stmt = match rng.gen_range(0..12) {
            // cd + the $OLDPWD bookkeeping, and `cd -` to return.
            0 => "cd a; p; cd -; p".to_string(),
            1 => "cd a/b; cd ../..; p".to_string(),
            // `cd old new` — string substitution on $PWD.
            2 => "cd a/b; cd b e 2>/dev/null; print -r -- \"rc=$? ${PWD#$base}\"".to_string(),
            // pushd/popd and the stack listing (relative, so it is portable).
            3 => "pushd a >/dev/null; pushd c >/dev/null; print -rl -- ${${(f)\"$(dirs -p)\"}#$base}; popd >/dev/null; p".to_string(),
            4 => "pushd a >/dev/null; pushd d >/dev/null; pushd +1 >/dev/null; p; print -r -- \"n=${#dirstack}\"".to_string(),
            5 => "pushd a >/dev/null; pushd c >/dev/null; popd +1 >/dev/null; print -r -- \"n=${#dirstack} ${PWD#$base}\"".to_string(),
            // pushd with no args swaps the top two entries.
            6 => "pushd a >/dev/null; pushd >/dev/null; p".to_string(),
            // `dirs -v` numbering, and the $dirstack array itself.
            7 => "pushd a >/dev/null; pushd c >/dev/null; print -rl -- ${dirstack#$base}; print -r -- END".to_string(),
            // popd on an empty stack is an error with a defined status.
            8 => "popd; print -r -- \"rc=$?\"".to_string(),
            // CDPATH: a bare `cd b` finds $d/a/b through the search path.
            9 => "CDPATH=$base/a; cd -- \"$base\"; cd b; print -r -- \"${PWD#$base}\"; CDPATH=".to_string(),
            // A named directory: ~name expands to its value, and %~ / ${(D)}
            // render a path back through the named-directory table.
            10 => "hash -d nd=$base/d/e; cd ~nd; print -r -- \"${PWD#$base}\"; print -r -- \"${(D)PWD}\"".to_string(),
            // cd to a nonexistent directory must fail without moving.
            _ => "cd nosuchdir 2>/dev/null; print -r -- \"rc=$? ${PWD#$base}\"".to_string(),
        };
        stmts.push(stmt);
    }
    // Cleanup: leave the fixture, then rm the LITERAL guarded target only.
    stmts.push("cd /".to_string());
    stmts.push("case $d in (*/pf_ds_*) command rm -rf -- \"$d\";; esac".to_string());
    stmts
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
    Subscript,
    Pattern,
    Typeset,
    Zutil,
    Func,
    Redir,
    Nest,
    Arith,
    Match,
    Regex,
    Builtin,
    Cmdsub,
    Loop,
    Split,
    Trap,
    Pipeline,
    Prompt,
    Modifier,
    Mathfunc,
    Emulate,
    Dirstack,
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
        Mode::Subscript => gen_subscript(seed),
        Mode::Pattern => gen_pattern(seed),
        Mode::Typeset => gen_typeset(seed),
        Mode::Zutil => gen_zutil(seed),
        Mode::Func => gen_func(seed),
        Mode::Redir => gen_redir(seed),
        Mode::Nest => gen_nest(seed),
        Mode::Arith => gen_arith_mode(seed),
        Mode::Match => gen_match(seed),
        Mode::Regex => gen_regex(seed),
        Mode::Builtin => gen_builtin(seed),
        Mode::Cmdsub => gen_cmdsub(seed),
        Mode::Loop => gen_loop(seed),
        Mode::Split => gen_split_mode(seed),
        Mode::Trap => gen_trap(seed),
        Mode::Pipeline => gen_pipeline(seed),
        Mode::Prompt => gen_prompt(seed),
        Mode::Modifier => gen_modifier(seed),
        Mode::Mathfunc => gen_mathfunc(seed),
        Mode::Emulate => gen_emulate(seed),
        Mode::Dirstack => gen_dirstack(seed),
    }
}

/// Mode → the name accepted by `--mode` (and printed by `--once`).
fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Stateful => "stateful",
        Mode::Expr => "expr",
        Mode::Glob => "glob",
        Mode::Printf => "printf",
        Mode::Heredoc => "heredoc",
        Mode::Subscript => "subscript",
        Mode::Pattern => "pattern",
        Mode::Typeset => "typeset",
        Mode::Zutil => "zutil",
        Mode::Func => "func",
        Mode::Redir => "redir",
        Mode::Nest => "nest",
        Mode::Arith => "arith",
        Mode::Match => "match",
        Mode::Regex => "regex",
        Mode::Builtin => "builtin",
        Mode::Cmdsub => "cmdsub",
        Mode::Loop => "loop",
        Mode::Split => "split",
        Mode::Trap => "trap",
        Mode::Pipeline => "pipeline",
        Mode::Prompt => "prompt",
        Mode::Modifier => "modifier",
        Mode::Mathfunc => "mathfunc",
        Mode::Emulate => "emulate",
        Mode::Dirstack => "dirstack",
    }
}

/// Parse a `--mode` value. Returns None for an unknown name.
fn mode_from_name(s: &str) -> Option<Mode> {
    const ALL: &[Mode] = &[
        Mode::Stateful,
        Mode::Expr,
        Mode::Glob,
        Mode::Printf,
        Mode::Heredoc,
        Mode::Subscript,
        Mode::Pattern,
        Mode::Typeset,
        Mode::Zutil,
        Mode::Func,
        Mode::Redir,
        Mode::Nest,
        Mode::Arith,
        Mode::Match,
        Mode::Regex,
        Mode::Builtin,
        Mode::Cmdsub,
        Mode::Loop,
        Mode::Split,
        Mode::Trap,
        Mode::Pipeline,
        Mode::Prompt,
        Mode::Modifier,
        Mode::Mathfunc,
        Mode::Emulate,
        Mode::Dirstack,
    ];
    ALL.iter().copied().find(|&m| mode_name(m) == s)
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
                match argv.get(i).and_then(|s| mode_from_name(s)) {
                    Some(m) => mode = m,
                    None => {
                        eprintln!("unknown --mode '{}'", argv.get(i).map(|s| s.as_str()).unwrap_or(""));
                        std::process::exit(2);
                    }
                }
            }
            // `--<mode>` shorthand for every mode name (`--expr`, `--arith`, …).
            a if a.starts_with("--") && mode_from_name(&a[2..]).is_some() => {
                mode = mode_from_name(&a[2..]).unwrap();
            }
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
            "--stderr" => {
                CMP_STDERR.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            "--help" | "-h" => {
                eprintln!(
                    "parity-fuzz — differential zsh/zshrs parity fuzzer\n\
                     \n\
                     --count N        number of cases (default 2000)\n\
                     --seed N         base seed; case i uses seed+i (default 1)\n\
                     --mode M         stateful (default), expr, glob, printf, heredoc,\n\
                     subscript, pattern, typeset, zutil, func, redir,\n\
                     nest, arith, match, regex, builtin, cmdsub, loop,\n\
                     split, trap, pipeline, prompt, modifier, mathfunc,\n\
                     emulate, dirstack\n\
                     (each also accepted as a `--<mode>` shorthand)\n\
                     --stderr         also require the DIAGNOSTICS to match (the\n\
                                      leading `zsh:`/`zshrs:` tag is normalized\n\
                                      away; the wording after it must agree)\n\
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
        let diverged = !z.timed_out && differs(&z, &r);
        println!("seed   : {}", args.base_seed);
        println!("mode   : {}", mode_name(args.mode));
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
                    if !z.timed_out && differs(&z, &r) {
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
                        let mut confirmed = differs(&mz, &mr);
                        for _ in 1..args.verify.max(1) {
                            if !confirmed {
                                break;
                            }
                            confirmed = diverges(&mscript, &bin, timeout);
                        }
                        if !confirmed {
                            continue; // flaky/transient — not a real gap
                        }
                        // Under --stderr the diagnostics are part of the compared
                        // output, so the record has to show them or the report is
                        // unreadable (identical stdout, invisible difference).
                        let err_of = |o: &RunOut| -> String {
                            if CMP_STDERR.load(Ordering::Relaxed) {
                                format!("\n  stderr: {}", norm_stderr(o.stderr.trim_end()).replace('\n', "\n  "))
                            } else {
                                String::new()
                            }
                        };
                        let rec = format!(
                            "==== seed {seed} ====\n\
                             program:\n  {}\n\
                             zsh   : exit={} timeout={}{}\n{}\n\
                             zshrs : exit={} timeout={}{}\n{}\n",
                            mscript.replace('\n', "\n  "),
                            mz.exit,
                            mz.timed_out,
                            err_of(&mz),
                            mz.stdout.trim_end_matches('\n'),
                            mr.exit,
                            mr.timed_out,
                            err_of(&mr),
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
