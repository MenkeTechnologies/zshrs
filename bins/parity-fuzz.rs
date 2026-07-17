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

/// The ORACLE. Every divergence this harness reports is "zshrs disagrees with
/// THIS binary", so which binary it is, is part of the result — a baseline is
/// only meaningful against the zsh that produced it.
///
/// That is not a hypothetical. Three different zsh builds are in play:
///   - local (macOS)  : Homebrew, `zsh-5.9.2-0-gddee3e7`
///   - CI (ubuntu)    : whatever `apt-get install zsh` resolves to
///   - the C spec     : ~/forkedRepos/zsh, upstream master past `zsh-5.9.0.2-test`
/// and they are NOT the same code. 5.9.2's commit `ddee3e7` is not even in the
/// fork's object database, and the fork carries changes 5.9.2 lacks — e.g.
/// upstream 61f35bb626 made `sysopen` on a missing file `return 2` where 5.9.2
/// still returns 1 (c:Src/Modules/system.c:388-391). Porting faithfully from
/// the fork therefore *creates* a divergence against the Homebrew oracle, and
/// the reverse "fix" would break the port. Silently picking whichever zsh
/// happens to be installed is how that becomes an infinite chase.
///
/// So: `ZSHRS_FUZZ_ZSH` names the oracle explicitly (point it at a zsh built
/// from the fork to make spec and oracle the same code). If it is set but
/// unusable this is a HARD ERROR — falling back to a different zsh would
/// silently answer a different question than the one that was asked.
fn zsh_path() -> &'static str {
    static ORACLE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ORACLE.get_or_init(|| {
        if let Ok(p) = std::env::var("ZSHRS_FUZZ_ZSH") {
            if !Path::new(&p).exists() {
                eprintln!("parity-fuzz: ZSHRS_FUZZ_ZSH={p}: no such file");
                std::process::exit(2);
            }
            return p;
        }
        for p in ["/opt/homebrew/bin/zsh", "/usr/local/bin/zsh"] {
            if Path::new(p).exists() {
                return p.to_string();
            }
        }
        "/bin/zsh".to_string()
    })
}

/// `<path> (<$ZSH_PATCHLEVEL>)`, for the run header and the report file, so a
/// divergence record can be attributed to the exact oracle that produced it.
fn zsh_oracle_id() -> String {
    let path = zsh_path();
    let level = Command::new(path)
        .args(["-fc", "print -rn -- $ZSH_PATCHLEVEL"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    format!("{path} ({level})")
}

/// Raw bytes, never `String`: a shell legitimately emits output that is not
/// valid UTF-8 (`$'\M-a'`, `printf '\xff'`, an 8-bit locale). `read_to_string`
/// FAILS on such a stream and leaves the buffer empty, so both shells would
/// report "" and silently agree — a divergence the harness could never see.
/// Comparing bytes (and only ever lossy-rendering for the human-facing report)
/// keeps the 8-bit surface honest.
struct RunOut {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit: i32,
    timed_out: bool,
}

/// Render captured bytes for a report. Invalid UTF-8 is shown lossily AND
/// followed by a hex line — two different invalid byte strings both render to
/// U+FFFD, so without the hex the record would show a divergence as identical
/// text.
fn render(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_end_matches('\n');
    if std::str::from_utf8(bytes).is_err() {
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        return format!("{text}\n  (hex) {}", hex.join(" "));
    }
    text.to_string()
}

/// --stderr: also require the two shells' DIAGNOSTICS to agree, not just stdout
/// and exit status. Off by default: a message-text mismatch is a much softer gap
/// than a wrong value, and mixing the two would drown the hard gaps.
static CMP_STDERR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The two shells necessarily disagree on their own name in a diagnostic
/// (`zsh: no such file` vs `zshrs: no such file`), which is not a parity gap.
/// Normalize the leading shell-name tag off every line; everything after it —
/// the wording, the offending word, the line number — must match exactly.
fn norm_stderr(s: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(s.len());
    for (i, line) in s.split(|&b| b == b'\n').enumerate() {
        if i > 0 {
            out.push(b'\n');
        }
        let l = line
            .strip_prefix(b"zshrs:".as_slice())
            .or_else(|| line.strip_prefix(b"zsh:".as_slice()))
            .unwrap_or(line);
        out.extend_from_slice(l);
    }
    out
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
                stdout: Vec::new(),
                stderr: Vec::new(),
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
                let mut buf: Vec<u8> = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_end(&mut buf);
                }
                let mut ebuf: Vec<u8> = Vec::new();
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_end(&mut ebuf);
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
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                        exit: -1,
                        timed_out: true,
                    };
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => {
                return RunOut {
                    stdout: Vec::new(),
                    stderr: Vec::new(),
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

/// Create (idempotently) the autoload fixture: a directory whose `fns/` holds
/// one file per autoloadable function. Under zsh's default (non-KSH) autoload
/// the file's TEXT IS THE BODY; under KSH_AUTOLOAD the file is expected to
/// DEFINE the function. `af_ksh` is written to satisfy the ksh contract so the
/// two loading styles produce visibly different results from the same file.
fn setup_autoload_fixture() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("parity-fuzz")
        .join("autoload-fixture");
    let _ = std::fs::remove_dir_all(&dir);
    let fns = dir.join("fns");
    std::fs::create_dir_all(&fns).expect("create autoload fixture dir");

    let files: &[(&str, &str)] = &[
        ("af_plain", "print -r -- plain\n"),
        ("af_args", "print -r -- \"args=$* n=$#\"\n"),
        ("af_zero", "print -r -- \"zero=$0\"\n"),
        // ksh-style: defines the function; KSH_AUTOLOAD then calls it. Under the
        // default style, running the body merely REDEFINES af_ksh and prints
        // nothing — that difference is the probe.
        ("af_ksh", "af_ksh() { print -r -- \"ksh $1\" }\n"),
        // Body calls `helper`, which the caller has defined BOTH as a function
        // and as an alias: -U picks the function, no -U picks the alias.
        ("af_alias", "helper arg\n"),
        // One file, several functions: the named one is the body, the rest are
        // defined as a side effect of running it.
        (
            "af_multi",
            "af_extra() { print -r -- extra }\nprint -r -- multi\naf_extra\n",
        ),
    ];
    for (name, body) in files {
        std::fs::write(fns.join(name), body).unwrap();
    }
    dir
}

/// An empty, disposable directory to run modes that can WRITE files.
///
/// alias mode probes a global alias in redirect position
/// (`alias -g N=/dev/null; print -r -- gone > N`). That is safe as written — but
/// the delta-minimizer's whole job is to drop statements, and dropping the alias
/// line leaves `print -r -- gone > N`, which creates a literal file called `N`
/// in the cwd. Without this fixture that file lands in the REPO ROOT. Any mode
/// whose probes can redirect must run from here, not from the source tree.
fn setup_scratch_fixture() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("parity-fuzz")
        .join("scratch");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch fixture dir");
    dir
}

/// Fixed fileset for the zmv mode. Names are chosen so the pattern surface has
/// something to bite on: two extensions that a `(txt|log)` alternation must
/// BOTH pick up, a name with a space (the `${f// /_}` case from zsh's own zmv
/// docs), multi-dot and digit names for multi-wildcard patterns, and a
/// subdirectory so `(**/)` has somewhere to recurse.
///
/// READ-ONLY at run time: every generated probe passes `-n`, so zmv only ever
/// PRINTS the mv/cp/ln commands it would run. That keeps the fixture shareable
/// across the parallel workers and means a minimized probe can never rename a
/// file out from under the other workers (or into the source tree).
fn setup_zmv_fixture() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("parity-fuzz")
        .join("zmv-fixture");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create zmv fixture dir");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    for name in [
        "foo.txt",
        "bar.txt",
        "baz.log",
        "one.two.txt",
        "a b.txt",
        "f1.dat",
        "f2.dat",
        "UP.TXT",
        "sub/nested.txt",
    ] {
        std::fs::write(dir.join(name), "x").unwrap();
    }
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

/// Default / alternate word operators composed onto subscripted values. The
/// colon forms treat an empty match as unset (subst.c:3187), the bare forms
/// only fire on a truly unset slot. `word` is metachar-free so it splices
/// verbatim.
const DEF_OPS: &[&str] = &[":-D", ":+A", "-D", "+A", ":-", ":+", "-", "+"];

