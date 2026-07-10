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

fn run_zsh(script: &str, timeout: Duration) -> RunOut {
    let mut cmd = Command::new(zsh_path());
    cmd.args(["-f", "-c", script]);
    run_with_timeout(cmd, timeout)
}

fn run_zshrs(script: &str, bin: &Path, timeout: Duration) -> RunOut {
    let mut cmd = Command::new(bin);
    cmd.args(["--zsh", "-f", "-c", script]).env_remove("ZSHRS_CACHE");
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
    "nums=(3 1 4 1 5 9 2 6); ",
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

/// Generate the raw expression list for a seed (before script assembly).
fn gen_parts(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let n = rng.gen_range(1..=3);
    let mut parts: Vec<String> = Vec::with_capacity(n);
    for _ in 0..n {
        let expr = match rng.gen_range(0..10) {
            0 => gen_scalar_pe(&mut rng),
            1 => gen_array_pe(&mut rng),
            2 => gen_assoc_pe(&mut rng),
            3 => format!("$(( {} ))", gen_arith(&mut rng, 3)),
            4 => gen_padding(&mut rng),
            5 => gen_split(&mut rng),
            6 => gen_modchain(&mut rng),
            7 => gen_nested(&mut rng),
            8 => gen_scalar_pe(&mut rng),
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

/// An observation statement — always emits to stdout, deterministically.
/// `${(t)var}` is weighted in because it reports a parameter's full type +
/// attribute set (e.g. `integer-local`, `array-unique`, `scalar-readonly`),
/// which is the most direct probe of whether typeset state was modelled right.
fn gen_observation(rng: &mut StdRng) -> String {
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
fn gen_program(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = base_state();
    let steps = rng.gen_range(3..=8);
    for _ in 0..steps {
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
// Main driver
// ---------------------------------------------------------------------------

struct Args {
    count: u64,
    base_seed: u64,
    once: bool,
    timeout_ms: u64,
    out_path: PathBuf,
    max_report: usize,
    jobs: usize,
    stateful: bool,
}

/// Generate the statement list for a seed in the selected mode.
fn gen_case(seed: u64, stateful: bool) -> Vec<String> {
    if stateful {
        gen_program(seed)
    } else {
        expr_program(seed)
    }
}

fn parse_args() -> Args {
    let mut count = 2000u64;
    let mut base_seed = 1u64;
    let mut once = false;
    let mut timeout_ms = 5000u64;
    let mut max_report = 200usize;
    let mut stateful = true;
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
                    Some("expr") => stateful = false,
                    Some("stateful") => stateful = true,
                    _ => {}
                }
            }
            "--expr" => stateful = false,
            "--help" | "-h" => {
                eprintln!(
                    "parity-fuzz — differential zsh/zshrs parity fuzzer\n\
                     \n\
                     --count N        number of cases (default 2000)\n\
                     --seed N         base seed; case i uses seed+i (default 1)\n\
                     --mode M         'stateful' (default) or 'expr'\n\
                     --once           run a single case (seed) and print both outputs\n\
                     --timeout-ms N   per-shell wall-clock timeout (default 5000)\n\
                     --out PATH       divergence corpus file\n\
                     --max-report N   stop after N divergences (default 200)\n\
                     --jobs N         parallel workers (default = CPU count)\n\
                     \n\
                     stateful mode builds a sequence of setopt/typeset/IFS/scope\n\
                     mutations interleaved with probes, then delta-debugs each\n\
                     divergence to the minimal state + observation that repros."
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
        stateful,
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

    // --once: replay a single seed, minimize if it diverges, dump both sides.
    if args.once {
        let stmts = gen_case(args.base_seed, args.stateful);
        let script = build_program(&stmts);
        let z = run_zsh(&script, timeout);
        let r = run_zshrs(&script, &bin, timeout);
        let diverged = !z.timed_out && (z.stdout != r.stdout || z.exit != r.exit);
        println!("seed   : {}", args.base_seed);
        println!("mode   : {}", if args.stateful { "stateful" } else { "expr" });
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
                    let stmts = gen_case(seed, args.stateful);
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
    println!(
        "\nfuzzed {checked} cases in {:.1}s ({:.0}/s)\n\
         divergences : {}\n\
         timeouts    : {}",
        elapsed.as_secs_f64(),
        checked as f64 / elapsed.as_secs_f64().max(0.001),
        divergences.len(),
        timeouts,
    );

    if !divergences.is_empty() {
        if let Some(parent) = args.out_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::File::create(&args.out_path) {
            Ok(mut f) => {
                for d in &divergences {
                    let _ = writeln!(f, "{d}");
                }
                println!("wrote {} divergences to {}", divergences.len(), args.out_path.display());
            }
            Err(e) => eprintln!("could not write {}: {e}", args.out_path.display()),
        }
        // Print the first few inline for immediate triage.
        println!("\n--- first {} divergence(s) ---", divergences.len().min(5));
        for d in divergences.iter().take(5) {
            println!("{d}");
        }
        std::process::exit(1);
    }
}