fn gen_subscript(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec![SUB_STATE.trim_end().to_string()];
    let n = rng.gen_range(2..=5);
    for _ in 0..n {
        let arr = pick(&mut rng, &["a", "nums", "dup"]);
        let expr = match rng.gen_range(0..13) {
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
            // Default-operator on a flag-form subscript: `${a[(r)pat]:-x}`.
            // A no-match makes the subscripted value vunset, so `:-`/`:=`/
            // `:?` must apply the default and `:+` must NOT. Composing a
            // search subscript with a default was previously ungenerated —
            // it is exactly where an assoc `(r)` no-match wrongly returned
            // "" instead of the default (fixed in subst.rs).
            8 => {
                let op = pick(&mut rng, DEF_OPS);
                let f = pick(&mut rng, &["r", "R", "i", "I"]);
                let p = pick(&mut rng, SUB_PATS);
                format!("${{{arr}[({f}){p}]{op}}}")
            }
            // Default-operator on assoc key / value-search subscripts,
            // including missing keys and no-match value searches.
            9 => {
                let op = pick(&mut rng, DEF_OPS);
                match rng.gen_range(0..4) {
                    0 => format!("${{m[{}]{op}}}", pick(&mut rng, &["k1", "k2", "nokey", "zz"])),
                    1 => format!("${{m[(r){}]{op}}}", pick(&mut rng, &["v1", "v9", "v*", "z*"])),
                    2 => format!("${{m[(i){}]{op}}}", pick(&mut rng, &["k1", "k*", "z*"])),
                    _ => format!("${{(k)m[(R){}]{op}}}", pick(&mut rng, &["v1", "v9", "z*"])),
                }
            }
            // Nested subscript whose inner default feeds the outer key —
            // the exact `${_comps[${_services[(r)$svc]:-$svc}]}` compdef
            // shape that the service-form lookup depends on.
            10 => {
                let inner = pick(&mut rng, &[":-k1", ":-nokey", ":-k2", ":-v1"]);
                let outer = pick(&mut rng, DEF_OPS);
                let vp = pick(&mut rng, &["v1", "v9", "z*", "v*"]);
                format!("${{m[${{m[(r){vp}]{inner}}}]{outer}}}")
            }
            // `:=` / `:?` on a plain key/index (assignment default and the
            // error-default), where the assign target is a real lvalue.
            11 => {
                match rng.gen_range(0..4) {
                    0 => format!("${{m[nokey]:=NEWVAL}}"),
                    1 => format!("${{m[k1]:=NEWVAL}}"),
                    2 => format!("${{{arr}[3]:=Z}}"),
                    _ => format!("${{m[nokey]:?}}"),
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
        match rng.gen_range(0..9) {
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
            7 => {
                let e = ar_expr(&mut rng, 1);
                stmts.push(format!(
                    "arr=(a b c d e); print -r -- \"[${{arr[(({e}) % 5) + 1]}}]\""
                ));
            }
            // Arithmetic in a RANGE subscript. A distinct caller from the
            // single-index arm above (c:Src/params.c getindex parses two
            // bounds), and the bounds are where a PARENTHESISED expression has
            // to survive: c:1389 only reads `(…)` as a flag group while the
            // chars are real flags, and c:1477-1482's `flagerr` REWINDS to
            // before the `(` on anything else, so the group is re-read as math.
            // `${arr[(x), 4]}` is `b c d`, not the whole array — the paren form
            // must agree with the bare `${arr[x, 4]}`.
            _ => {
                let lo = pick(&mut rng, &["1", "2", "x", "(x)", "(x+1)", "(1+1)", "-3", "(i)"]);
                let hi = pick(&mut rng, &["3", "4", "(y)", "(y-1)", "(2+2)", "-1", "5", "(j)"]);
                stmts.push(format!(
                    "arr=(a b c d e); x=2; y=4; i=1; j=3; print -rl -- ${{arr[{lo}, {hi}]}}; print -r -- \"n=${{#${{arr[{lo}, {hi}]}}}}\""
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
    // NEGATIVE (minus) widths. c:Src/prompt.c:663-672 reads `-N` on `<`/`>`
    // as "truncate to N columns BEFORE the right margin", not as a literal
    // width: `arg = zterm_columns - t0 + arg`, then c:670-671 clamps a
    // non-positive result to 1. The `-` is parsed for EVERY escape (c:374-382,
    // where a bare `-` with no digits means arg = -1), but only these atoms
    // make the value observable.
    //
    // The mode had truncation atoms yet none with a minus, so it ran clean
    // while `%-5<<abc` printed `abc` where zsh prints `c`. The width also has
    // to be reachable from BOTH sides of the margin: `%-90<<` against a
    // 97-column terminal must leave 7 columns, while any of these against a
    // shell with no tty (zterm_columns == 0) must collapse to the 1-column
    // clamp — the two cases that a hardcoded 80-column fallback silently
    // papered over.
    "%-5<<abcdefghij",
    "%-0<<abcdef",
    "%-1<<abcdef",
    "%-5>>abcdefghij",
    "%-<<abcdef",
    "%-3<...<abcdefghij",
    "%-90<<abcdefghijklmnopqrstuvwxyz",
    "XY%-5<<abcdef",
    "%-10(l.wide.narrow)",
    "%(-5l.wide.narrow)",
    "%(0l.wide.narrow)",
    // Escapes the vocabulary never reached. All are deterministic across the
    // two shells under the harness's identical `-fc` invocation and cwd;
    // time/history escapes (%D %T %t %* %@ %W %w %h %!) are deliberately
    // EXCLUDED because they are not.
    "%L",
    "%i",
    "%e",
    "%?",
    "%_",
    "%^",
    "%G",
    "%C",
    "%1~",
    "%N",
    "%F{300}oob%f",
    "%F{#ff8800}hex%f",
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
    // `:c` — PATH search (c:Src/hist.c:863). It was absent from this list, and
    // that is exactly why the fuzzer never noticed it was unimplemented in the
    // UNBRACED form: every probe below used `${f:mod}`, which worked.
    ":c", ":c:t", ":c:h",
];

/// Values for the `:c` PATH-search modifier: a name that resolves, and one that
/// does not (an unresolvable name is left UNCHANGED, not emptied).
const MOD_CMDS: &[&str] = &["ls", "sh", "no_such_command_xyz", "a:b"];

fn gen_modifier(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();

    match rng.gen_range(0..8) {
        // Scalar + a modifier chain.
        0 | 1 => {
            for _ in 0..rng.gen_range(1..=3) {
                let p = pick(&mut rng, MOD_PATHS);
                let m = pick(&mut rng, MODS);
                stmts.push(format!("f='{p}'; print -r -- \"[${{f{m}}}]\""));
            }
        }
        // The UNBRACED form `$f:mod`. zsh applies the whole modifier set to a
        // bare parameter too, and this arm is the one that was missing: every
        // other probe here braces the expansion, so an unbraced-only gap (`$f:c`
        // left as literal text) was invisible.
        6 => {
            // Unquoted `$f:mod`. `&` and `|` are shell metachars bare (`&`
            // backgrounds the command → output-ordering race; `|` pipes), so the
            // UNQUOTED probe uses only metachar-free modifiers. The quoted arm
            // above still exercises `&`/`|` forms.
            let safe: Vec<&&str> = MODS.iter().filter(|m| !m.contains('&') && !m.contains('|')).collect();
            for _ in 0..rng.gen_range(1..=3) {
                let p = pick(&mut rng, MOD_PATHS);
                let m = **pick(&mut rng, &safe);
                stmts.push(format!("f='{p}'; print -r -- $f{m}"));
                stmts.push(format!("f='{p}'; print -r -- \"$f{m}\""));
            }
        }
        // `:c` resolves a command name through $PATH; an unresolvable name is
        // left alone. Probed both braced and unbraced, and in an assignment RHS
        // (where the modifier still applies).
        7 => {
            let c = pick(&mut rng, MOD_CMDS);
            stmts.push(format!("v='{c}'; print -r -- \"[${{v:c}}]\""));
            stmts.push(format!("v='{c}'; print -r -- \"[$v:c]\""));
            stmts.push(format!("v='{c}'; w=$v:c; print -r -- \"[$w]\""));
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
// unicode generator
//
// Multibyte text is the single largest hole in the rest of the corpus: every
// other mode is pure ASCII, so nothing pins the character-vs-byte distinction.
// zsh counts CHARACTERS (not bytes) for ${#ts} and subscripts, but pads by
// DISPLAY WIDTH under the (m) flag — a wide CJK char counts 1 for ${#} and 2
// for (ml:N:). C: Src/utils.c mb_metastrlen()/MB_METASTRWIDTH, Src/subst.c.
//
// Deterministic: fixed literals, no locale-dependent collation (no (o)/(O) on
// non-ASCII — strcoll order is a libc property, not a parity property).
// ---------------------------------------------------------------------------

const UNI_STATE: &str = concat!(
    "u=héllo_wörld; ",           // Latin-1 supplement: 2 bytes/char
    "j=日本語テキスト; ",         // CJK: 3 bytes/char, display width 2
    "gr=αβγδε; ",                // Greek: 2 bytes/char
    "cy=Привет; ",               // Cyrillic: 2 bytes/char
    "acc=$'e\\u0301'; ",         // e + COMBINING ACUTE — 2 chars, 3 bytes, 1 glyph
    "mix='aé漢z'; ",             // 1+2+3+1 bytes across four chars
    "wide='一二三'; ",            // all-wide, for (m) width padding
    "uarr=(é ö 漢 z a); ",
);

fn gen_unicode(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let vars = ["u", "j", "gr", "cy", "acc", "mix", "wide"];
    let mut stmts = vec![UNI_STATE.trim_end().to_string()];
    // MULTIBYTE is default-on; flipping it off switches every length/index
    // operation to raw bytes — the same probe must then agree byte-for-byte.
    if rng.gen_bool(0.2) {
        stmts.push("unsetopt multibyte".to_string());
    }
    for _ in 0..rng.gen_range(2..=4) {
        let v = *pick(&mut rng, &vars);
        let stmt = match rng.gen_range(0..16) {
            // Character count vs byte count: ${#v} is chars, $#v under
            // nomultibyte is bytes.
            0 => format!("print -r -- \"len=${{#{v}}}\""),
            // Character subscripting — indexes are character offsets.
            1 => format!("print -r -- \"[${{{v}[2]}}][${{{v}[2,3]}}][${{{v}[-1]}}]\""),
            2 => format!("print -r -- \"[${{{v}[1,-2]}}]\""),
            // Case conversion over non-ASCII (Greek/Cyrillic have real case).
            3 => format!("print -r -- \"${{({0}){v}}}\"", pick(&mut rng, &["U", "L", "C"])),
            // Padding: (l)/(r) count CHARACTERS; with (m) they count WIDTH.
            4 => format!("print -r -- \"[${{(l:8::.:){v}}}]\""),
            5 => format!("print -r -- \"[${{(r:8::.:){v}}}]\""),
            6 => format!("print -r -- \"[${{(ml:8::.:){v}}}]\""),
            // Splitting into characters: (s::) with an empty separator.
            7 => format!("print -rl -- ${{(s::){v}}}; print -r -- END"),
            // (#) — arithmetic value → character. `##x` is the codepoint of x.
            8 => format!("print -r -- \"${{(#)$(( ##${{{v}[1]}} ))}}\""),
            9 => format!("print -r -- \"cp=$(( ##${{{v}[1]}} ))\""),
            // Pattern matching: `?` is one CHARACTER, not one byte.
            10 => format!("[[ ${v} = ? ]] && print -r -- one || print -r -- many"),
            11 => format!("[[ ${v} = ??* ]] && print -r -- Y || print -r -- N"),
            // Character classes over non-ASCII.
            12 => format!("print -r -- \"${{{v}//[[:alpha:]]/.}}\""),
            // Quoting a multibyte string must not split a character.
            13 => format!("print -r -- \"${{(q){v}}}|${{(qq){v}}}\""),
            // (V) makes unprintables visible — must not mangle valid multibyte.
            14 => format!("print -r -- \"${{(V){v}}}\""),
            // Substitution with a multibyte needle and replacement.
            _ => format!("print -r -- \"${{{v}/[[:alpha:]]/漢}}\""),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// quote generator
//
// Two halves of one round trip:
//   * `$'…'` DECODES escapes (C: Src/utils.c getkeystring()) — \x41, \101,
//     é, \U0001F600, \M-a (meta: sets bit 7), \C-a (control), \e, \a…
//   * the (q…) family ENCODES a string back to a re-parseable form (C:
//     Src/subst.c quotestring(), QT_* modes): (q) backslash, (qq) single,
//     (qqq) double, (qqqq) $'…', (q-) minimal, (q+) $'…' only when needed.
// `${(Q)${(q…)v}}` must be the identity for every v — that is the invariant.
//
// The RunOut byte-comparison above is what makes this mode possible at all:
// \M-a emits a bare 0xE1 byte, which is not valid UTF-8.
// ---------------------------------------------------------------------------

/// Strings chosen so every quoting mode has something to escape: spaces, both
/// quote characters, glob metachars, `!`, `$`, backslash, tab/newline, and a
/// leading `-` / `~` (which only (q-)/(q+) treat specially).
const QUOTE_SUBJECTS: &[&str] = &[
    "plain",
    "a b",
    "a'b",
    r#"a"b"#,
    "a\\$b",
    "a*b?c[d]",
    "a!b",
    "-lead",
    "~home",
    "a#b",
    "",
    "a=b",
];

/// `$'…'` escape bodies. Every one has a single, defined byte expansion.
const QUOTE_ESCAPES: &[&str] = &[
    r"\x41\x42",
    r"\101\102",
    r"é",
    r"日本",
    r"\U0001F600",
    r"\a\b\f\v",
    r"\e[1m",
    r"\t|\n|",
    r"\\|\'",
    r"\M-a",
    r"\M-\C-a",
    r"\C-a\C-z",
    r"\x7f",
    r"\0101",
];

fn gen_quote(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(2..=4) {
        let stmt = match rng.gen_range(0..10) {
            // $'…' decoding, byte-exact (piped through od so an invalid-UTF-8
            // result is still a comparable, printable byte sequence — and the
            // raw form is printed too, since the harness compares bytes).
            0 => {
                let e = pick(&mut rng, QUOTE_ESCAPES);
                format!("print -rn -- $'{e}' | od -An -tx1 | tr -s ' '")
            }
            1 => {
                let e = pick(&mut rng, QUOTE_ESCAPES);
                format!("v=$'{e}'; print -r -- \"len=${{#v}}\"")
            }
            2 => {
                let e = pick(&mut rng, QUOTE_ESCAPES);
                format!("v=$'{e}'; print -r -- \"[${{(V)v}}]\"")
            }
            // (q…) encoding of a subject with something to escape.
            3 => {
                let s = pick(&mut rng, QUOTE_SUBJECTS);
                let f = pick(&mut rng, &["q", "qq", "qqq", "qqqq", "q-", "q+"]);
                format!("v='{}'; print -r -- \"[${{({f})v}}]\"", s.replace('\'', "'\\''"))
            }
            // The round-trip invariant: (Q) undoes every (q…) form exactly.
            4 => {
                let s = pick(&mut rng, QUOTE_SUBJECTS);
                let f = pick(&mut rng, &["q", "qq", "qqq", "qqqq", "q-", "q+"]);
                format!(
                    "v='{}'; r=${{(Q)${{({f})v}}}}; [[ $r == $v ]] && print -r -- ok || print -r -- \"BAD[$r]\"",
                    s.replace('\'', "'\\''")
                )
            }
            // Quoting a value that itself contains decoded escapes.
            5 => {
                let e = pick(&mut rng, QUOTE_ESCAPES);
                let f = pick(&mut rng, &["q", "qq", "qqqq", "q+"]);
                format!("v=$'{e}'; print -r -- \"[${{({f})v}}]\" | od -An -tx1 | tr -s ' '")
            }
            // printf %q — the builtin path into the same quoting code.
            6 => {
                let s = pick(&mut rng, QUOTE_SUBJECTS);
                format!("printf '%q\\n' '{}'", s.replace('\'', "'\\''"))
            }
            // (q) applied element-wise across an array.
            7 => "arr=('a b' \"c'd\" 'e*f' '' '-g'); print -r -- \"${(q)arr}\"; print -r -- \"${(qq)arr}\"".to_string(),
            // Quoting inside a nested expansion — the flag must not leak out.
            8 => {
                let s = pick(&mut rng, QUOTE_SUBJECTS);
                format!("v='{}'; print -r -- \"${{(q)${{v}}}}\"", s.replace('\'', "'\\''"))
            }
            // ${(z)…} splits a quoted string back into shell words — the
            // consumer of everything above.
            _ => {
                let s = pick(&mut rng, QUOTE_SUBJECTS);
                format!(
                    "v=\"${{(q)$(print -rn -- '{}')}}\"; print -rl -- ${{(z)v}}; print -r -- END",
                    s.replace('\'', "'\\''")
                )
            }
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// datetime generator (zsh/datetime)
//
// `strftime` is on the hot path of every prompt theme, so its format-specifier
// coverage is load-bearing. TZ is pinned to UTC in the preamble: without it the
// two shells would still agree (same env) but the corpus would not reproduce
// across machines. Only FIXED epochs are used — never $EPOCHSECONDS, which is
// nondeterministic by construction. C: Src/Modules/datetime.c bin_strftime().
// ---------------------------------------------------------------------------

/// Epochs chosen to straddle the awkward cases: a leap day, a year boundary,
/// an ISO-week boundary (Jan 1 falling in the previous ISO year), the epoch
/// itself, and a pre-epoch (negative) instant.
const EPOCHS: &[&str] = &[
    "0",            // 1970-01-01T00:00:00Z
    "1600000000",   // 2020-09-13 (Sunday — %u/%w edge)
    "1583020800",   // 2020-03-01, just past a leap day
    "1582934400",   // 2020-02-29 — leap day
    "1577836800",   // 2020-01-01 — ISO week 1 of 2020
    "1609459199",   // 2020-12-31T23:59:59Z
    "946684800",    // 2000-01-01
    "2147483647",   // Y2038 boundary
    "-86400",       // 1969-12-31 — pre-epoch
];

const STRF_FMTS: &[&str] = &[
    "%Y-%m-%d",
    "%H:%M:%S",
    "%Y-%m-%dT%H:%M:%S",
    "%j",           // day of year
    "%U|%W|%V",     // week numbers: Sunday-based, Monday-based, ISO
    "%G-W%V-%u",    // ISO year/week/day — disagrees with %Y at year boundaries
    "%a %A %b %B",  // names
    "%C|%y",        // century, 2-digit year
    "%e|%k|%l",     // space-padded variants
    "%I %p",        // 12-hour
    "%D|%F|%R|%T",  // compound specifiers
    "%s",           // seconds back out — must round-trip the input
    "%n|%t|%%",     // literal newline/tab/percent
    "%Z|%z",        // TZ-pinned
    "%c|%x|%X",     // locale-dependent (same libc both sides)
    "%w|%u",        // weekday numbering: 0-6 (Sun=0) vs 1-7 (Mon=1)
];

fn gen_datetime(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec![
        "zmodload zsh/datetime".to_string(),
        "export TZ=UTC".to_string(),
    ];
    for _ in 0..rng.gen_range(2..=4) {
        let f = pick(&mut rng, STRF_FMTS);
        let e = pick(&mut rng, EPOCHS);
        let stmt = match rng.gen_range(0..6) {
            // Plain: format an epoch to stdout.
            0 => format!("strftime '{f}' {e}"),
            // -s: assign instead of printing.
            1 => format!("strftime -s v '{f}' {e}; print -r -- \"[$v]\""),
            // -r: parse a formatted time back to an epoch. Round-trips only for
            // formats that carry a full date+time, so use a fixed pair.
            2 => format!("strftime -r '%Y-%m-%d %H:%M:%S' '2020-09-13 12:26:40'"),
            3 => format!("strftime -r -s v '%Y-%m-%dT%H:%M:%S' '2000-01-01T00:00:00'; print -r -- \"[$v]\""),
            // The %s round trip: format then re-parse must be the identity.
            4 => format!(
                "strftime -s t '%Y-%m-%d %H:%M:%S' {e}; strftime -r -s back '%Y-%m-%d %H:%M:%S' \"$t\"; print -r -- \"$(( back == {e} ))\""
            ),
            // Nanosecond arg (3rd positional) — %N is a zsh extension.
            _ => format!("strftime '{f}' {e} 123456789"),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// paramod generator (zsh/parameter)
//
// The introspection surface every serious plugin framework is built on:
// $functions (function BODIES as text — the formatting is a parity contract),
// $parameters (type strings), $options, $aliases/$galiases/$saliases,
// $funcstack/$funcfiletrace, $+commands/$+builtins.
// C: Src/Modules/parameter.c.
//
// Deterministic: membership tests and single-key reads only; whole-table dumps
// always go through an ordering flag, and $commands is only ever probed with
// `${+commands[…]}` (its contents depend on PATH, its membership does not).
// ---------------------------------------------------------------------------

fn gen_paramod(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec!["zmodload zsh/parameter".to_string()];
    for _ in 0..rng.gen_range(2..=4) {
        let stmt = match rng.gen_range(0..14) {
            // $functions[f] is the function body, re-rendered from the parse
            // tree — indentation, `;` placement and keyword spelling must match.
            0 => "f() { print -r -- hi }; print -r -- \"$functions[f]\"".to_string(),
            1 => "f() { if [[ -n $1 ]]; then print -r -- \"$1\"; else print -r -- none; fi }; print -r -- \"$functions[f]\"".to_string(),
            2 => "f() { for i in 1 2 3; do print -r -- $i; done }; print -r -- \"$functions[f]\"".to_string(),
            3 => "f() { local x=1; (( x++ )); print -r -- $x }; print -r -- \"$functions[f]\"".to_string(),
            // Round trip: eval the stored body back into a function and run it.
            4 => "f() { print -r -- body }; functions[g]=$functions[f]; g".to_string(),
            // Membership / removal.
            5 => "f() { : }; print -r -- \"${+functions[f]} ${+functions[nope]}\"; unfunction f; print -r -- \"${+functions[f]}\"".to_string(),
            6 => "f() { : }; g() { : }; print -r -- ${(ok)functions[(I)[fg]]}".to_string(),
            // $parameters — the TYPE string for each declared parameter.
            7 => {
                let d = pick(
                    &mut rng,
                    &[
                        "x=1",
                        "typeset -i x=1",
                        "typeset -a x=(1 2)",
                        "typeset -A x=(k v)",
                        "typeset -F x=1.5",
                        "typeset -r x=ro",
                        "typeset -x x=exported",
                        "typeset -Z 3 x=7",
                    ],
                );
                format!("{d}; print -r -- \"$parameters[x]\"; print -r -- \"${{(t)x}}\"")
            }
            // $options tracks setopt state, both directions.
            8 => {
                let o = pick(&mut rng, &["extendedglob", "nullglob", "ksharrays", "shwordsplit", "nomatch"]);
                format!("print -r -- \"$options[{o}]\"; setopt {o}; print -r -- \"$options[{o}]\"; unsetopt {o}; print -r -- \"$options[{o}]\"")
            }
            // $options is writable: assigning drives setopt.
            9 => "options[extendedglob]=on; [[ -o extendedglob ]] && print -r -- on || print -r -- off".to_string(),
            // $aliases / $galiases / $saliases.
            10 => "alias a1='print x'; alias -g G1='| cat'; alias -s sfx='print'; print -r -- \"[$aliases[a1]][$galiases[G1]][$saliases[sfx]]\"".to_string(),
            11 => "alias b='print y'; print -r -- ${(ok)aliases[(I)b]}; unalias b; print -r -- \"${+aliases[b]}\"".to_string(),
            // $funcstack / $functrace inside nested calls.
            12 => "outer() { inner }; inner() { print -r -- \"stack=${(j:,:)funcstack}\" }; outer".to_string(),
            // Builtin/command membership (never the whole table — PATH varies).
            _ => "print -r -- \"${+builtins[print]} ${+builtins[nosuchbuiltin]} ${+functions[print]}\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// procsub generator
//
// Process substitution: `<(…)` (a /dev/fd path fed by a forked writer) and
// `=(…)` (a real temp file, written fully before the command runs). The two
// differ in exactly the ways that matter — a `=()` file is seekable and
// complete, a `<()` fifo/fd is neither. C: Src/exec.c getproc()/getoutputfile().
//
// Deterministic: the substituted PATH is never printed (an fd number / temp
// name is not a parity property) — only the CONTENT read through it. `>(…)` is
// excluded on purpose: zsh does not wait for the writer before the next
// command, so its output ordering is racy and would produce false positives.
// ---------------------------------------------------------------------------

fn gen_procsub(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(1..=3) {
        let stmt = match rng.gen_range(0..12) {
            0 => "cat <(print -l a b c)".to_string(),
            1 => "cat <(print -l a b) <(print -l c d)".to_string(),
            // Nested process substitution.
            2 => "cat <(cat <(print -l x y))".to_string(),
            // Redirect stdin FROM a process substitution.
            3 => "while read -r l; do print -r -- \"L:$l\"; done < <(print -l p q r)".to_string(),
            4 => "read -r first < <(print -l one two); print -r -- \"first=$first\"".to_string(),
            // `=(…)`: a real file — seekable, and `$(<f)` works on it.
            5 => "cat =(print -l 1 2 3)".to_string(),
            6 => "f==(print -l a b); print -r -- \"$(<$f)\"; print -r -- \"exists=$([[ -f $f ]] && print y || print n)\"".to_string(),
            // A `=()` file is a regular file; a `<()` path is not.
            7 => "[[ -f =(print x) ]] && print -r -- regular || print -r -- notregular".to_string(),
            // Command substitution wrapping process substitution.
            8 => "v=$(cat <(print -l m n)); print -r -- \"[${v//$'\\n'/,}]\"".to_string(),
            // Word splitting of the read-back content.
            9 => "print -rl -- ${(f)\"$(cat <(print -l q w e))\"}; print -r -- END".to_string(),
            // Process substitution as a non-first argument, twice in one word list.
            10 => "wc -l < <(print -l a b c d)".to_string(),
            // Exit status: the SUBSTITUTION's status is not the command's.
            _ => "cat <(exit 3) > /dev/null; print -r -- \"rc=$?\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// alias generator
//
// Aliases are expanded by the LEXER, not the evaluator: the binding is frozen
// when the enclosing function is PARSED, so redefining an alias afterwards does
// not change the already-parsed body. An AST-first shell gets this wrong by
// construction unless it models it deliberately. Also covered: global aliases
// (expand in any word position), suffix aliases (expand on the command word by
// extension), a trailing space in an alias body (which makes the NEXT word
// alias-eligible), and self-referential aliases (expanded once, not forever).
// C: Src/lex.c checkalias(), Src/hashtable.c, Src/builtin.c bin_alias().
// ---------------------------------------------------------------------------

fn gen_alias(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(1..=3) {
        let stmt = match rng.gen_range(0..14) {
            0 => "alias hi='print -r -- HI'; hi".to_string(),
            // Alias body ending in a space makes the next word alias-eligible.
            1 => "alias e='print -r -- '; alias w=WORD; e w".to_string(),
            // No trailing space ⇒ the next word is NOT expanded.
            2 => "alias e='print -r --'; alias w=WORD; e w".to_string(),
            // Alias chaining.
            3 => "alias a=b; alias b='print -r -- CHAIN'; a".to_string(),
            // Self-reference: expanded once, then the command word wins.
            4 => "alias print='print -r -- pre'; print x; unalias print; print -r -- post".to_string(),
            // Parse-time binding: f captures the alias as it was at PARSE time.
            5 => "alias x='print -r -- A'; f() { x }; alias x='print -r -- B'; f; x".to_string(),
            // Global alias: expands in ANY word position, not just command.
            6 => "alias -g UP='| tr a-z A-Z'; print -r -- hello UP".to_string(),
            7 => "alias -g N=/dev/null; print -r -- gone > N; print -r -- done".to_string(),
            8 => "alias -g A='a b'; print -rl -- A; print -r -- END".to_string(),
            // Suffix alias: a command word with that extension runs the alias.
            9 => "alias -s zzz='print -r -- ran'; foo.zzz".to_string(),
            // Alias listing forms — the OUTPUT FORMAT is the contract.
            10 => "alias q1='print \"a b\"'; alias q2='x*y'; alias".to_string(),
            11 => "alias l1='print 1'; alias -L".to_string(),
            12 => "alias m1='print 1'; alias m2='print 2'; alias -m 'm*'".to_string(),
            // `command`/`\` bypass alias expansion; unalias -m removes by pattern.
            _ => "alias p='print -r -- ALIASED'; \\p -r -- raw; unalias -m 'p*'; p -r -- gone".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// autoload generator
//
// Function autoloading off $fpath: zsh's default is "the file IS the body"
// (a bare list of commands), while KSH_AUTOLOAD means "the file DEFINES the
// function and is then called". `autoload +X` loads without running; `-U`
// suppresses alias expansion inside the loaded body; `functions`/`whence -v`
// must report the loaded state. Runs from a read-only fixture directory whose
// `fns/` holds the function files. C: Src/exec.c loadautofn()/execautofn().
// ---------------------------------------------------------------------------

fn gen_autoload(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec!["fpath=(./fns)".to_string()];
    for _ in 0..rng.gen_range(1..=3) {
        let stmt = match rng.gen_range(0..12) {
            // Default (non-ksh) autoload: the file's text is the body.
            0 => "autoload -Uz af_args; af_args a b c".to_string(),
            1 => "autoload -Uz af_plain; af_plain; af_plain".to_string(),
            // $0 inside an autoloaded function is the FUNCTION name (unless
            // FUNCTION_ARGZERO is off).
            2 => "autoload -Uz af_zero; af_zero".to_string(),
            // +X loads the body without executing it; the body is then visible.
            3 => "autoload -Uz +X af_plain; print -r -- \"${+functions[af_plain]}\"; functions af_plain".to_string(),
            // Undefined until called: the stub body is the marker.
            4 => "autoload -Uz af_plain; print -r -- \"$functions[af_plain]\"".to_string(),
            5 => "autoload -Uz af_plain; whence -v af_plain".to_string(),
            // KSH_AUTOLOAD: the file DEFINES the function, then it is called.
            6 => "setopt kshautoload; autoload -Uz af_ksh; af_ksh one".to_string(),
            // Without KSH_AUTOLOAD, a file that both defines AND calls runs the
            // definition as the body — the extra call is the observable.
            7 => "autoload -Uz af_ksh; af_ksh one".to_string(),
            // -U suppresses alias expansion inside the loaded body: the body's
            // `helper arg` resolves to the FUNCTION, not the alias.
            8 => "helper() { print -r -- \"FUNC $1\" }; alias helper='print -r -- HIJACKED'; autoload -Uz af_alias; af_alias".to_string(),
            // …without -U, the alias DOES apply inside the body.
            9 => "helper() { print -r -- \"FUNC $1\" }; alias helper='print -r -- HIJACKED'; autoload -z af_alias; af_alias".to_string(),
            // A file defining several functions; only the named one autoloads.
            10 => "autoload -Uz af_multi; af_multi".to_string(),
            // Listing autoloads and unfunction'ing an unresolved stub.
            _ => "autoload -Uz af_plain af_args; autoload +X af_plain; unfunction af_args; print -r -- \"${+functions[af_args]} ${+functions[af_plain]}\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// stat generator (zsh/stat)
//
// Runs against the same fixed fixture as glob mode (distinct sizes, staggered
// mtimes), so every field read is deterministic. atime and ctime are NEVER
// probed: reading a file updates atime, so the second shell to run would see a
// different value — a harness artifact, not a parity gap.
// C: Src/Modules/stat.c bin_stat().
// ---------------------------------------------------------------------------

const STAT_FILES: &[&str] = &["a.txt", "bb.log", "ccc.txt", "d.md", "empty", "dir1", "link"];
const STAT_FIELDS: &[&str] = &["+size", "+mode", "+nlink", "+mtime", "+uid", "+gid", "+link"];

fn gen_stat(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec![
        "zmodload zsh/stat".to_string(),
        "export TZ=UTC".to_string(),
    ];
    for _ in 0..rng.gen_range(1..=3) {
        let f = pick(&mut rng, STAT_FILES);
        let fld = pick(&mut rng, STAT_FIELDS);
        let stmt = match rng.gen_range(0..10) {
            // Single field, raw.
            0 => format!("zstat {fld} {f}"),
            // -s: string-ify (mode becomes -rw-r--r--, uid becomes a name).
            1 => format!("zstat -s {fld} {f}"),
            // -H: read the whole stat into an assoc, then index it.
            2 => format!("zstat -H h {f}; print -r -- \"$h[size] $h[nlink]\""),
            3 => format!("zstat -s -H h {f}; print -r -- \"$h[mode]\""),
            // -A: read into an array (order is the canonical field order).
            4 => format!("zstat -A arr +size {f}; print -r -- \"${{(j:,:)arr}}\""),
            // -F: format the time fields (TZ-pinned above).
            5 => format!("zstat -F '%Y-%m-%d %H:%M:%S' +mtime {f}"),
            // -L: lstat — the dangling symlink is the whole point.
            6 => format!("zstat -L +size link; zstat -L -s +mode link"),
            // -n: prefix each result with the file name (multi-file form).
            7 => format!("zstat -n {fld} a.txt bb.log ccc.txt"),
            // Whole stat, no field selector: every name=value line.
            8 => format!("zstat -H h {f}; print -rl -- ${{(ok)h}}; print -r -- END"),
            // Error path: a nonexistent file must fail the same way.
            _ => "zstat +size nosuchfile 2>/dev/null; print -r -- \"rc=$?\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// errexit generator
//
// Failure propagation is a nest of interacting options, and the exception
// carve-outs are the part shells get wrong: under ERR_EXIT a command that fails
// inside an `if` condition, a `&&`/`||` left operand, a `!`-negated pipeline, or
// a `while` test must NOT exit — but the same command as a plain statement must.
// PIPE_FAIL changes which member of a pipeline supplies `$?`; ERR_RETURN makes a
// failure return from the enclosing function instead of exiting the shell; an
// `always` block runs regardless and can clear the error via TRY_BLOCK_ERROR.
// The generated script's EXIT STATUS is itself a compared output here.
// C: Src/exec.c execlist()/execpline(), Src/loop.c, Src/builtin.c.
// ---------------------------------------------------------------------------

fn gen_errexit(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    // Roughly half the cases arm one or more of the propagation options.
    let mut opts: Vec<&str> = Vec::new();
    if rng.gen_bool(0.5) {
        opts.push("errexit");
    }
    if rng.gen_bool(0.3) {
        opts.push("pipefail");
    }
    if rng.gen_bool(0.3) {
        opts.push("errreturn");
    }
    if !opts.is_empty() {
        stmts.push(format!("setopt {}", opts.join(" ")));
    }
    for _ in 0..rng.gen_range(1..=3) {
        let stmt = match rng.gen_range(0..16) {
            // Bare failure — under errexit the shell dies here and the trailing
            // print never runs, so the script's exit code is the observable.
            0 => "false; print -r -- after".to_string(),
            1 => "(exit 3); print -r -- \"rc=$?\"".to_string(),
            // Carve-out: a failing command in an `if` CONDITION never triggers
            // errexit.
            2 => "if false; then print -r -- t; else print -r -- f; fi; print -r -- after".to_string(),
            // Carve-out: left operand of && / ||.
            3 => "false || print -r -- fallback; print -r -- after".to_string(),
            4 => "false && print -r -- never; print -r -- after".to_string(),
            // Carve-out: a `!`-negated pipeline.
            5 => "! false; print -r -- \"rc=$? after\"".to_string(),
            // Carve-out: a `while` test.
            6 => "n=0; while (( n < 2 )) && false; do :; done; print -r -- after".to_string(),
            // NOT a carve-out: the last command of an && chain still triggers.
            7 => "true && false; print -r -- after".to_string(),
            // Pipelines: $? is the LAST member's status, or the last FAILING
            // member's under pipefail.
            8 => "true | false | true; print -r -- \"rc=$?\"".to_string(),
            9 => "false | true; print -r -- \"rc=$?\"".to_string(),
            10 => "print -r -- x | false; print -r -- \"rc=$?\"".to_string(),
            // errreturn: the failure returns from f, not from the shell.
            11 => "f() { false; print -r -- unreached }; f; print -r -- \"rc=$? after\"".to_string(),
            12 => "f() { return 4 }; f; print -r -- \"rc=$?\"".to_string(),
            // always: runs on both paths; TRY_BLOCK_ERROR reports/clears the error.
            13 => "{ false } always { print -r -- \"always tbe=$TRY_BLOCK_ERROR\" }; print -r -- \"rc=$? after\"".to_string(),
            14 => "{ false } always { TRY_BLOCK_ERROR=0 }; print -r -- \"rc=$? cleared\"".to_string(),
            // A failing command in a subshell/command substitution.
            _ => "v=$(false); print -r -- \"rc=$? v=[$v]\"".to_string(),
        };
        stmts.push(stmt);
    }
    // Make the final exit status observable even when errexit killed the script
    // early (in which case this line never runs — which is itself the signal).
    stmts.push("print -r -- END".to_string());
    stmts
}

// ---------------------------------------------------------------------------
// posparam generator
//
// Positional parameters are their own parameter type in C: `$@`/`$*` differ
// only under quoting, `argv` is an ALIAS for the whole list (assigning to it
// rewrites the positionals), `shift` takes a count, and `${@:off:len}` slices
// with 1-based, not 0-based, semantics. The joins all key off IFS[1].
// C: Src/params.c (argvgetfn/argvsetfn), Src/builtin.c bin_shift.
// ---------------------------------------------------------------------------

fn gen_posparam(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec!["set -- alpha beta gamma delta".to_string()];
    for _ in 0..rng.gen_range(2..=4) {
        let stmt = match rng.gen_range(0..16) {
            0 => "print -r -- \"n=$# 1=$1 last=${@[-1]} all=$*\"".to_string(),
            // "$*" joins on IFS[1]; "$@" never joins.
            1 => "print -rl -- \"$*\"; print -r -- --; print -rl -- \"$@\"; print -r -- END".to_string(),
            2 => "IFS=:; print -r -- \"[$*]\"; IFS=' '; print -r -- \"[$*]\"".to_string(),
            // Unquoted $@ word-splits and DROPS empties; "$@" keeps them.
            3 => "set -- a '' b; print -r -- \"n=$#\"; for x in \"$@\"; do print -r -- \"<$x>\"; done".to_string(),
            4 => "set -- a '' b; for x in $@; do print -r -- \"<$x>\"; done; print -r -- END".to_string(),
            // Slices are 1-based.
            5 => format!("print -r -- \"[${{@:{}:{}}}]\"", rng.gen_range(1..4), rng.gen_range(1..4)),
            6 => "print -r -- \"[${@:2}]\" \"[${*:2:2}]\"".to_string(),
            // shift with a count, and past the end.
            7 => format!("shift {}; print -r -- \"n=$# rest=$*\"", rng.gen_range(1..3)),
            8 => "shift 9 2>/dev/null; print -r -- \"rc=$? n=$#\"".to_string(),
            // `argv` aliases the whole list, in both directions.
            9 => "print -r -- \"argv=(${(j:,:)argv}) count=${#argv}\"".to_string(),
            10 => "argv=(x y); print -r -- \"n=$# 1=$1 2=$2\"".to_string(),
            11 => "argv[2]=BETA; print -r -- \"$*\"".to_string(),
            // set -- with no args clears; `set --` vs `set -` differ.
            12 => "set --; print -r -- \"n=$# empty=[$*]\"".to_string(),
            // Flags applied to the positional list.
            13 => "print -r -- \"${(o)@}\" ; print -r -- \"${(U)@}\"".to_string(),
            14 => "print -r -- \"${#@} ${(j:-:)@}\"".to_string(),
            // Positionals inside a function are the FUNCTION's, not the shell's.
            _ => "f() { print -r -- \"in=$# $1\" }; f one two; print -r -- \"out=$# $1\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// numfmt generator
//
// How a number turns back into TEXT: the output base of `typeset -i N`, the
// precision of `-F`/`-E`, the zero/blank padding of `-Z`/`-L`/`-R`, and the
// float formatting the arithmetic evaluator uses when it prints a result. These
// are the details a prompt or a numeric script trips over.
// C: Src/params.c convfloat()/convbase(), Src/builtin.c typeset_setbase().
// ---------------------------------------------------------------------------

fn gen_numfmt(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(2..=4) {
        let stmt = match rng.gen_range(0..14) {
            // Integer with an output base: the value re-renders as `base#digits`.
            0 => {
                let b = pick(&mut rng, &[2, 8, 16, 36]);
                format!("typeset -i {b} v={}; print -r -- $v", rng.gen_range(0..300))
            }
            // A negative value in a non-decimal base.
            1 => format!("typeset -i 16 v=-{}; print -r -- $v", rng.gen_range(1..300)),
            // Bases are a RENDER property: arithmetic on it stays numeric.
            2 => "typeset -i 16 v=255; print -r -- $v; print -r -- $(( v + 1 ))".to_string(),
            // Float precision, fixed vs scientific.
            3 => {
                let p = rng.gen_range(0..8);
                format!("typeset -F {p} f=3.14159265358979; print -r -- $f")
            }
            4 => {
                let p = rng.gen_range(0..8);
                format!("typeset -E {p} f=3.14159265358979; print -r -- $f")
            }
            5 => "typeset -F f=0.1; print -r -- $f; typeset -E e=0.1; print -r -- $e".to_string(),
            // Very large / very small magnitudes pick the exponent form.
            6 => "typeset -F f=1e20; print -r -- $f; typeset -E g=1e-20; print -r -- $g".to_string(),
            // Bare arithmetic float printing (no typeset) — the default format.
            7 => "print -r -- $(( 1.0 / 3 )); print -r -- $(( 2.0 ** 0.5 ))".to_string(),
            8 => "print -r -- $(( 1 / 3 )); print -r -- $(( 1.0 / 3.0 * 3 ))".to_string(),
            // Zero / left / right padding.
            9 => {
                let w = rng.gen_range(2..8);
                format!("typeset -Z {w} z={}; print -r -- \"[$z]\"", rng.gen_range(1..9999))
            }
            10 => {
                let w = rng.gen_range(2..8);
                format!("typeset -L {w} l=abc; typeset -R {w} r=abc; print -r -- \"[$l][$r]\"")
            }
            // -Z on a value LONGER than the width truncates, it does not grow.
            11 => "typeset -Z 3 z=123456; print -r -- \"[$z]\"".to_string(),
            // printf's own float formats over the same values.
            12 => "printf '%g|%e|%f\\n' 0.1 0.1 0.1; printf '%g|%e\\n' 1e20 1e-20".to_string(),
            // Integer overflow / 64-bit edges re-rendered.
            _ => "print -r -- $(( 2**62 )); print -r -- $(( -(2**62) ))".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// mapfile generator (zsh/mapfile)
//
// $mapfile[file] reads a whole file as one scalar and, on assignment, WRITES
// it. Unlike `$(<file)` it preserves the trailing newline, and unsetting an
// element unlinks the file. Runs from the scratch fixture because it mutates
// the filesystem. C: Src/Modules/mapfile.c.
// ---------------------------------------------------------------------------

fn gen_mapfile(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    // A per-case file name keeps parallel workers from colliding in the shared
    // scratch cwd; both shells run the SAME seed, so they use the same name and
    // each fully rewrites it before reading.
    let f = format!("mf_{seed}");
    let mut stmts = vec!["zmodload zsh/mapfile".to_string()];
    for _ in 0..rng.gen_range(1..=3) {
        let stmt = match rng.gen_range(0..10) {
            // Write then read back — mapfile keeps the trailing newline that
            // `$(<f)` strips.
            0 => format!("mapfile[{f}]=$'a\\nb\\n'; print -r -- \"[${{mapfile[{f}]}}]\""),
            1 => format!("mapfile[{f}]=$'a\\nb\\n'; print -r -- \"[$(<{f})]\""),
            2 => format!("mapfile[{f}]=$'x\\n'; print -r -- \"len=${{#mapfile[{f}]}}\""),
            // Split the read-back content into lines.
            3 => format!("mapfile[{f}]=$'l1\\nl2\\nl3\\n'; print -rl -- ${{(f)mapfile[{f}]}}; print -r -- END"),
            // Append by re-assigning the concatenation.
            4 => format!("mapfile[{f}]=$'one\\n'; mapfile[{f}]=\"${{mapfile[{f}]}}two\"; print -r -- \"[${{mapfile[{f}]}}]\""),
            // An empty write creates an empty file.
            5 => format!("mapfile[{f}]=''; print -r -- \"e=${{#mapfile[{f}]}} exists=$([[ -f {f} ]] && print y || print n)\""),
            // Membership / absence of a nonexistent file.
            6 => format!("print -r -- \"${{+mapfile[nosuch_{seed}]}} [${{mapfile[nosuch_{seed}]}}]\""),
            // unset unlinks the file.
            7 => format!("mapfile[{f}]=$'z\\n'; unset \"mapfile[{f}]\"; print -r -- \"gone=$([[ -f {f} ]] && print n || print y)\""),
            // Binary-ish content round trip (no NULs — those are not preservable).
            8 => format!("mapfile[{f}]=$'\\x01\\x02\\n'; print -r -- \"len=${{#mapfile[{f}]}}\""),
            // Content with no trailing newline stays that way.
            _ => format!("mapfile[{f}]='notrail'; print -r -- \"[${{mapfile[{f}]}}] len=${{#mapfile[{f}]}}\""),
        };
        stmts.push(stmt);
    }
    // Cleanup: literal name, guarded by a glob that can only match the fixture.
    stmts.push(format!("case {f} in (mf_*) command rm -f -- {f};; esac"));
    stmts
}

// ---------------------------------------------------------------------------
// pcre generator (zsh/pcre)
//
// The PCRE backend is a SECOND regex engine with its own capture plumbing:
// `pcre_compile`/`pcre_match` set $MATCH/$match (or a named assoc via -a/-A),
// and `setopt rematchpcre` re-points the `=~` operator at it. The capture
// side effects are where the two engines diverge.
// C: Src/Modules/pcre.c.
// ---------------------------------------------------------------------------

const PCRE_PATS: &[&str] = &[
    "([a-z]+)([0-9]+)",
    "^(\\w+)@(\\w+)\\.com$",
    "(?i)HELLO",
    "a(b*)c",
    "(\\d{2,4})-(\\d{2})",
    "(?<word>[a-z]+)",
    "x(y)?z",
    "^$",
    "(a|b)+",
];

const PCRE_SUBJ: &[&str] = &[
    "abc123", "user@site.com", "hello", "ac", "abbbc", "2024-06", "xz", "xyz", "", "ababab",
];

fn gen_pcre(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec!["zmodload zsh/pcre".to_string()];
    for _ in 0..rng.gen_range(1..=3) {
        let p = pick(&mut rng, PCRE_PATS);
        let s = pick(&mut rng, PCRE_SUBJ);
        let stmt = match rng.gen_range(0..8) {
            // Plain compile + match, reporting only the status.
            0 => format!("pcre_compile '{p}'; pcre_match '{s}'; print -r -- \"rc=$?\""),
            // $MATCH / $match capture side effects.
            1 => format!(
                "pcre_compile '{p}'; if pcre_match '{s}'; then print -r -- \"M=[$MATCH] m=(${{(j:,:)match}})\"; else print -r -- NOMATCH; fi"
            ),
            // -a: captures into a named ARRAY instead of $match.
            2 => format!(
                "pcre_compile '{p}'; if pcre_match -a arr '{s}'; then print -r -- \"arr=(${{(j:,:)arr}}) n=${{#arr}}\"; else print -r -- NOMATCH; fi"
            ),
            // -n: start the match at an offset.
            3 => format!("pcre_compile '{p}'; pcre_match -n 1 '{s}'; print -r -- \"rc=$?\""),
            // -b: report the byte offsets of the match ($ZPCRE_OP).
            4 => format!(
                "pcre_compile '{p}'; if pcre_match -b '{s}'; then print -r -- \"op=[$ZPCRE_OP]\"; else print -r -- NOMATCH; fi"
            ),
            // Case-insensitive compile flag.
            5 => format!("pcre_compile -i '{p}'; pcre_match '{s}'; print -r -- \"rc=$?\""),
            // REMATCH_PCRE re-points `=~` at the PCRE engine — same operator,
            // different engine, same capture variables.
            6 => format!(
                "setopt rematchpcre; if [[ '{s}' =~ '{p}' ]]; then print -r -- \"M=[$MATCH] m=(${{(j:,:)match}})\"; else print -r -- NOMATCH; fi"
            ),
            // …and the POSIX engine for the same input, as the control.
            _ => format!(
                "if [[ '{s}' =~ '{p}' ]]; then print -r -- \"M=[$MATCH]\"; else print -r -- NOMATCH; fi"
            ),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// zwc generator (zcompile + .zwc autoload)
//
// The compiled-digest path is a compat-floor item: zsh writes a `.zwc` next to
// a function file, and every later autoload reads the DIGEST instead of the
// source. A shell that cannot read its own (or zsh's) .zwc silently falls back
// to reparsing, which is exactly the regression this mode is here to catch.
//
// Each case builds its own mktemp fixture INSIDE the script, so the two shells
// never share a .zwc — otherwise whichever shell ran first would leave a digest
// for the other to consume, making the case order-dependent and flaky.
// C: Src/parse.c bin_zcompile / Src/exec.c getfpfunc (dump lookup).
// ---------------------------------------------------------------------------

fn gen_zwc(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec![
        "d=$(mktemp -d /tmp/pf_zwc_XXXXXX) || exit 1".to_string(),
        "mkdir -p $d/fns".to_string(),
        "print -r -- 'print -r -- \"zw args=$* n=$#\"' > $d/fns/zw_fn".to_string(),
        "print -r -- 'print -r -- second' > $d/fns/zw_two".to_string(),
    ];
    for _ in 0..rng.gen_range(1..=2) {
        let stmt = match rng.gen_range(0..8) {
            // Compile a function file, then autoload it — the .zwc must be used
            // and must produce the same result as the source would.
            0 => "zcompile $d/fns/zw_fn; fpath=($d/fns); autoload -Uz zw_fn; zw_fn a b".to_string(),
            // The digest file exists and is non-empty.
            1 => "zcompile $d/fns/zw_fn; print -r -- \"zwc=$([[ -s $d/fns/zw_fn.zwc ]] && print y || print n)\"".to_string(),
            // -c: compile a set of files into ONE digest named by the first arg.
            2 => "zcompile $d/all.zwc $d/fns/zw_fn $d/fns/zw_two; print -r -- \"rc=$? made=$([[ -s $d/all.zwc ]] && print y || print n)\"".to_string(),
            // A digest earlier on $fpath than the source still resolves.
            3 => "zcompile $d/fns/zw_fn; command rm -f $d/fns/zw_fn; fpath=($d/fns); autoload -Uz zw_fn; zw_fn x".to_string(),
            // -t: TEST whether a digest is up to date. Its LISTING output is
            // discarded on purpose: it prints the digest's own path, and the two
            // shells each build their own mktemp fixture, so the path is not a
            // parity property. Only the status is.
            4 => "zcompile $d/fns/zw_fn; zcompile -t $d/fns/zw_fn.zwc >/dev/null 2>&1; print -r -- \"rc=$?\"".to_string(),
            // A stale digest (source newer) must not be preferred.
            5 => "zcompile $d/fns/zw_fn; print -r -- 'print -r -- FRESH' > $d/fns/zw_fn; touch $d/fns/zw_fn; fpath=($d/fns); autoload -Uz zw_fn; zw_fn".to_string(),
            // zcompile on a missing file fails cleanly.
            6 => "zcompile $d/fns/nosuch 2>/dev/null; print -r -- \"rc=$?\"".to_string(),
            // Two functions from one digest.
            _ => "zcompile $d/fns/zw_two; fpath=($d/fns); autoload -Uz zw_two; zw_two; zw_two".to_string(),
        };
        stmts.push(stmt.to_string());
    }
    // Cleanup: literal path, glob-guarded so it can only ever match the fixture.
    stmts.push("case $d in (/tmp/pf_zwc_*) command rm -rf -- \"$d\";; esac".to_string());
    stmts
}

// ---------------------------------------------------------------------------
// (no nameref mode)
//
// `typeset -n` namerefs exist in the zsh SOURCE (PM_NAMEREF) but NOT in the
// reference binary this fuzzer differentials against — zsh 5.9.2 answers
// `typeset: bad option: -n`. A mode for them would compare zshrs against a
// shell that only ever errors, which teaches nothing about parity. Add one when
// the reference zsh is new enough to have them.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// tied generator
//
// `typeset -T SCALAR array [sep]` ties a scalar to an array: writing either
// rewrites the other, joined/split on `sep` (`:` by default). It is how
// PATH/path, FPATH/fpath and MANPATH/manpath work, so every plugin manager
// depends on the tie holding in BOTH directions. C: Src/params.c (PM_TIED,
// tiedarr GSU), Src/builtin.c typeset_single's -T arm.
// ---------------------------------------------------------------------------

fn gen_tied(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    // NB: the tied names are deliberately obscure. A first cut used `S`/`s`, and
    // `S` happened to be an exported variable in the ambient environment, so the
    // probe silently tested "tie an already-exported scalar" instead of "tie a
    // fresh one" — a real gap, but found by accident and only reproducible on
    // that machine. Env-collidable names make a mode's meaning depend on who
    // runs it; the export case is now probed on purpose below.
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(1..=3) {
        let stmt = match rng.gen_range(0..14) {
            // Scalar → array propagation, and back.
            0 => "typeset -T TS ts; TS=a:b:c; print -r -- \"n=${#ts} [${(j:|:)ts}]\"".to_string(),
            1 => "typeset -T TS ts; ts=(x y z); print -r -- \"[$TS]\"".to_string(),
            // A custom separator.
            2 => "typeset -T TS ts ,; TS=a,b,c; print -r -- \"n=${#ts} [${(j:|:)ts}]\"".to_string(),
            3 => "typeset -T TS ts ,; ts=(p q); print -r -- \"[$TS]\"".to_string(),
            // The tie survives an append to either side.
            4 => "typeset -T TS ts; TS=a:b; ts+=(c); print -r -- \"[$TS] n=${#ts}\"".to_string(),
            5 => "typeset -T TS ts; ts=(a b); TS=$TS:c; print -r -- \"n=${#ts} [${(j:|:)ts}]\"".to_string(),
            // Empty fields are preserved on the scalar side.
            6 => "typeset -T TS ts; TS=a::b; print -r -- \"n=${#ts} [${(j:|:)ts}]\"".to_string(),
            // An empty array yields an empty scalar.
            7 => "typeset -T TS ts; ts=(); print -r -- \"[$TS] n=${#ts}\"".to_string(),
            // Types: the scalar is scalar-tied, the array is array-tied.
            8 => "typeset -T TS ts; TS=a:b; print -r -- \"${(t)TS} ${(t)ts}\"".to_string(),
            // Element assignment through the array reaches the scalar.
            9 => "typeset -T TS ts; TS=a:b:c; ts[2]=B; print -r -- \"[$TS]\"".to_string(),
            // The real thing: PATH/path are tied by the shell itself.
            10 => "path=(/bin /usr/bin); print -r -- \"[$PATH]\"; PATH=/x:/y; print -r -- \"n=${#path} [${(j:|:)path}]\"".to_string(),
            // Untying leaves both halves standing.
            _ => "typeset -T TS ts; TS=a:b; unset S; print -r -- \"${+S} ${+s}\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// readb generator
//
// The `read` builtin's flag matrix, driven from a here-string / heredoc so the
// input is fixed. -A reads a whole array, -d changes the record delimiter, -k
// reads a character COUNT (not a line), -q asks a y/n question, -e/-E echo, -u
// picks an fd, and the trailing-word rule ("the last name gets the rest") is
// its own contract. C: Src/builtin.c bin_read().
// ---------------------------------------------------------------------------

fn gen_readb(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(1..=3) {
        let stmt = match rng.gen_range(0..14) {
            // The last name absorbs the remaining words.
            0 => "read a b <<< 'one two three'; print -r -- \"[$a][$b]\"".to_string(),
            1 => "read a b c d <<< 'one two'; print -r -- \"[$a][$b][$c][$d]\"".to_string(),
            // -A: the whole line as an array.
            2 => "read -A arr <<< 'one two three'; print -r -- \"n=${#arr} [${(j:,:)arr}]\"".to_string(),
            // -r keeps backslashes; without it they escape.
            3 => "read x <<< 'a\\tb'; print -r -- \"[$x]\"".to_string(),
            4 => "read -r x <<< 'a\\tb'; print -r -- \"[$x]\"".to_string(),
            // -d: a custom record delimiter.
            5 => "read -d , x <<< 'first,second'; print -r -- \"[$x]\"".to_string(),
            6 => "printf 'a\\0b\\0' | { read -d '' x; print -r -- \"[$x]\" }".to_string(),
            // -k: read exactly N characters, not a line.
            7 => "read -k 3 x <<< 'abcdef'; print -r -- \"[$x]\"".to_string(),
            8 => "read -k 2 -u 0 x <<< 'xyz'; print -r -- \"[$x]\"".to_string(),
            // -q: y/n — true only for `y`.
            9 => "read -q x <<< 'y'; print -r -- \"rc=$? [$x]\"".to_string(),
            10 => "read -q x <<< 'n'; print -r -- \"rc=$? [$x]\"".to_string(),
            // IFS drives the field split.
            11 => "IFS=: read a b <<< 'l:r'; print -r -- \"[$a][$b]\"".to_string(),
            // EOF with no input: nonzero status, name left empty.
            12 => "read x </dev/null; print -r -- \"rc=$? [$x]\"".to_string(),
            // Reading several lines in sequence from one heredoc.
            _ => "{ read a; read b } <<'EOF'\nline1\nline2\nEOF\nprint -r -- \"[$a][$b]\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// fd generator
//
// Explicit file-descriptor manipulation: `exec N>file` opens a long-lived fd,
// `{v}>file` allocates one and stores its number, `>&N` / `<&N` duplicate, and
// `>&-` closes. The allocated NUMBER in `{v}` is not a parity property (it
// depends on what else is open), so it is never printed — only what flows
// through it. Runs from the scratch fixture: these probes create files.
// C: Src/exec.c (addfd/closemn), Src/parse.c redirection parsing.
// ---------------------------------------------------------------------------

fn gen_fd(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    // Per-seed file names keep parallel workers from colliding in the shared
    // scratch cwd; both shells run the same seed, so both use the same names.
    let f = format!("fd_{seed}");
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(1..=2) {
        let stmt = match rng.gen_range(0..12) {
            // exec-opened fd, written across several commands, then closed.
            0 => format!("exec 3> {f}; print -u 3 -r -- one; print -u 3 -r -- two; exec 3>&-; cat {f}"),
            // Read side.
            1 => format!("print -l a b c > {f}; exec 3< {f}; read -u 3 x; read -u 3 y; exec 3<&-; print -r -- \"[$x][$y]\""),
            // {v} auto-allocation — print the CONTENT, never the fd number.
            2 => format!("exec {{v}}> {f}; print -u $v -r -- alloc; exec {{v}}>&-; cat {f}"),
            3 => format!("print -r -- src > {f}; exec {{v}}< {f}; read -u $v x; exec {{v}}<&-; print -r -- \"[$x]\""),
            // Duplication: 2>&1 merges, and the ORDER of the dup matters.
            4 => "{ print -r -- out; print -r -- err >&2 } 2>&1 | cat".to_string(),
            5 => format!("{{ print -r -- o; print -r -- e >&2 }} > {f} 2>&1; cat {f}"),
            // Closing a fd then using it must fail.
            6 => "exec 3>&-; print -u 3 -r -- x 2>/dev/null; print -r -- \"rc=$?\"".to_string(),
            // Appending vs truncating.
            7 => format!("print -r -- a > {f}; print -r -- b >> {f}; cat {f}"),
            8 => format!("print -r -- a > {f}; print -r -- b > {f}; cat {f}"),
            // Read-write open.
            9 => format!("print -r -- rw > {f}; exec 3<> {f}; read -u 3 x; exec 3>&-; print -r -- \"[$x]\""),
            // A redirection scoped to a single compound command.
            10 => format!("for i in 1 2; do print -r -- \"line$i\"; done > {f}; wc -l < {f}"),
            // stdin from a file for a whole block.
            _ => format!("print -l p q > {f}; {{ read a; read b }} < {f}; print -r -- \"[$a][$b]\""),
        };
        stmts.push(stmt);
    }
    stmts.push(format!("case {f} in (fd_*) command rm -f -- {f};; esac"));
    stmts
}

// ---------------------------------------------------------------------------
// special generator
//
// Shell-maintained parameters whose VALUES are deterministic: $0, $?, $#,
// LINENO, ZSH_SUBSHELL (subshell nesting depth), funcstack/funcfiletrace,
// ZSH_NAME/ZSH_ARGZERO, and the option-driven ones. $RANDOM, $$, $EPOCHSECONDS
// and $SECONDS are excluded by construction — they are nondeterministic, not
// parity-relevant. C: Src/params.c (the special-parameter GSU table).
// ---------------------------------------------------------------------------

fn gen_special(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(2..=4) {
        let stmt = match rng.gen_range(0..14) {
            // $? threading.
            0 => "true; print -r -- $?; false; print -r -- $?; (exit 7); print -r -- $?".to_string(),
            // LINENO counts SOURCE lines, and is 1-based.
            1 => "print -r -- $LINENO\nprint -r -- $LINENO".to_string(),
            2 => "f() { print -r -- \"in=$LINENO\" }; f".to_string(),
            // ZSH_SUBSHELL is the nesting depth.
            3 => "print -r -- $ZSH_SUBSHELL; ( print -r -- $ZSH_SUBSHELL; ( print -r -- $ZSH_SUBSHELL ) )".to_string(),
            // A command substitution is a subshell too.
            4 => "print -r -- \"$(print -r -- $ZSH_SUBSHELL)\"".to_string(),
            // funcstack / funcfiletrace inside nested calls.
            5 => "outer() { inner }; inner() { print -r -- \"${(j:>:)funcstack}\" }; outer".to_string(),
            6 => "f() { print -r -- \"depth=${#funcstack}\" }; f".to_string(),
            // $0 inside and outside a function (FUNCTION_ARGZERO).
            // $0 INSIDE a function is the function name. At top level it is the
            // interpreter's own path, which is `zsh` vs `zshrs` by construction —
            // never a parity property, so it is not probed.
            7 => "f() { print -r -- \"[$0]\" }; f".to_string(),
            8 => "f() { print -r -- \"argzero=$0\" }; f; f() { print -r -- \"again=$0\" }; f".to_string(),
            // $# / $* with no positionals.
            9 => "print -r -- \"n=$# [$*]\"".to_string(),
            // $_ is the last argument of the previous command.
            10 => "print -r -- alpha >/dev/null; print -r -- \"[$_]\"".to_string(),
            // Option-state params.
            11 => "print -r -- \"[$ZSH_NAME]\"".to_string(),
            // $pipestatus for a whole pipeline.
            12 => "true | false | true; print -r -- \"[${(j:,:)pipestatus}]\"".to_string(),
            // Read-only specials must reject assignment.
            _ => "ZSH_SUBSHELL=9 2>/dev/null; print -r -- \"rc=$? [$ZSH_SUBSHELL]\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// brace generator
//
// Brace expansion happens BEFORE parameter expansion and is purely lexical:
// `{a,b}` alternation, `{1..9}` ranges (which count backwards, zero-pad when
// the endpoints are padded, and take a third `..step` field), and the products
// of adjacent braces. A brace with no comma and no range is NOT an expansion —
// it stays literal — and that rule is where implementations drift.
// C: Src/glob.c xpandbraces().
// ---------------------------------------------------------------------------

fn gen_brace_mode(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(2..=4) {
        let stmt = match rng.gen_range(0..18) {
            0 => "print -r -- {a,b,c}".to_string(),
            1 => "print -r -- x{a,b}y".to_string(),
            // Adjacent braces multiply.
            2 => "print -r -- {a,b}{1,2}".to_string(),
            3 => "print -r -- {a,b}{c,d}{e,f}".to_string(),
            // Nested.
            4 => "print -r -- {a,{b,c},d}".to_string(),
            5 => "print -r -- x{a,b{c,d}}y".to_string(),
            // Numeric ranges, including descending.
            6 => "print -r -- {1..5}".to_string(),
            7 => "print -r -- {5..1}".to_string(),
            8 => "print -r -- {-2..2}".to_string(),
            // Zero padding is inferred from the endpoints.
            9 => "print -r -- {01..10}".to_string(),
            10 => "print -r -- {001..003}".to_string(),
            // A step field.
            11 => "print -r -- {1..10..3}".to_string(),
            12 => "print -r -- {10..1..3}".to_string(),
            // Character ranges.
            13 => "print -r -- {a..e}".to_string(),
            14 => "print -r -- {e..a}".to_string(),
            // NOT expansions: no comma, no range — stays literal.
            15 => "print -r -- {abc}; print -r -- {}; print -r -- {a}".to_string(),
            // An empty alternative is a real (empty) word.
            16 => "print -r -- x{,a}y; print -rl -- {a,,b}; print -r -- END".to_string(),
            // Braces are expanded before parameters, so a `$var` holding
            // `{a,b}` does NOT re-expand (without GLOB_SUBST).
            _ => "v='{a,b}'; print -r -- $v; print -r -- \"{a,b}\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// getopts generator
//
// `getopts` is a stateful parse loop: $OPTIND indexes the next word, $OPTARG
// carries the argument, a leading `:` in the option string switches to silent
// error reporting (`?`/`:` land in the NAME with OPTARG set), clustered flags
// (`-ab`) come out one per call, and `--` ends the options. Every one of those
// is an interaction, which is what makes it worth fuzzing rather than unit
// testing. C: Src/builtin.c bin_getopts().
// ---------------------------------------------------------------------------

fn gen_getopts(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    // A fixed driver so only the option string / argv vary.
    const DRIVE: &str = "while getopts {OPTSTR} o; do print -r -- \"o=$o arg=[$OPTARG] ind=$OPTIND\"; done; print -r -- \"rest=$* ind=$OPTIND\"";
    let cases: &[(&str, &str)] = &[
        ("ab", "set -- -a -b x"),
        ("ab", "set -- -ab x"),
        ("a:b", "set -- -a val -b"),
        ("a:b", "set -- -aval -b"),
        // Missing argument: loud vs silent.
        ("a:", "set -- -a"),
        (":a:", "set -- -a"),
        // Unknown option: loud vs silent.
        ("ab", "set -- -z"),
        (":ab", "set -- -z"),
        // `--` terminates.
        ("ab", "set -- -a -- -b"),
        // A non-option word stops the scan.
        ("ab", "set -- -a plain -b"),
        // No options at all.
        ("ab", "set -- plain"),
        // Optind is resettable and the loop can be run twice.
        ("ab", "set -- -a -b"),
    ];
    let (optstr, setup) = pick(&mut rng, cases);
    let mut stmts = vec![setup.to_string()];
    stmts.push(DRIVE.replace("{OPTSTR}", optstr));
    // Half the cases re-run the loop after resetting OPTIND, which is the
    // documented way to parse a second argument list.
    if rng.gen_bool(0.4) {
        stmts.push("OPTIND=1".to_string());
        stmts.push(DRIVE.replace("{OPTSTR}", optstr));
    }
    stmts
}

// ---------------------------------------------------------------------------
// assoc generator
//
// Associative arrays: the (k)/(v)/(kv) flags, key-ordered iteration, the search
// subscripts ((i)/(I)/(r)/(R)) over KEYS vs VALUES, element append/unset, and
// `${(@)h}` vs `$h`. Every whole-table read goes through an ordering flag —
// hash order is undefined, and an unordered dump would report a divergence that
// is nothing but the hash seed. C: Src/params.c (assoc GSU), Src/subst.c flags.
// ---------------------------------------------------------------------------

const ASSOC_STATE: &str = "typeset -A h; h=(one 1 two 2 three 3 four 4); typeset -A e; e=()";

fn gen_assoc(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec![ASSOC_STATE.to_string()];
    for _ in 0..rng.gen_range(2..=4) {
        let stmt = match rng.gen_range(0..18) {
            // Ordered dumps — never an unordered one.
            0 => "print -r -- \"${(ok)h}\"".to_string(),
            1 => "print -r -- \"${(Ok)h}\"".to_string(),
            2 => "print -r -- \"${(kv)h[(I)*]}\" | tr ' ' '\\n' | sort | tr '\\n' ' '; print -r -- END".to_string(),
            3 => "for k in ${(ok)h}; do print -r -- \"$k=$h[$k]\"; done".to_string(),
            // Values, sorted so the order is defined.
            4 => "print -r -- \"${(on)${(v)h}}\"".to_string(),
            // Single-key read / membership / count.
            5 => "print -r -- \"[$h[two]] ${+h[two]} ${+h[nope]} n=${#h}\"".to_string(),
            // A missing key is empty, not an error.
            6 => "print -r -- \"[$h[nope]] rc=$?\"".to_string(),
            // Element assign, append and unset.
            7 => "h[five]=5; print -r -- \"${(ok)h} n=${#h}\"".to_string(),
            8 => "h[one]+=X; print -r -- \"[$h[one]]\"".to_string(),
            9 => "unset 'h[two]'; print -r -- \"${(ok)h} n=${#h}\"".to_string(),
            // Whole-table append via +=.
            10 => "h+=(six 6); print -r -- \"${(ok)h} n=${#h}\"".to_string(),
            // Search subscripts: (i)/(I) match KEYS, (r)/(R) match VALUES.
            11 => "print -r -- \"[${h[(i)two]}] [${h[(r)2]}]\"".to_string(),
            12 => "print -rl -- ${(o)h[(I)t*]} | sort | tr '\\n' ' '; print -r -- END".to_string(),
            13 => "print -rl -- ${(o)h[(R)[0-9]]} | sort | tr '\\n' ' '; print -r -- END".to_string(),
            // The empty assoc.
            14 => "print -r -- \"n=${#e} [${(ok)e}] ${+e[x]}\"".to_string(),
            // (@) keeps elements separate; a bare $h joins with spaces.
            15 => "print -r -- \"n=${#${(@v)h}}\"".to_string(),
            // Keys containing spaces survive a round trip.
            16 => "typeset -A s; s=('a b' 'v 1'); print -r -- \"[${s[a b]}] n=${#s}\"".to_string(),
            // ${(t)} reports the type.
            _ => "print -r -- \"${(t)h}\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// casesel generator
//
// `case` pattern selection and its three terminators: `;;` stops, `;&` falls
// through to the NEXT body unconditionally, and `;|` re-tests the remaining
// patterns. Plus alternation, the `(pat)` form, quoting (a quoted pattern is
// literal), and the exit status of a case with no match.
// C: Src/exec.c execcase(), Src/parse.c par_case().
// ---------------------------------------------------------------------------

fn gen_casesel(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let subjects = ["abc", "a", "xyz", "", "a b", "A", "123"];
    let s = pick(&mut rng, &subjects);
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(1..=2) {
        let stmt = match rng.gen_range(0..12) {
            0 => format!("case '{s}' in a*) print -r -- STAR;; a) print -r -- EXACT;; *) print -r -- OTHER;; esac"),
            // Alternation.
            1 => format!("case '{s}' in a|b|abc) print -r -- ALT;; *) print -r -- NO;; esac"),
            // `;&` falls through unconditionally.
            2 => format!("case '{s}' in a*) print -r -- ONE;& x*) print -r -- TWO;; *) print -r -- THREE;; esac"),
            // `;|` re-tests the remaining patterns.
            3 => format!("case '{s}' in a*) print -r -- A;| *c) print -r -- C;; *) print -r -- REST;; esac"),
            // No match at all: status 0, no output.
            4 => format!("case '{s}' in zzz) print -r -- Z;; esac; print -r -- \"rc=$?\""),
            // The leading-paren form.
            5 => format!("case '{s}' in (a*) print -r -- P1;; (*) print -r -- P2;; esac"),
            // A quoted pattern is a literal.
            6 => format!("case '{s}' in '*') print -r -- LITSTAR;; *) print -r -- GLOB;; esac"),
            // Empty subject vs `*` (which matches it).
            7 => format!("case '{s}' in '') print -r -- EMPTY;; *) print -r -- NONEMPTY;; esac"),
            // Character classes and extended-glob forms in patterns.
            8 => format!("setopt extendedglob; case '{s}' in [0-9]##) print -r -- NUM;; [a-z]##) print -r -- LOWER;; *) print -r -- OTHER;; esac"),
            // The body's status is the case's status.
            9 => format!("case '{s}' in *) false;; esac; print -r -- \"rc=$?\""),
            // A pattern containing a parameter.
            10 => format!("p='a*'; case '{s}' in $~p) print -r -- MATCH;; *) print -r -- NO;; esac"),
            // Nested case.
            _ => format!("case '{s}' in a*) case '{s}' in *c) print -r -- INNER;; *) print -r -- OUTER;; esac;; *) print -r -- NONE;; esac"),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// default generator
//
// The parameter default / alternate / assign / error family, which every
// config script leans on:
//   ${x-w} ${x:-w}   use w when x is unset (`-`) / unset-or-empty (`:-`)
//   ${x=w} ${x:=w}   the same, AND assign w back to x
//   ${x+w} ${x:+w}   use w when x IS set / set-and-nonempty
//   ${x?m} ${x:?m}   error with message m when x is unset / unset-or-empty
// The colon is the whole subtlety: it folds "empty" in with "unset". Probed
// across all three states (unset, empty, set) so each branch is hit.
// C: Src/subst.c (the `-`/`+`/`=`/`?` arms of paramsubst).
// ---------------------------------------------------------------------------

fn gen_default(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    // Three starting states for the tested parameter.
    let state = match rng.gen_range(0..3) {
        0 => "unset x",             // unset
        1 => "x=''",                // empty
        _ => "x=set",               // non-empty
    };
    let mut stmts = vec![state.to_string()];
    for _ in 0..rng.gen_range(2..=4) {
        let stmt = match rng.gen_range(0..16) {
            // `-` vs `:-`.
            0 => "print -r -- \"[${x-DEF}]\"".to_string(),
            1 => "print -r -- \"[${x:-DEF}]\"".to_string(),
            // `+` vs `:+`.
            2 => "print -r -- \"[${x+ALT}]\"".to_string(),
            3 => "print -r -- \"[${x:+ALT}]\"".to_string(),
            // `=` / `:=` assign back — observe both the expansion and x after.
            4 => "print -r -- \"[${x=ASG}]\"; print -r -- \"x=[$x]\"".to_string(),
            5 => "print -r -- \"[${x:=ASG}]\"; print -r -- \"x=[$x]\"".to_string(),
            // `?` / `:?` error path (status + message on stderr).
            6 => "( print -r -- \"[${x?missing}]\" ) 2>/dev/null; print -r -- \"rc=$?\"".to_string(),
            7 => "( print -r -- \"[${x:?empty or unset}]\" ) 2>/dev/null; print -r -- \"rc=$?\"".to_string(),
            // The replacement word is itself an expansion.
            8 => "y=fallback; print -r -- \"[${x:-$y}]\"".to_string(),
            9 => "print -r -- \"[${x:-$(print -n sub)}]\"".to_string(),
            // Nested defaults.
            10 => "print -r -- \"[${x:-${y:-inner}}]\"".to_string(),
            // The word undergoes the usual expansions (but no split in DQ).
            11 => "y='a b'; print -r -- \"[${x:-$y}]\"".to_string(),
            // Length of a defaulted value.
            12 => "print -r -- \"n=${#x:-abc}\"".to_string(),
            // A default in the middle of a larger word.
            13 => "print -r -- \"pre-${x:-mid}-post\"".to_string(),
            // `:+` guard idiom (append only when set).
            14 => "print -r -- \"${x:+prefix:}rest\"".to_string(),
            // Assignment default reflects into a later plain read.
            _ => "v=${x:=filled}; print -r -- \"v=[$v] x=[$x]\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// anonfn generator
//
// Anonymous functions: `() { body } args` defines and immediately calls a
// function with no name. Inside, `$1..$N`/`$#`/`$@` are the args, `$0` is
// "(anon)", and local declarations are scoped to the call. They also take
// redirections (`() { … } > file`) and compose with other constructs. This is
// a zsh-native idiom (the `{ … }` block that takes positional args), distinct
// from a plain group. C: Src/exec.c execfuncdef (the anonymous branch).
// ---------------------------------------------------------------------------

fn gen_anonfn(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(1..=2) {
        let stmt = match rng.gen_range(0..12) {
            0 => "() { print -r -- \"n=$# 1=$1 2=$2\" } a b c".to_string(),
            // $@ / $* inside.
            1 => "() { print -rl -- \"$@\"; print -r -- END } x y z".to_string(),
            2 => "() { print -r -- \"[$*]\" } one two".to_string(),
            // $0 is (anon).
            3 => "() { print -r -- \"[$0]\" }".to_string(),
            // A local is scoped to the call.
            4 => "v=outer; () { local v=inner; print -r -- \"[$v]\" }; print -r -- \"[$v]\"".to_string(),
            // No args.
            5 => "() { print -r -- \"n=$#\" }".to_string(),
            // A return value / status.
            6 => "() { return 3 }; print -r -- \"rc=$?\"".to_string(),
            // Args that need splitting / quoting.
            7 => "() { print -r -- \"[$1][$2]\" } 'a b' c".to_string(),
            // A redirection on the anon function.
            8 => "() { print -r -- inside } >/dev/null; print -r -- after".to_string(),
            // Loop inside.
            9 => "() { for a; do print -r -- \"<$a>\"; done } p q r".to_string(),
            // Nested anon.
            10 => "() { () { print -r -- inner } } ".to_string(),
            // Arguments from an expansion.
            _ => "arr=(1 2 3); () { print -r -- \"n=$# sum=$(( $1 + $2 + $3 ))\" } $arr".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// printv generator
//
// The `print` builtin's OUTPUT-shaping flags, which zsh piles onto one command:
// -v assigns to a parameter instead of stdout, -f is a printf format, -l is
// one-per-line, -c/-C lay out in columns, -a fills across (with -C), -o/-O sort,
// -N nul-separates, -r is raw, -R BSD-echo-compatible, -n suppresses the
// newline, -s pushes to history (skipped — stateful), -D/-P do directory /
// prompt expansion. Combinations are where the ordering rules bite.
// C: Src/builtin.c bin_print().
// ---------------------------------------------------------------------------

fn gen_printv(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(2..=4) {
        let stmt = match rng.gen_range(0..18) {
            // -v: assign the joined args to a scalar.
            0 => "print -v x -- one two three; print -r -- \"[$x]\"".to_string(),
            1 => "print -v x hi; print -r -- \"[$x]\"".to_string(),
            // -v with -l joins on newline into the scalar.
            2 => "print -v x -l a b c; print -r -- \"[$x]\"".to_string(),
            // printf -v.
            3 => "printf -v y '%s=%d' k 7; print -r -- \"[$y]\"".to_string(),
            4 => "printf -v y '%05.2f' 3.14159; print -r -- \"[$y]\"".to_string(),
            // -f format string with cycling over extra args.
            5 => "print -f '%s\\n' a b c".to_string(),
            6 => "print -f '[%d]' 1 2 3; print".to_string(),
            // -l one per line.
            7 => "print -l a b c".to_string(),
            // -o / -O sort ascending / descending.
            8 => "print -o banana apple cherry".to_string(),
            9 => "print -O 3 1 2 10".to_string(),
            // -n suppress newline; -r raw.
            10 => "print -n a; print -n b; print".to_string(),
            11 => "print -r -- 'a\\tb'".to_string(),
            // -N nul-separated (piped through tr for a visible form).
            12 => "print -N a b c | tr '\\0' ','; print -r -- END".to_string(),
            // -c column layout (fixed width so it's deterministic).
            13 => "print -c a b c d e f".to_string(),
            // -m: only args matching a pattern.
            14 => "print -m 'a*' apple banana avocado cherry".to_string(),
            // -s to history is stateful — instead test -u to an fd.
            15 => "print -u 1 -- fd1".to_string(),
            // -f with width/precision and a string.
            16 => "print -f '%-6s|%6s|\\n' ab cd".to_string(),
            // -v accumulates nothing extra: empty args → empty scalar.
            _ => "print -v x; print -r -- \"e=${#x}\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// globanchor generator
//
// The extended-glob PATTERN features the `pattern` mode does not reach: the
// `(#s)` / `(#e)` string-start / string-end anchors, the `(#cN,M)` counted
// closure, case flags `(#i)` / `(#l)`, and the exclusion `x~y` ("x but not y")
// / negation `^x` operators. Exercised through `[[ = ]]` and `${var//…}` so the
// output is a deterministic match verdict or substitution, never a filename.
// C: Src/pattern.c (PAT_START/PAT_END, the (#…) flag parser).
// ---------------------------------------------------------------------------

fn gen_globanchor(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let subj = ["abc", "aaa", "abab", "xyz", "a1b2", "AbC", "", "hello"];
    let s = pick(&mut rng, &subj);
    let mut stmts = vec!["setopt extendedglob".to_string()];
    for _ in 0..rng.gen_range(1..=3) {
        let stmt = match rng.gen_range(0..16) {
            // (#s) start anchor inside a larger pattern.
            0 => format!("[[ '{s}' = (#s)a* ]] && print -r -- Y || print -r -- N"),
            1 => format!("[[ '{s}' = *(#e) ]] && print -r -- Y || print -r -- N"),
            // Anchors in a substitution: (#s) only matches at the very start.
            2 => format!("v='{s}'; print -r -- \"${{v//(#s)a/X}}\""),
            3 => format!("v='{s}'; print -r -- \"${{v//a(#e)/X}}\""),
            // Counted closures (#cN,M).
            4 => format!("[[ '{s}' = a(#c2,3) ]] && print -r -- Y || print -r -- N"),
            5 => format!("[[ '{s}' = (a(#c1,2))* ]] && print -r -- Y || print -r -- N"),
            6 => format!("[[ '{s}' = [a-z](#c3) ]] && print -r -- Y || print -r -- N"),
            // Case-insensitive / lowering flags.
            7 => format!("[[ '{s}' = (#i)ABC ]] && print -r -- Y || print -r -- N"),
            8 => format!("v='{s}'; print -r -- \"${{v//(#i)a/X}}\""),
            // Exclusion `x~y`: matches x but not y.
            9 => format!("[[ '{s}' = [a-z]##~xyz ]] && print -r -- Y || print -r -- N"),
            10 => format!("[[ '{s}' = ??? ~ x* ]] && print -r -- Y || print -r -- N"),
            // Negation `^x`.
            11 => format!("[[ '{s}' = ^*b* ]] && print -r -- Y || print -r -- N"),
            12 => format!("[[ '{s}' = ^abc ]] && print -r -- Y || print -r -- N"),
            // `#`/`##` closures (one-or-more / zero-or-more).
            13 => format!("[[ '{s}' = a#b#c# ]] && print -r -- Y || print -r -- N"),
            14 => format!("v='{s}'; print -r -- \"${{v//[a-z]##/<&>}}\""),
            // Combining an anchor with a class.
            _ => format!("[[ '{s}' = (#s)[a-z]##(#e) ]] && print -r -- Y || print -r -- N"),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// whence generator
//
// Command resolution: `whence` / `type` / `where` / `command -v` / `-V`, and
// the flag matrix (-w kind, -v verbose, -c csh-style, -m pattern, -a all,
// -s resolve symlinks). The answer depends on WHAT the name is — a function, an
// alias (plain / global / suffix), a builtin, a reserved word, a hashed
// command — and those categories are the whole contract. Kept deterministic by
// probing shell-defined names, not $PATH lookups (a resolved path is machine
// state, not a parity property).
// C: Src/builtin.c bin_whence().
// ---------------------------------------------------------------------------

const WHENCE_SETUP: &str =
    "f() { : }; alias al='print hi'; alias -g GL='| cat'; alias -s sfx='run'";

fn gen_whence(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts = vec![WHENCE_SETUP.to_string()];
    // Names whose resolution is fully shell-determined (no filesystem).
    let names = ["f", "al", "GL", "sfx", "print", "if", "typeset", "nosuchname", "[["];
    for _ in 0..rng.gen_range(2..=4) {
        let n = pick(&mut rng, &names);
        let stmt = match rng.gen_range(0..14) {
            // -w: the KIND (function/alias/builtin/reserved/command/none).
            0 => format!("whence -w {n}"),
            // Verbose sentence form.
            1 => format!("whence -v {n} 2>/dev/null || print -r -- \"rc=$?\""),
            // Bare whence: the definition / name / nothing.
            2 => format!("whence {n} 2>/dev/null; print -r -- \"rc=$?\""),
            // `type` is whence -v; `type -w` is whence -w.
            3 => format!("type {n} 2>/dev/null || print -r -- \"rc=$?\""),
            4 => format!("type -w {n}"),
            // command -v / -V.
            5 => format!("command -v {n} 2>/dev/null; print -r -- \"rc=$?\""),
            6 => format!("command -V {n} 2>/dev/null || print -r -- \"rc=$?\""),
            // -a: every resolution of the name.
            7 => format!("whence -a {n} 2>/dev/null; print -r -- \"rc=$?\""),
            // -m with a pattern. The output of a pattern that reaches external
            // commands is in command-hash-table order (an internal detail, not a
            // parity property, like unsorted assoc iteration), so sort it — only
            // the matched SET is meaningful.
            8 => "whence -wm 'f'; whence -wm 'a*' | sort".to_string(),
            // functions / aliases introspection.
            9 => "print -r -- \"${+functions[f]} ${+aliases[al]} ${+galiases[GL]} ${+saliases[sfx]}\"".to_string(),
            // A function shadowing a builtin resolves to the function.
            10 => "print() { builtin print SHADOW }; whence -w print; unfunction print; whence -w print".to_string(),
            // `where` lists all locations (whence -ca form) — probe on a builtin.
            11 => format!("where {n} 2>/dev/null; print -r -- \"rc=$?\""),
            // Disabling a builtin changes its resolution.
            12 => "disable print 2>/dev/null; whence -w print; enable print; whence -w print".to_string(),
            // A reserved word.
            _ => "whence -w while; whence -w do; whence -w '{'".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// zstyle generator
//
// The zstyle database: `zstyle CONTEXT style value...` defines, and the query
// forms read it back — `-g` (get into scalar), `-a`/`-b`/`-t`/`-T` (array /
// boolean / test), `-s` (join with sep), `-m` (pattern-match a value), `-e`
// (evaluated value), `-L` (list), `-d` (delete). Lookup is by MOST-SPECIFIC
// matching context pattern, which is the part that carries real logic.
// C: Src/Modules/zutil.c bin_zstyle().
// ---------------------------------------------------------------------------

fn gen_zstyle(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stmts: Vec<String> = Vec::new();
    for _ in 0..rng.gen_range(1..=3) {
        let stmt = match rng.gen_range(0..14) {
            // Define + get.
            0 => "zstyle ':x:y' color blue; zstyle -g v ':x:y' color; print -r -- \"[$v] rc=$?\"".to_string(),
            // Pattern context — most-specific wins.
            1 => "zstyle ':a:*' k general; zstyle ':a:b' k specific; zstyle -g v ':a:b' k; print -r -- \"[$v]\"".to_string(),
            2 => "zstyle ':p:*' k star; zstyle -g v ':p:q' k; print -r -- \"[$v] rc=$?\"".to_string(),
            // -a: array value.
            3 => "zstyle ':c' list a b c; zstyle -a ':c' list arr; print -r -- \"n=${#arr} [${(j:,:)arr}]\"".to_string(),
            // -b: boolean into scalar (true/false).
            4 => "zstyle ':d' flag yes; zstyle -b ':d' flag b; print -r -- \"[$b] rc=$?\"".to_string(),
            5 => "zstyle ':d' flag off; zstyle -b ':d' flag b; print -r -- \"[$b] rc=$?\"".to_string(),
            // -t / -T: test a boolean/value.
            6 => "zstyle ':e' on true; zstyle -t ':e' on; print -r -- \"rc=$?\"".to_string(),
            7 => "zstyle ':e' v x; zstyle -t ':e' v x; print -r -- \"rc=$?\"; zstyle -t ':e' v y; print -r -- \"rc=$?\"".to_string(),
            // -s: join into a scalar with a separator.
            8 => "zstyle ':f' parts a b c; zstyle -s ':f' parts s '+'; print -r -- \"[$s]\"".to_string(),
            // -m: pattern-match a value in the style.
            9 => "zstyle ':g' words foo bar baz; zstyle -m ':g' words 'ba*'; print -r -- \"rc=$?\"".to_string(),
            // A missing style: nonzero status, target untouched.
            10 => "v=PRE; zstyle -g v ':none' missing; print -r -- \"rc=$? [$v]\"".to_string(),
            // Delete then re-query.
            11 => "zstyle ':h' k val; zstyle -d ':h' k; zstyle -g v ':h' k; print -r -- \"rc=$?\"".to_string(),
            // Overwrite replaces the value.
            12 => "zstyle ':i' k first; zstyle ':i' k second; zstyle -g v ':i' k; print -r -- \"[$v]\"".to_string(),
            // Several styles under one context.
            _ => "zstyle ':j' s1 a; zstyle ':j' s2 b; zstyle -g v1 ':j' s1; zstyle -g v2 ':j' s2; print -r -- \"[$v1][$v2]\"".to_string(),
        };
        stmts.push(stmt);
    }
    stmts
}

// ---------------------------------------------------------------------------
// atflag generator
//
// The `(@)` word-flag (nojoin=2) and its composition with subscripts, outer
// length/join operators, and nested expansions. The load-bearing zsh rule
// (Src/subst.c:2915 + 3881): `(@)` forces NO-JOIN but does NOT make a scalar
// array-shaped — `isarr` comes from the value / subscript scanflags, and
// `LF_ARRAY` tracks `isarr`, not the flag. So:
//   ${#${(@)scalar}}  counts CHARACTERS   (scalar stays scalar)
//   ${#${(@)array}}   counts ELEMENTS     (array stays array, even len 1)
//   ${(@)a[N]}        picks element N      (single index applied before nojoin)
//   ${(@)a[lo,hi]}    splats the slice
//   ${(@)a[@]}        splats the whole array
// This mode stresses every combination against the reference shell.
// ---------------------------------------------------------------------------

fn gen_atflag(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    // Value declarations: (setup, name, kind) where kind picks legal subscripts.
    let decls: &[(&str, &str, u8)] = &[
        ("s=hi", "s", 0),                              // scalar, multi-char
        ("s=", "s", 0),                                // scalar, empty
        ("s=abcde", "s", 0),                           // scalar, longer
        ("s=x", "s", 0),                               // scalar, 1-char
        ("a=(only)", "a", 1),                          // array, 1 element
        ("a=(one two three)", "a", 1),                 // array, 3 elements
        ("a=()", "a", 1),                              // array, empty
        ("a=(a b c d e)", "a", 1),                     // array, 5 elements
        ("typeset -A h; h=(k v)", "h", 2),             // assoc, 1 pair
        ("typeset -A h; h=(k1 v1 k2 v2 k3 v3)", "h", 2), // assoc, 3 pairs
    ];
    let (setup, name, kind) = *pick(&mut rng, decls);

    // A flag group containing (@) plus optional companions. `(k)`/`(v)`/`(kv)`
    // are only meaningful on an ASSOC (keys/values) — on an indexed array they
    // exercise a separate (index-enumeration) subsystem, so scope them to
    // assocs and keep this mode focused on `(@)`/`(A)`/`(o)` array-SHAPE.
    let flags = match kind {
        2 => pick(
            &mut rng,
            &["(@)", "(@k)", "(@v)", "(@kv)", "(o@)", "(@o)", "(A@)"],
        ),
        _ => pick(&mut rng, &["(@)", "(@)", "(o@)", "(@o)", "(A@)", "(@)"]),
    };

    // A subscript legal for the value kind (or none).
    let sub: &str = match kind {
        0 => pick(&mut rng, &["", "", "[1]", "[2]", "[-1]", "[1,2]"]), // scalar → char subscripts
        1 => pick(
            &mut rng,
            &["", "[1]", "[2]", "[-1]", "[1,2]", "[2,-1]", "[@]", "[*]", "[(r)a]", "[(R)*]"],
        ),
        _ => pick(&mut rng, &["", "[@]", "[k1]", "[(R)v1]", "[(I)k*]"]),
    };

    // Outer operator wrapping the `${flags name sub}` expansion.
    let inner = format!("${{{flags}{name}{sub}}}");
    let expr = match rng.gen_range(0..8) {
        0 => format!("${{#{inner}}}"),                 // length (elements vs chars)
        1 => format!("${{#{}}}", nested_at(&inner)),   // nested ${#${(@)…}}
        2 => inner.clone(),                            // bare splat
        3 => format!("${{(j:,:){inner}}}"),            // join
        4 => nested_at(&inner),                        // ${(@)${(@)…}}
        5 => format!("x{inner}y"),                     // surrounded by text
        6 => format!("${{(o){inner}}}"),               // sort
        _ => format!("${{#{inner}}}"),                 // length again
    };

    // Emit both an unquoted (word-split) and a double-quoted read so the
    // isarr/LF_ARRAY shape is checked in both contexts.
    let quoted = rng.gen_bool(0.5);
    let read = if quoted {
        format!("print -rl -- \"{expr}\"")
    } else {
        format!("print -rl -- {expr}")
    };
    vec![format!("{setup}; {read}")]
}

/// Wrap an already-formed `${…}` inner in an outer `${(@)…}` for nesting.
fn nested_at(inner: &str) -> String {
    format!("${{(@){inner}}}")
}

// ---------------------------------------------------------------------------
// subexp generator
//
// Nested substitution `${ ${inner} <subscript> <op> }` — an INNER expansion
// whose result the OUTER one subscripts, flags, or measures. Covers the
// `(P)` named-reference (Src/subst.c (P) aspar) across scalar / array / assoc
// referents, plus plain `${${x}...}` nesting. This is the shape zinit/p10k
// lean on (`${${(P)mapname}[key]}`), and the family where array-vs-assoc
// shape and outer-flag application (c:2681 multsub PREFORK_SUBEXP) interact.
// ---------------------------------------------------------------------------

fn gen_subexp(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    // (setup, refname-holding-var, kind): kind 0=scalar 1=array 2=assoc.
    let decls: &[(&str, u8)] = &[
        ("s=hello", 0),
        ("s=", 0),
        ("arr=(one two three)", 1),
        ("arr=(solo)", 1),
        ("arr=()", 1),
        ("typeset -A h=(a 1 b 2 c 3)", 2),
        ("typeset -A h=(k v)", 2),
        ("typeset -A h=(x 10 y 20 z 10)", 2),
    ];
    let (setup, kind) = *pick(&mut rng, decls);
    let refname = match kind {
        0 => "s",
        1 => "arr",
        _ => "h",
    };

    // An optional inner shape-flag applied to the referent BEFORE the outer
    // expansion sees it (merged into the same `(...)` group as any `P`). Only
    // `(@)` (force-array) is used: `(o)` sort composes with a subscript via a
    // separate evaluation-ORDER concern (zsh subscripts before sorting) that
    // is its own gap class, so it's kept out of this value-semantics mode.
    let inner_flag = pick(&mut rng, &["", "", "@"]);

    // The inner expansion referencing `refname`, either directly (`${arr}`)
    // or via the (P) indirect flag through a name-holding scalar `n`. Flags
    // combine into ONE group: `${(@P)n}`, not `${(@)(P)n}`.
    let use_p = rng.gen_bool(0.6);
    let (pre_setup, inner2) = if use_p {
        let grp = format!("{inner_flag}P");
        (format!("{setup}; n={refname}"), format!("${{({grp})n}}"))
    } else if inner_flag.is_empty() {
        (setup.to_string(), format!("${{{refname}}}"))
    } else {
        (setup.to_string(), format!("${{({inner_flag}){refname}}}"))
    };

    // A subscript the outer applies to the inner result, chosen by kind.
    // Search/index-of-match subscripts (`(r)`/`(i)`/`(R)`/`(I)`) return the
    // matched value-or-INDEX, an orthogonal getindex concern, so they're
    // excluded here to keep the focus on plain index / slice / splat / key.
    let sub: &str = match kind {
        0 => pick(&mut rng, &["", "[1]", "[2]", "[-1]", "[1,3]", "[2,4]"]),
        1 => pick(&mut rng, &["", "[1]", "[2]", "[-1]", "[1,2]", "[@]", "[*]"]),
        _ => pick(&mut rng, &["", "[a]", "[b]", "[k]", "[x]", "[@]", "[*]"]),
    };

    // An outer flag/operator wrapping `${inner sub}`. Only value-shaping ops
    // (length / join / splat / plain). The enumeration flags `(k)`/`(v)`/
    // `(kv)` and the type flag `(t)` on a nested-subscripted result are
    // distinct subsystems (key-enumeration, type-report) with their own gap
    // classes — excluded so this mode pins the VALUE of `${${…}[…]}`.
    let body = format!("{inner2}{sub}");
    let expr = match rng.gen_range(0..6) {
        0 => format!("${{{body}}}"),                 // plain
        1 => format!("${{#{body}}}"),                // length
        2 => format!("${{(@){body}}}"),              // array splat
        3 => format!("${{(j:,:){body}}}"),           // join
        _ => format!("${{{body}}}"),                 // plain
    };

    // UNQUOTED only. A DOUBLE-QUOTED nested expansion runs its inner in a
    // scalar (ssub) context that sepjoins the inner array to a scalar BEFORE
    // the outer subscript applies (c:subst.c:3901-3907) — so `"${${(P)arr}[1]}"`
    // is a CHARACTER index of the joined string, a large separate DQ-collapse
    // concern. This mode pins the unquoted `${${…}[…]}` value semantics.
    vec![format!("{pre_setup}; print -rl -- {expr}")]
}

// ---------------------------------------------------------------------------
// replace generator
//
// The `${name/pat/repl}` / `${name//pat/repl}` substitution-replacement
// engine (Src/glob.c getmatch / igetmatch, Src/subst.c:3120-3412): anchors
// (`#`/`%` and the `(#s)`/`(#e)` glob anchors), the `(S)` shortest / `(I:n:)`
// nth-match / `(B)`/`(E)` offset flags, `x#`/`x##` zero-or-more, alternation,
// and `(#b)`/`(#m)` backreferences in the replacement. Runs under
// `setopt extendedglob` so the `(#…)` operators are live.
// ---------------------------------------------------------------------------

fn gen_replace(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    // Subject strings chosen to exercise empty/edge, repeats, mixed classes.
    let subj = pick(
        &mut rng,
        &[
            "abc", "aaa", "aXbXc", "hello", "", "a1b2c3", "foobar",
            "xx", "abcabc", "aabbcc", "path/to/file", "CamelCase",
        ],
    );
    // Pattern body (no anchor). Some use extendedglob operators.
    let pat = pick(
        &mut rng,
        &[
            "a", "X", "?", "*", "l", "[0-9]", "[a-z]", "a#", "a##", "b#",
            "(a|b)", "(a|ab)", "o", "x", "?", ".", "[A-Z]", "c",
            "(#s)", "(#e)", "(#m)a", "(#b)(a)", "(#b)([a-z])", "a*c",
        ],
    );
    // Optional anchor. A `#`/`%` char-anchor and a `(#s)`/`(#e)` op-anchor are
    // mutually exclusive with an anchor-op PATTERN — combining e.g. `#` with a
    // `(#e)` pattern is a self-contradiction (start AND end) that just tests
    // error/no-match parity, not the replacement engine, so skip it.
    let pat_is_anchor_op = pat.starts_with("(#s)") || pat.starts_with("(#e)");
    let anchor = if pat_is_anchor_op {
        &""
    } else {
        pick(&mut rng, &["", "", "#", "%", "(#s)", "(#e)"])
    };
    // Replacement text. Backreference forms only make sense with (#b)/(#m).
    let repl = if pat.contains("(#b)") {
        pick(&mut rng, &["[${match[1]}]", "<${match[1]}>", "${match[1]}${match[1]}", "-"])
    } else if pat.contains("(#m)") {
        pick(&mut rng, &["<$MATCH>", "${#MATCH}", "$MBEGIN-$MEND", "[$MATCH]"])
    } else {
        pick(&mut rng, &["-", "X", "", "&", "_", "[&]", "YY"])
    };
    // Global vs single, and an optional leading flag group. `(I:N:)` nth-match
    // and `(B)`/`(E)` offset flags are deliberately excluded: on the single-`/`
    // path they need the shared `patmatch` refactor (the window scan can't
    // reproduce the engine's nth zero-width / anchored match bookkeeping) and
    // would flood the gate. `(S)` shortest is kept (works except when combined
    // with a `#`/`%` char-anchor + a zero-or-more body — filtered below).
    let slash = if rng.gen_bool(0.6) { "//" } else { "/" };
    let flag = pick(&mut rng, &["", "", "", "(S)", "(M)"]);

    // Compose `${flag name slash anchor pat / repl}`. The anchor is either a
    // `#`/`%` char prefix on the pattern or a `(#s)`/`(#e)` operator spliced in.
    let (a_char, a_op): (&str, &str) = match *anchor {
        "#" | "%" => (anchor, ""),
        "(#s)" | "(#e)" => ("", anchor),
        _ => ("", ""),
    };
    // `(S)` shortest combined with a `#`/`%` char-anchor AND a zero-or-more
    // body (`*`/`x#`/`x##`) selects the shortest span at the anchor, which the
    // single-`/` window scan gets wrong — same patmatch-refactor class as the
    // (I:N:) cases, so drop the flag for that specific shape.
    let body_zero_or_more = pat.contains('*') || pat.contains('#');
    let flag = if *flag == "(S)" && (a_char == "#" || a_char == "%") && body_zero_or_more {
        ""
    } else {
        *flag
    };
    let expr = format!("${{{flag}s{slash}{a_char}{a_op}{pat}/{repl}}}");
    let quoted = rng.gen_bool(0.5);
    let read = if quoted {
        format!("print -rl -- \"{expr}\"")
    } else {
        format!("print -rl -- {expr}")
    };
    vec![format!("setopt extendedglob; s={subj}; {read}")]
}

// ---------------------------------------------------------------------------
// assign generator
//
// Assignment INSIDE a parameter expansion: `${name=word}` / `${name:=word}` /
// `${name::=word}` (set default & assign) with the `(A)`/`(AA)` array/assoc
// flags, the `=` (SH_WORD_SPLIT) and `(s:X:)` split flags, and various RHS
// shapes. Src/subst.c:3245-3307 (assignsparam / setaparam / sethparam),
// c:3272 sepsplit-on-`spsep||spbreak`. The result is read back via the target
// so the array/assoc SHAPE and element count are what's compared.
// ---------------------------------------------------------------------------

fn gen_assign(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    // RHS value: a plain string (may contain spaces / separators / empties).
    let rhs = pick(
        &mut rng,
        &[
            "1 2 3", "a b c d", "x", "", "k1 v1 k2 v2", "a:b:c", "one  two",
            "p q r s t", "single", "a b", "1 2 3 4 5 6",
        ],
    );
    // Deliver the RHS via a variable (so the `=` flag word-splits $v) or
    // literally in the expansion.
    let via_var = rng.gen_bool(0.6);
    let (pre, word) = if via_var {
        (format!("v={:?}; ", rhs), "$v".to_string())
    } else {
        (String::new(), (*rhs).to_string())
    };

    // Flags: array/assoc + optional `=` split or `(s:X:)` separator.
    let flag = pick(
        &mut rng,
        &[
            "", "(A)", "(A)=", "(AA)", "(AA)=", "(A)", "(As.:.)", "(AAs.:.)",
            "=", "(A)=", "",
        ],
    );
    // Assign operator (all "assign if unset/empty" or unconditional).
    let op = pick(&mut rng, &["::=", ":=", "="]);
    let target = "out";

    // Read the result back in a shape that exposes the array/assoc element
    // count and a couple of members.
    let readback = if flag.contains("AA") {
        format!("print -r -- \"[${{(kv){target}}}]\"")
    } else if flag.contains('A') {
        format!("print -r -- \"n=${{#{target}}} [${{{target}[1]}}][${{{target}[2]}}]\"")
    } else {
        format!("print -r -- \"[${{{target}}}]\"")
    };

    // `unset out` first so `:=`/`::=` fire deterministically.
    let prog = format!(
        "unset {target} 2>/dev/null; {pre}: ${{{flag}{target}{op}{word}}}; {readback}"
    );
    vec![prog]
}

// ---------------------------------------------------------------------------
// gflag generator
//
// `${(g:opts:)v}` — run the value through getkeystring(), the shared escape
// decoder (c:Src/utils.c:6915). The opts letters map to GETKEY_* bits at
// c:Src/subst.c:2411-2425: `e` → GETKEY_EMACS, `o` → GETKEY_OCTAL_ESC,
// `c` → GETKEY_CTRL; empty opts → 0. The bits are NOT independent — the
// octal branch (c:7156-7178) is entered for any `0-7`/`x` and then re-checks
// OCTAL_ESC, so `\1` under `e`-without-`o` takes the `*t++ = '\\', s--;
// continue;` arm (a LITERAL backslash, unsuppressed by EMACS) while `\1`
// under `o` is a byte. That cross-bit coupling is what this mode exists to
// hammer: every opts subset × every escape shape.
//
// Determinism: pure string→string, no clock/pid/filesystem. Output is piped
// through `od` because getkeystring legitimately emits raw non-UTF-8 bytes
// (`\xff`, `\M-a`, octal >\177), and a bare print of two DIFFERENT invalid
// byte strings would render identically in a report.
// ---------------------------------------------------------------------------

/// Every subset of the `g` opts letters, plus repeats/orderings (the C parse
/// is a per-char bit-set loop, so `ec` and `ce` must agree, and a doubled
/// letter must be idempotent).
const GFLAG_OPTS: &[&str] = &[
    "", "e", "o", "c", "oe", "eo", "ce", "ec", "co", "oc", "coe", "ceo", "eoc", "oce", "ee", "oo",
];

/// Escape shapes spanning every getkeystring branch. Written as they appear
/// INSIDE a double-quoted zsh string, so `\` reaches the value literally.
const GFLAG_ESCAPES: &[&str] = &[
    // Simple letter escapes (c:7000-7018) — flag-independent.
    "a", "\\a", "\\b", "\\e", "\\E", "\\f", "\\n", "\\r", "\\t", "\\v",
    // Octal, leading NON-zero: the OCTAL_ESC-gated branch (c:7157-7161).
    // Without `o` these must come back as a literal backslash + digits.
    "\\1", "\\7", "\\10", "\\77", "\\101", "\\102x", "\\377", "\\400", "\\600", "\\777",
    // Octal, leading zero: taken as the introducer when OCTAL_ESC is unset
    // (c:7158 `if (*s == '0') s++`), so `\0101` is 'A' even with no `o`.
    "\\0", "\\00", "\\0101", "\\0377", "\\0400", "\\0777", "\\08", "\\09z",
    // Digits >= 8 are NOT octal (c:7156 `*s < '8'`) — default arm.
    "\\8", "\\9", "\\8a",
    // Hex (c:7169) — entered regardless of OCTAL_ESC, max 2 digits.
    "\\x", "\\x0", "\\x41", "\\x4", "\\xff", "\\xg", "\\x41B", "\\xfff",
    // Unicode (c:7072-7138) — ungated, writes UTF-8 not a raw byte.
    "\\u0041", "\\u00e9", "\\u20ac", "\\U0001F600", "\\u", "\\uzz",
    // Emacs key escapes (c:7029-7052), gated on GETKEY_EMACS.
    "\\M-a", "\\C-a", "\\C-A", "\\M-\\C-a", "\\C-?", "\\C-@", "\\C-[", "\\M-", "\\C-",
    "\\M-\\M-b", "\\C-\\C-c",
    // `^X` control form (c:7194), gated on GETKEY_CTRL.
    "^A", "^a", "^?", "^@", "^", "a^Bb", "^[",
    // Unknown escapes — default arm (c:7180-7184): the backslash survives
    // only when EMACS is unset.
    "\\q", "\\z", "\\-", "\\.", "\\'", "\\\"", "\\\\", "\\%", "\\c", "\\ ",
    // Mixed / adjacent shapes: an escape butted against ordinary text, and
    // multi-escape strings where one arm's `s` fixup feeds the next.
    "x\\101y", "\\101\\102", "\\1\\0101", "\\x41\\101", "ab\\", "\\\\1", "\\\\x41",
    "pre\\M-a post", "\\0101\\x41\\u0041", "^A\\C-a\\1",
];

fn gen_gflag(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let opts = pick(&mut rng, GFLAG_OPTS);
    let esc = pick(&mut rng, GFLAG_ESCAPES);

    match rng.gen_range(0..6) {
        // Byte-exact decode. The core case.
        0 | 1 => vec![format!(
            "v=\"{esc}\"; print -rn -- \"${{(g:{opts}:)v}}\" | od -An -tx1 | tr -s ' '"
        )],
        // Length of the decoded result: catches a decode that emits the right
        // bytes as the WRONG number of characters (metafication slips, a
        // multibyte \u counted as its byte length).
        2 => vec![format!(
            "v=\"{esc}\"; r=${{(g:{opts}:)v}}; print -r -- \"len=${{#r}}\""
        )],
        // Decode inside a larger expansion: the result feeds a second flag, so
        // a wrong intermediate (e.g. an unmetafied high byte) surfaces as a
        // different visible rendering rather than raw bytes.
        3 => vec![format!(
            "v=\"{esc}\"; print -r -- \"[${{(V)${{(g:{opts}:)v}}}}]\""
        )],
        // Decoded value used as a pattern subject — pushes the bytes through
        // the matcher rather than straight to stdout.
        4 => vec![format!(
            "v=\"{esc}\"; r=${{(g:{opts}:)v}}; [[ -n $r ]] && print -r -- \"n=${{#r}}\" || print -r -- empty"
        )],
        // Array elements: each word decoded independently (c:subst.c:3965
        // loops getkeystring over `*ap2`), which is a different call site from
        // the scalar arm at c:3970.
        _ => vec![format!(
            "a=(\"{esc}\" \"x\" \"{esc}\"); print -rn -- \"${{(g:{opts}:)a}}\" | od -An -tx1 | tr -s ' '"
        )],
    }
}

// ---------------------------------------------------------------------------
// select generator
//
// `select name in words; do … done` — c:Src/loop.c:217 execselect. The menu
// itself is `selectlist` (c:347): column-MAJOR layout, width driven by the
// longest item plus the digit count of the item total.
//
// The C control flow is two NESTED loops, and the nesting is the whole point:
//   more = selectlist(args, 0);        // c:264 — ONCE, before the loop
//   for (;;) {                         // c:265 — per selection
//       for (;;) {                     // c:266 — read until non-empty
//           print prompt3; read str
//           if (!str) { REPLY=""; goto done; }   // c:277-285 EOF
//           if (*str) break;                     // c:288
//           more = selectlist(args, more);       // c:290 — EMPTY line reprints
//       }
//       REPLY=str; name=nth(args, atoi(str));    // c:291-300
//       execlist(body)                           // c:303
//   }
// So the menu appears once per select, plus once per blank line — never per
// body iteration. A non-numeric or out-of-range reply sets `name` to "" but
// still runs the body (c:293-299 `if (!i) str = ""`).
//
// Determinism: the menu goes to STDERR and its layout depends on the terminal
// width, and the prompt is PS3 — so each script PINS `COLUMNS`/`LINES`/`PS3`
// rather than inheriting them (the harness passes its own env through to both
// shells, and a caller exporting a colored PS3 or COLUMNS=0 would otherwise
// decide the output). stdin is always a fixed here-string/pipe, never a tty.
// ---------------------------------------------------------------------------

/// Word lists: varying counts/widths so the column math (longest item, digit
/// count, items-per-row) lands differently.
const SELECT_LISTS: &[&str] = &[
    "a b",
    "a b c",
    "one two three",
    "x",
    "alpha beta gamma delta",
    "a b c d e f g h i j k l",
    "'a b' c",
    "short longeritem s",
    "1 2 3 4 5 6 7 8 9 10 11 12 13 14 15",
    "aa bb cc dd",
    "'' x",
    "verylongsingleitemhere z",
];

/// Reply sequences fed on stdin. Cover: valid picks, out-of-range, zero,
/// negative, non-numeric, EMPTY (the c:290 reprint path), and multi-select
/// runs that expose a per-iteration redraw.
const SELECT_INPUTS: &[&str] = &[
    "1", "2", "3", "0", "9", "-1", "abc", "", "1\\n2", "\\n1", "\\n\\n2", "1\\n1",
    "2\\n\\n1", "1\\n2\\n3", "x\\n1", "10", "1 2",
];

fn gen_select(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let list = pick(&mut rng, SELECT_LISTS);
    let input = pick(&mut rng, SELECT_INPUTS);
    // Pin every terminal/prompt input the menu layout reads.
    let pin = "COLUMNS=80; LINES=24; PS3='?# '; ";
    // `break` after the first body run vs letting the loop drain stdin: the
    // draining form is what catches a menu redrawn per iteration.
    let body = pick(
        &mut rng,
        &[
            "print -r -- \"[$x]\"; break",
            "print -r -- \"[$x] r=$REPLY\"; break",
            "print -r -- \"[$x]\"",
            "print -r -- \"[$x] r=$REPLY\"",
            "print -r -- \"n=$#x\"; break",
            "break",
            "print -r -- \"[$x]\"; continue",
        ],
    );

    match rng.gen_range(0..4) {
        // `select … in <words>` with the replies piped in. stderr is folded in
        // so the MENU and PS3 prompt are compared, not just the body's stdout.
        0 | 1 => vec![format!(
            "{pin}printf '{input}\\n' | {{ select x in {list}; do {body}; done }} 2>&1; print -r -- \"rc=$?\""
        )],
        // Positional-parameter form (c:235-242 WC_SELECT_PPARAM) — the word
        // list comes from $@ rather than an explicit `in`.
        2 => vec![format!(
            "{pin}f() {{ select x; do {body}; done }}; printf '{input}\\n' | f {list} 2>&1; print -r -- \"rc=$?\""
        )],
        // Empty/undefined list — c:248-252 skips the body entirely and the
        // menu is never printed.
        _ => vec![format!(
            "{pin}a=({list}); printf '{input}\\n' | {{ select x in $a; do {body}; done }} 2>&1; print -r -- \"rc=$? n=${{#a}}\""
        )],
    }
}

// ---------------------------------------------------------------------------
// bindkey generator
//
// `bindkey` (c:Src/Zle/zle_keymap.c:1022/1038/1045/1104/1119) decodes every
// key spec with `getkeystring(seq, &len, GETKEYS_BINDKEY, NULL)` — the SAME
// decoder as the `g` flag, but with a flag set the `(g:…:)` form cannot
// produce on its own: OCTAL_ESC|EMACS|CTRL together (c:zsh.h:3187). That
// combination is what makes `^X`, `\C-x`, `\M-x`, `\101` and the chained
// `\M-^?` forms all live in one grammar, and the modifier bits interact:
// c:7034 `meta = 1 + control` records whether `\M` was seen before or after
// a pending control, which c:7261-7275 then applies in that order (`\M-\C-?`
// is 0xff, `\C-\M-?` is 0x9f).
//
// The strongest probe here is not "does it decode" but "do two DIFFERENT
// spellings of the same byte sequence land on the same binding" — bind via
// one notation, look up via an equivalent one. That catches a decoder that is
// self-consistently wrong.
//
// Determinism: bindkey output is a stable, targeted lookup — no clock, pid or
// filesystem. Full `-L` dumps are avoided (keymap iteration order is not a
// parity guarantee worth pinning); counts via `grep -c` are used instead.
// ---------------------------------------------------------------------------

/// Key specs spanning the GETKEYS_BINDKEY vocabulary. Written as they reach
/// the shell inside SINGLE quotes, so backslashes arrive literally.
const BINDKEY_SPECS: &[&str] = &[
    // Caret control form (c:7194, GETKEY_CTRL).
    "^X", "^A", "^?", "^[", "^@", "^A^B", "^X^X",
    // Backslash control form (c:7041-7052).
    "\\C-x", "\\C-?", "\\C-@", "\\C-a",
    // Meta (c:7029-7040) — sets the HIGH BIT, not an ESC prefix.
    "\\M-a", "\\M-x", "\\M--", "\\M-1",
    // Chained modifiers — the `meta = 1 + control` ordering surface.
    "\\M-^?", "\\M-^A", "\\M-^H", "\\M-\\C-a", "\\C-\\M-a", "^\\M-a", "\\M-\\M-a",
    "\\M-\\C-?", "\\C-\\M-?", "\\M-\\C-x",
    // Simple letter escapes.
    "\\e", "\\E", "\\e[A", "\\e[1;5C", "\\n", "\\t", "\\r", "\\a", "\\b", "\\f", "\\v",
    "\\M-\\n", "\\M-\\t",
    // Numeric escapes — OCTAL_ESC is SET here, so `\101` is `A` (unlike the
    // bare `(g::)` form where it stays a literal backslash).
    "\\101", "\\0101", "\\x41", "\\x7f", "\\377", "\\777", "\\400", "\\0",
    // Unicode.
    "\\u0041", "\\U00000041",
    // Plain / multi-char sequences.
    "A", "abc", "x",
];

/// Pairs of DIFFERENT spellings that must decode to the SAME bytes, so a bind
/// through one is visible through the other.
const BINDKEY_EQUIV: &[(&str, &str)] = &[
    ("^X", "\\C-x"),
    ("^A", "\\C-a"),
    ("^?", "\\C-?"),
    ("^[", "\\e"),
    ("\\e", "\\E"),
    ("\\M-^A", "\\M-\\C-a"),
    ("\\M-^?", "\\M-\\C-?"),
    ("\\101", "A"),
    ("\\0101", "A"),
    ("\\x41", "A"),
    ("\\u0041", "A"),
    ("\\x7f", "^?"),
    ("^@", "\\C-@"),
    ("\\M-\\C-a", "\\M-^A"),
];

/// Widgets that exist in both shells' default keymaps.
const BINDKEY_WIDGETS: &[&str] = &[
    "self-insert",
    "backward-kill-word",
    "up-line-or-history",
    "accept-line",
    "forward-char",
    "undefined-key",
    "beep",
    "kill-line",
];

fn gen_bindkey(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let widget = pick(&mut rng, BINDKEY_WIDGETS);

    match rng.gen_range(0..7) {
        // Bind, then read the binding back through the SAME spec. Exercises
        // decode AND the display path (c:zle_keymap.c printbind → nicechar).
        0 | 1 => {
            let spec = pick(&mut rng, BINDKEY_SPECS);
            vec![format!("bindkey '{spec}' {widget}; bindkey '{spec}'")]
        }
        // Bind via one spelling, look up via an equivalent one. A decoder that
        // is wrong the same way on both sides still fails this.
        2 => {
            let (a, b) = pick(&mut rng, BINDKEY_EQUIV);
            vec![format!("bindkey '{a}' {widget}; bindkey '{b}'")]
        }
        // Remove, then look up (c:1104 `bindkey -r`).
        3 => {
            let spec = pick(&mut rng, BINDKEY_SPECS);
            vec![format!(
                "bindkey '{spec}' {widget}; bindkey -r '{spec}'; bindkey '{spec}'"
            )]
        }
        // A user keymap (c:1119 `-N`/`-M`) keeps its own trie.
        4 => {
            let spec = pick(&mut rng, BINDKEY_SPECS);
            vec![format!(
                "bindkey -N km; bindkey -M km '{spec}' {widget}; bindkey -M km '{spec}'; bindkey '{spec}'"
            )]
        }
        // `-s` string binding (c:1038) — the RHS runs through getkeystring too.
        5 => {
            let spec = pick(&mut rng, BINDKEY_SPECS);
            let rhs = pick(&mut rng, &["hi", "\\C-a", "\\M-b", "\\101", "x y"]);
            vec![format!("bindkey -s '{spec}' '{rhs}'; bindkey -s '{spec}'")]
        }
        // Count the bindings that landed — catches a spec that decoded to the
        // WRONG NUMBER of keys (a 2-byte binding where zsh makes 1).
        _ => {
            let spec = pick(&mut rng, BINDKEY_SPECS);
            vec![format!(
                "bindkey '{spec}' {widget}; print -r -- \"n=$(bindkey -L | grep -c -- {widget})\""
            )]
        }
    }
}

// ---------------------------------------------------------------------------
// zmv generator
//
// `zmv` is a zsh autoload FUNCTION, not a builtin — its source is the spec
// (Functions/Misc/zmv). zshrs ships a native Rust impl instead, so this mode
// exists to pin that impl against the real thing. The mapping rules it has to
// reproduce, with zmv line numbers:
//
//   zmv:126     `setopt extendedglob` — `(…)` in the pattern is BOTH a glob
//               group and the capture syntax; the pattern IS the glob
//               (zmv:237-239 `fpat=$pat; files=(${~fpat})`).
//   zmv:255-257 `set -- "$match[@]"; g=${(Xe)repl}` — the captures become the
//               POSITIONALS and the replacement gets FULL parameter expansion,
//               which is why `$f` (zmv:245's loop variable) and even
//               `${f// /_}` work — not just `$1`..`$N`.
//   zmv:190-216 `-w` parenthesises the wildcards in the pattern; `-W` also
//               rewrites the replacement's wildcards to `${1}..${N}` and errors
//               when the counts disagree.
//   zmv:264-276 empty expansion / unaltered name / collision handling.
//   zmv:280-284 every substitution error is reported TOGETHER under one
//               `$myname: error(s) in substitution:` header.
//
// Determinism + safety: EVERY probe passes `-n`, so zmv prints the commands it
// would run and touches nothing. That keeps the fixture read-only (shared by
// the parallel workers) and means even a minimized probe cannot rename a file.
// ---------------------------------------------------------------------------

/// (pattern, replacement) pairs using explicit `(…)` capture groups.
const ZMV_PAREN: &[(&str, &str)] = &[
    ("(*).txt", "$1.bak"),
    ("(*).txt", "${1}.q"),
    ("(*).txt", "$1.txt"),
    ("(*).(txt|log)", "$1.$2.new"),
    ("(*).(txt|log)", "$2/$1"),
    ("(f)(o)(o).txt", "$3$2$1.txt"),
    ("(*).(*)", "$2.$1"),
    ("(?)(*).txt", "$2$1.txt"),
    ("f(<1-9>).dat", "g$1.dat"),
    ("([fb])(*).txt", "$1-$2.x"),
    ("(*) (*).txt", "$1_$2.txt"),
    ("(one).(two).txt", "$2.$1.txt"),
    ("(*).log", ""),
    ("(*).txt", "same.out"),
];

/// Patterns paired with `$f`-based replacements — the surface that only a real
/// `${(Xe)repl}` expansion can satisfy.
const ZMV_FVAR: &[(&str, &str)] = &[
    ("*.txt", "new_$f"),
    ("*.txt", "${f}.orig"),
    ("* *", "${f// /_}"),
    ("*.log", "${f:r}.out"),
    ("*.txt", "${f:u}"),
    ("*.dat", "pre-$f"),
    ("*.txt", "$f"),
];

/// Wildcard patterns for `-w` / `-W` (counts must match for `-W`).
const ZMV_WILD: &[(&str, &str)] = &[
    ("*.txt", "*.bak"),
    ("*.*", "${2}.${1}"),
    ("*.txt", "${1}.md"),
    ("?.dat", "${1}.dat2"),
    ("*.*", "*.*"),
    ("*.txt", "x-*.txt"),
];

fn gen_zmv(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    // -n is NOT optional: it is what keeps this mode read-only.
    let extra = pick(&mut rng, &["", "", "", "-v ", "-f ", "-Q "]);
    // Only `zmv` itself: a stock zsh ships zmv in $fpath but NOT zcp/zln (they
    // are usually links the distro may or may not create — this install has
    // only zmv), so a `zcp` probe compares zshrs's native impl against
    // `zcp: function definition file not found` and teaches nothing. The `-C`
    // and `-L` arms below reach the copy/link actions through zmv, which is how
    // zmv:149-151 selects them anyway.
    let action = "zmv";

    match rng.gen_range(0..6) {
        // Explicit capture groups + $N references.
        0 | 1 => {
            let (pat, repl) = pick(&mut rng, ZMV_PAREN);
            vec![format!(
                "autoload -Uz {action}; {action} -n {extra}'{pat}' '{repl}'; print -r -- \"rc=$?\""
            )]
        }
        // $f-based replacements (full ${(Xe)} expansion).
        2 => {
            let (pat, repl) = pick(&mut rng, ZMV_FVAR);
            vec![format!(
                "autoload -Uz {action}; {action} -n {extra}'{pat}' '{repl}'; print -r -- \"rc=$?\""
            )]
        }
        // -w: parenthesise the pattern's wildcards only.
        3 => {
            let (pat, repl) = pick(&mut rng, ZMV_PAREN);
            vec![format!(
                "autoload -Uz {action}; {action} -n -w {extra}'{pat}' '{repl}'; print -r -- \"rc=$?\""
            )]
        }
        // -W: pattern AND replacement wildcards.
        4 => {
            let (pat, repl) = pick(&mut rng, ZMV_WILD);
            vec![format!(
                "autoload -Uz {action}; {action} -n -W {extra}'{pat}' '{repl}'; print -r -- \"rc=$?\""
            )]
        }
        // Action overrides (-C copy / -L link / -p prog).
        _ => {
            let (pat, repl) = pick(&mut rng, ZMV_PAREN);
            let flag = pick(&mut rng, &["-C", "-L", "-M", "-p echo"]);
            vec![format!(
                "autoload -Uz zmv; zmv -n {flag} {extra}'{pat}' '{repl}'; print -r -- \"rc=$?\""
            )]
        }
    }
}

// ---------------------------------------------------------------------------
// zcalc generator
//
// `zcalc` is an autoload FUNCTION (Functions/Misc/zcalc) that zshrs intercepts
// with a native Rust impl, so — like the zmv mode — this pins the impl against
// the real thing. The rules that are easy to get wrong, with zcalc line refs:
//
//   zcalc:133 `zmodload -i zsh/mathfunc` — zcalc LOADS the math library on
//             startup. That is why `zcalc -e 'sqrt(16)'` resolves while a bare
//             `$(( sqrt(16) ))` is "unknown function: sqrt": the named-function
//             table stays empty until the module boots.
//   zcalc:99-112 `zcalc_show_value` — the result is REFORMATTED, not echoed:
//             a value containing `.` goes through `_forms[1]` = '%2$g' (so 6
//             significant digits — `atan(1)*4` is 3.14159, NOT the full
//             3.1415926535897931), EXCEPT one ending in a bare `.` which
//             prints raw (`sqrt(16)` → `4.`); no dot at all → `printf "%d"`.
//   zcalc:105 a FAILED evaluation prints its diagnostic and no value — the
//             expansion never produced one, so `1/0` writes nothing to stdout.
//
// Determinism: expressions are pure arithmetic — no rand48(), no clock, no
// filesystem. `-e` (expression mode) always, since the REPL needs a tty.
// ---------------------------------------------------------------------------

/// Expressions spanning the surfaces above. Anything whose value is not a
/// function of the text alone (rand48, time) is deliberately absent.
const ZCALC_EXPRS: &[&str] = &[
    // Integer arithmetic → printf "%d" (zcalc:110).
    "1+2", "10/4", "3 % 2", "1<<4", "2**10", "(1+2)*3", "-5+3", "2^10", "7&3", "7|8", "~5",
    "10 > 3", "1 ? 2 : 3",
    // Bases.
    "0x10", "0b101", "010",
    // Floats → %g via _forms[1] (zcalc:107).
    "10.0/4", "1.0/3", "22.0/7", "1e10", "1.5e-8", "2.0**0.5", "1.0/7", "100.0/3", "0.1+0.2",
    // zsh/mathfunc, loaded by zcalc:133. `sqrt(16)` is the bare-trailing-dot
    // case (`4.`), the rest exercise the %g path.
    "sqrt(16)", "sqrt(2)", "atan(1)*4", "sin(0)", "cos(0)", "log(1)", "exp(1)", "exp(0)",
    "ceil(1.2)", "floor(1.8)", "int(3.7)", "abs(-5)", "fabs(-5.5)", "log10(100)", "sqrt(2)*sqrt(2)",
    "atan2(1,1)", "pow(2,10)", "fmod(10,3)",
    // Error paths: the diagnostic is printed and NO value (zcalc:105).
    "1/0", "zzundefined_zz+1", "1%0",
];

fn gen_zcalc(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let e = pick(&mut rng, ZCALC_EXPRS);

    match rng.gen_range(0..4) {
        // Plain `-e EXPR`.
        0 | 1 => vec![format!(
            "autoload -Uz zcalc; zcalc -e '{e}'; print -r -- \"rc=$?\""
        )],
        // `-f` = forcefloat (zcalc:187 `setopt forcefloat`), which turns
        // integer division into float division: `3/4` is 0.75, not 0.
        2 => vec![format!(
            "autoload -Uz zcalc; zcalc -e -f '{e}'; print -r -- \"rc=$?\""
        )],
        // Several expressions in one call — each is shown independently, and
        // the forcefloat option must not leak past the call (zcalc runs under
        // `emulate -L zsh`, so its setopt is function-local).
        _ => {
            let e2 = pick(&mut rng, ZCALC_EXPRS);
            vec![format!(
                "autoload -Uz zcalc; zcalc -e '{e}' '{e2}'; print -r -- \"rc=$? ff=${{options[forcefloat]}}\""
            )]
        }
    }
}

// ---------------------------------------------------------------------------
// rcexpand generator
//
// RC_EXPAND_PARAM — the `^` flag (c:Src/subst.c:2550-2557 `plan9`) and its
// `setopt rcexpandparam` counterpart. `${^a}` cross-products the array with the
// surrounding text (`x${^a}y` → `xa y xb y`), `${^^a}` forces it back OFF, and
// C parses the flag with the SAME loop braced or not (the start guard at
// c:1890-1891 admits `Hat`), so `$^a` and `${^a}` must agree exactly.
//
// The rules this pins, each one a bug this mode's construction found:
//   * The UNBRACED `$^a` form exists at all — it used to compile to literal
//     text, which is why promptinit:23's `$^fpath/prompt_*_setup(N)` found zero
//     themes (29 of zsh's own functions use the form, `_git` among them).
//   * Inside DOUBLE QUOTES the array is JOINED before plan9 ever runs
//     (c:3029-3036 `if (qt && !getlen && isarr > 0) { val = sepjoin(...);
//     isarr = 0; }`, and the plan9 block at c:4316 sits inside the isarr arm),
//     so `"pre${^b}"` is ONE word.
//   * `${^^a}` sets plan9 = 0 and NOTHING else (c:2554) — the value keeps its
//     array shape, so it still splats one word per element.
//   * Affixes attach per element when distributing but only to the last
//     element when not.
//
// Determinism: pure array/text expansion — no clock, pid or filesystem.
// ---------------------------------------------------------------------------

/// Array values: element counts and empty/space-bearing members change the
/// cross-product's word count and the DQ join's shape.
const RCEXPAND_ARRAYS: &[&str] = &[
    "(a b)",
    "(a b c)",
    "(one)",
    "()",
    "(a '' b)",
    "('x y' z)",
    "(1 2 3 4)",
    "(d1 d2)",
];

/// Word shapes around the expansion: bare, prefix, suffix, both, and the
/// braced/unbraced pair that must agree.
///
/// The braced/unbraced pairing is the load-bearing part. C runs ONE flag loop
/// for both forms (c:1890-1891 admits the flag char at the paramsubst start,
/// c:2550-2557 parses it), so `$^a` and `${^a}` must agree exactly — whereas
/// this port reaches them through different code, which is precisely how `$^a`
/// came to compile to literal text while `${^a}` worked.
const RCEXPAND_WORDS: &[&str] = &[
    "$^a", "${^a}", "pre$^a", "pre${^a}", "$^a.x", "${^a}.x", "pre$^a.x", "pre${^a}suf",
    "$^^a", "${^^a}", "$^^a.x", "${^^a}.x", "x$^a$^a", "$^a-$^a", "$a.x", "${a}.x",
];

/// The sibling unbraced flag `=` (SH_WORD_SPLIT / spbreak, c:2558-2569), paired
/// with its braced form. Same structure as `^`: the whole-word fast path could
/// not express an affix, so `pre$=s` / `$=s.x` / `$==s.x` were literal text
/// while the bare `$=s` worked.
const RCEXPAND_SPLIT_WORDS: &[&str] = &[
    "$=s", "${=s}", "pre$=s", "pre${=s}", "$=s.x", "${=s}.x", "x$=s.y", "x${=s}.y",
    "$==s", "${==s}", "$==s.x", "${==s}.x", "$=s-$=s",
];

/// Scalar values for the `=` split surface: IFS-whitespace runs, leading and
/// trailing separators, and an all-blank value all change the word count.
const RCEXPAND_SCALARS: &[&str] = &[
    "'a b'", "'a  b'", "' a b '", "'a'", "''", "'  '", "'a b c'", "'x'",
];

fn gen_rcexpand(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let arr = pick(&mut rng, RCEXPAND_ARRAYS);
    let word = pick(&mut rng, RCEXPAND_WORDS);
    // The option is half the surface: plan9 starts from isset(RCEXPANDPARAM)
    // (c:1663), and `^`/`^^` override it either way.
    let opt = pick(&mut rng, &["", "", "setopt rcexpandparam; ", "unsetopt rcexpandparam; "]);

    match rng.gen_range(0..6) {
        // Unquoted — the distributing case. `print -rl` makes the word COUNT
        // visible, which is the whole point.
        0 | 1 => vec![format!("{opt}a={arr}; print -rl -- {word}")],
        // Double-quoted — must JOIN (c:3032), never cross-product.
        2 => vec![format!("{opt}a={arr}; print -r -- \"{word}\"")],
        // Captured into an array: pins the element COUNT rather than the
        // rendered text.
        3 => vec![format!(
            "{opt}a={arr}; r=({word}); print -r -- \"n=${{#r}} [${{r[1]}}][${{r[2]}}]\""
        )],
        // The `=` split flag — same braced/unbraced pairing as `^`. `shwordsplit`
        // matters here the way `rcexpandparam` does for `^`: c:1705 seeds spbreak
        // from the option, and `=`/`==` override it either way.
        4 => {
            let w = pick(&mut rng, RCEXPAND_SPLIT_WORDS);
            let v = pick(&mut rng, RCEXPAND_SCALARS);
            let so = pick(&mut rng, &["", "", "setopt shwordsplit; ", "unsetopt shwordsplit; "]);
            vec![format!("{so}s={v}; print -rl -- {w}")]
        }
        // `=` in a scalar-assignment RHS: c:3901-3920 `force_split = !ssub &&
        // spbreak`, so ssub suppresses the split and the joined value is
        // assigned — `v=$=s` must NOT split.
        _ => {
            let w = pick(&mut rng, RCEXPAND_SPLIT_WORDS);
            let v = pick(&mut rng, RCEXPAND_SCALARS);
            vec![format!(
                "s={v}; r=({w}); print -r -- \"n=${{#r}}\"; v={w}; print -r -- \"[$v]\""
            )]
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
    Unicode,
    Quote,
    Datetime,
    Paramod,
    Procsub,
    Alias,
    Autoload,
    Stat,
    Errexit,
    Posparam,
    Numfmt,
    Mapfile,
    Pcre,
    Zwc,
    Tied,
    Readb,
    Fd,
    Special,
    Brace,
    Getopts,
    Assoc,
    Casesel,
    Default,
    Anonfn,
    Printv,
    Globanchor,
    Whence,
    Zstyle,
    Atflag,
    Subexp,
    Replace,
    Assign,
    Gflag,
    Select,
    Bindkey,
    Zmv,
    Zcalc,
    Rcexpand,
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
        Mode::Unicode => gen_unicode(seed),
        Mode::Quote => gen_quote(seed),
        Mode::Datetime => gen_datetime(seed),
        Mode::Paramod => gen_paramod(seed),
        Mode::Procsub => gen_procsub(seed),
        Mode::Alias => gen_alias(seed),
        Mode::Autoload => gen_autoload(seed),
        Mode::Stat => gen_stat(seed),
        Mode::Errexit => gen_errexit(seed),
        Mode::Posparam => gen_posparam(seed),
        Mode::Numfmt => gen_numfmt(seed),
        Mode::Mapfile => gen_mapfile(seed),
        Mode::Pcre => gen_pcre(seed),
        Mode::Zwc => gen_zwc(seed),
        Mode::Tied => gen_tied(seed),
        Mode::Readb => gen_readb(seed),
        Mode::Fd => gen_fd(seed),
        Mode::Special => gen_special(seed),
        Mode::Brace => gen_brace_mode(seed),
        Mode::Getopts => gen_getopts(seed),
        Mode::Assoc => gen_assoc(seed),
        Mode::Casesel => gen_casesel(seed),
        Mode::Default => gen_default(seed),
        Mode::Anonfn => gen_anonfn(seed),
        Mode::Printv => gen_printv(seed),
        Mode::Globanchor => gen_globanchor(seed),
        Mode::Whence => gen_whence(seed),
        Mode::Zstyle => gen_zstyle(seed),
        Mode::Atflag => gen_atflag(seed),
        Mode::Subexp => gen_subexp(seed),
        Mode::Replace => gen_replace(seed),
        Mode::Assign => gen_assign(seed),
        Mode::Gflag => gen_gflag(seed),
        Mode::Select => gen_select(seed),
        Mode::Bindkey => gen_bindkey(seed),
        Mode::Zmv => gen_zmv(seed),
        Mode::Zcalc => gen_zcalc(seed),
        Mode::Rcexpand => gen_rcexpand(seed),
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
        Mode::Unicode => "unicode",
        Mode::Quote => "quote",
        Mode::Datetime => "datetime",
        Mode::Paramod => "paramod",
        Mode::Procsub => "procsub",
        Mode::Alias => "alias",
        Mode::Autoload => "autoload",
        Mode::Stat => "stat",
        Mode::Errexit => "errexit",
        Mode::Posparam => "posparam",
        Mode::Numfmt => "numfmt",
        Mode::Mapfile => "mapfile",
        Mode::Pcre => "pcre",
        Mode::Zwc => "zwc",
        Mode::Tied => "tied",
        Mode::Readb => "readb",
        Mode::Fd => "fd",
        Mode::Special => "special",
        Mode::Brace => "brace",
        Mode::Getopts => "getopts",
        Mode::Assoc => "assoc",
        Mode::Casesel => "casesel",
        Mode::Default => "default",
        Mode::Anonfn => "anonfn",
        Mode::Printv => "printv",
        Mode::Globanchor => "globanchor",
        Mode::Whence => "whence",
        Mode::Zstyle => "zstyle",
        Mode::Atflag => "atflag",
        Mode::Subexp => "subexp",
        Mode::Replace => "replace",
        Mode::Assign => "assign",
        Mode::Gflag => "gflag",
        Mode::Select => "select",
        Mode::Bindkey => "bindkey",
        Mode::Zmv => "zmv",
        Mode::Zcalc => "zcalc",
        Mode::Rcexpand => "rcexpand",
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
        Mode::Unicode,
        Mode::Quote,
        Mode::Datetime,
        Mode::Paramod,
        Mode::Procsub,
        Mode::Alias,
        Mode::Autoload,
        Mode::Stat,
        Mode::Errexit,
        Mode::Posparam,
        Mode::Numfmt,
        Mode::Mapfile,
        Mode::Pcre,
        Mode::Zwc,
        Mode::Tied,
        Mode::Readb,
        Mode::Fd,
        Mode::Special,
        Mode::Brace,
        Mode::Getopts,
        Mode::Assoc,
        Mode::Casesel,
        Mode::Default,
        Mode::Anonfn,
        Mode::Printv,
        Mode::Globanchor,
        Mode::Whence,
        Mode::Zstyle,
        Mode::Atflag,
        Mode::Subexp,
        Mode::Replace,
        Mode::Assign,
        Mode::Gflag,
        Mode::Select,
        Mode::Bindkey,
        Mode::Zmv,
        Mode::Zcalc,
        Mode::Rcexpand,
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
                     emulate, dirstack, unicode, quote, datetime,\n\
                     paramod, procsub, alias, autoload, stat, errexit,\n\
                     posparam, numfmt, mapfile, pcre, zwc, tied,\n\
                     readb, fd, special, brace, getopts, assoc,\n\
                     casesel, default, anonfn, printv, globanchor,\n\
                     whence, zstyle, atflag, subexp, replace, assign,\n\
                     gflag, select, bindkey, zmv, zcalc, rcexpand\n\
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
                     env  ZSHRS_FUZZ_ZSH=PATH  the zsh to compare against. The\n\
                                      oracle is part of the result: a baseline is\n\
                                      only valid against the zsh that produced it,\n\
                                      and Homebrew 5.9.2, apt's zsh and the C-spec\n\
                                      fork are three different codebases. Set this\n\
                                      to a zsh built from the fork to make the\n\
                                      oracle and the port's spec the same code.\n\
                                      Every run prints the oracle it used.\n\
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

    // Modes that read the filesystem run from a fixture directory. glob and
    // stat share one (fixed sizes + staggered mtimes make both the ordering
    // qualifiers and the stat fields deterministic); autoload gets its own
    // `fns/` tree on $fpath. Every fixture is READ-ONLY at run time, which is
    // what lets the parallel workers share a single cwd.
    match args.mode {
        Mode::Glob | Mode::Stat => {
            FIXTURE_CWD.set(setup_glob_fixture()).ok();
        }
        Mode::Autoload => {
            FIXTURE_CWD.set(setup_autoload_fixture()).ok();
        }
        Mode::Zmv => {
            FIXTURE_CWD.set(setup_zmv_fixture()).ok();
        }
        // Modes whose probes (or whose MINIMIZED probes) can create files must
        // never run from the source tree — see setup_scratch_fixture.
        Mode::Alias | Mode::Procsub | Mode::Errexit | Mode::Mapfile | Mode::Fd => {
            FIXTURE_CWD.set(setup_scratch_fixture()).ok();
        }
        _ => {}
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
        let _ = std::io::stdout().write_all(&z.stdout);
        println!("--- zshrs exit={} timeout={} ---", r.exit, r.timed_out);
        let _ = std::io::stdout().write_all(&r.stdout);
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
                                format!("\n  stderr: {}", render(&norm_stderr(&o.stderr)).replace('\n', "\n  "))
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
                            render(&mz.stdout),
                            mr.exit,
                            mr.timed_out,
                            err_of(&mr),
                            render(&mr.stdout),
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

    let oracle = zsh_oracle_id();
    println!(
        "\nfuzzed {checked} cases in {:.1}s ({:.0}/s)\n\
         oracle      : {}\n\
         divergences : {} ({} known / {} new)\n\
         timeouts    : {}",
        elapsed.as_secs_f64(),
        checked as f64 / elapsed.as_secs_f64().max(0.001),
        oracle,
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
            // Attribute the records: "zshrs disagrees with zsh" is only a claim
            // about the zsh that ran, and three different ones are in play.
            let _ = writeln!(f, "# oracle: {oracle}");
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
