//! ztst_runner — runs zsh's .ztst integration test files against zshrs
//!
//! Parses the %prep / %test / %clean sections from each .ztst file and
//! drives ONE persistent `zshrs --zsh -f` child per file, mirroring zsh's
//! own harness Test/ztst.zsh: every chunk is eval'd in the SAME shell
//! process (ztst.zsh:294-307 ZTST_execchunk), so variables, functions,
//! options and aliases set by earlier chunks carry forward to later ones
//! exactly as in the real harness. Exit status + stdout + stderr are
//! captured to files (ztst.zsh:484) and compared per test block.
//!
//! Transport: zshrs does not yet execute a stdin script incrementally
//! (it buffers to EOF — verified by probe), so chunks are delivered by
//! sourcing a FIFO in a loop from a driver script; a marker line printed
//! to the harness fd (ztst.zsh:198 `exec {ZTST_fd}>&1`) signals chunk
//! completion and carries `$ZTST_status` (ztst.zsh:301).
//!
//! Run:  cargo test -p zsh --test ztst_runner -- [filter]
//!       ZTST_VERBOSE=1 cargo test -p zsh --test ztst_runner -- --nocapture
//!
//! Env vars:
//!   ZTST_TIMEOUT_MS=N — per-chunk timeout in milliseconds (default: 2000)
//!   ZTST_VERBOSE=1  — print pass/skip results, not just failures
//!   ZTST_ZSH_SOURCE=/path/to/zsh — zsh source tree used to build $fpath
//!     the way ztst.zsh:112-114 does (default: $HOME/forkedRepos/zsh)

use std::env;
use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader, Read as _, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Parsed representations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TestBlock {
    /// Human-readable description after the status code
    message: String,
    /// The indented code to eval
    code: String,
    /// Expected exit status (None means "don't check")
    expected_status: Option<i32>,
    /// Flags: d = ignore stdout, D = ignore stderr, f = expected-fail, q = delayed subst
    flags: String,
    /// Expected stdout lines (joined with \n)
    expected_stdout: String,
    /// Expected stderr lines (joined with \n)
    expected_stderr: String,
    /// Stdin to feed the command
    stdin_data: String,
    /// Use pattern matching for stdout
    stdout_pattern: bool,
    /// Use pattern matching for stderr
    stderr_pattern: bool,
}

#[derive(Debug)]
#[allow(dead_code)]
struct ZtstFile {
    name: String,
    prep: Vec<String>,
    tests: Vec<TestBlock>,
    clean: Vec<String>,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

fn parse_ztst(path: &Path) -> ZtstFile {
    let raw = fs::read(path).unwrap_or_else(|e| {
        panic!("failed to read {}: {}", path.display(), e);
    });
    let content = String::from_utf8_lossy(&raw).into_owned();
    let name = path.file_name().unwrap().to_string_lossy().into_owned();

    let lines: Vec<&str> = content.lines().collect();
    let mut idx = 0;
    let mut prep: Vec<String> = Vec::new();
    let mut tests: Vec<TestBlock> = Vec::new();
    let mut clean: Vec<String> = Vec::new();
    let mut current_section = "";

    while idx < lines.len() {
        let line = lines[idx];

        // Skip comments at top level
        if line.starts_with('#') {
            idx += 1;
            continue;
        }

        // Section headers
        if line.starts_with('%') {
            let sect = line.trim_start_matches('%').trim();
            current_section = if sect.starts_with("prep") {
                "prep"
            } else if sect.starts_with("test") {
                "test"
            } else if sect.starts_with("clean") {
                "clean"
            } else {
                idx += 1;
                continue;
            };
            idx += 1;
            continue;
        }

        // Skip blank lines between chunks
        if line.trim().is_empty() {
            idx += 1;
            continue;
        }

        match current_section {
            "prep" => {
                let before = idx;
                if let Some(chunk) = read_code_chunk(&lines, &mut idx) {
                    prep.push(chunk);
                } else if idx == before {
                    // read_code_chunk didn't advance — skip this line
                    idx += 1;
                }
            }
            "clean" => {
                let before = idx;
                if let Some(chunk) = read_code_chunk(&lines, &mut idx) {
                    clean.push(chunk);
                } else if idx == before {
                    idx += 1;
                }
            }
            "test" => {
                let before = idx;
                if let Some(test) = read_test_block(&lines, &mut idx) {
                    tests.push(test);
                } else if idx == before {
                    idx += 1;
                }
            }
            _ => {
                idx += 1;
            }
        }
    }

    if env::var("ZTST_VERBOSE").map(|v| v != "0").unwrap_or(false) {
        eprintln!(
            "  parsed {}: {} prep chunks, {} tests, {} clean chunks",
            name,
            prep.len(),
            tests.len(),
            clean.len()
        );
    }

    ZtstFile {
        name,
        prep,
        tests,
        clean,
    }
}

/// Read an indented code chunk (lines starting with whitespace).
/// Returns None if current line isn't indented.
///
/// Format contract verified against zsh's own harness:
///   - ztst.zsh:208-214 `ZTST_getline` drops any line starting with
///     `#` at column 0 — comments may appear ANYWHERE, including
///     between the lines of a code chunk (A04redirect.ztst:46 has
///     one inside a here-document chunk).
///   - ztst.zsh:249-251 `ZTST_getchunk` accepts lines matching
///     `[[:blank:]]##[^[:blank:]]*` and stores `$ZTST_curline`
///     VERBATIM — the indentation is NOT stripped before eval.
///     Stripping it corrupts here-document bodies/terminators
///     (`cat <<'  HERE'` expects body lines starting with two
///     spaces; `<<-HERE` tests need their literal tabs).
fn read_code_chunk(lines: &[&str], idx: &mut usize) -> Option<String> {
    // Skip blank lines and column-0 comment lines (ztst.zsh:208-214)
    while *idx < lines.len() && (lines[*idx].trim().is_empty() || lines[*idx].starts_with('#')) {
        *idx += 1;
    }
    if *idx >= lines.len() {
        return None;
    }
    // Must start with whitespace
    let first = lines[*idx];
    if !first.starts_with(' ') && !first.starts_with('\t') {
        return None;
    }

    let mut chunk = String::new();
    while *idx < lines.len() {
        let line = lines[*idx];
        if line.starts_with('#') {
            // ztst.zsh:208-214 — comment lines are invisible to the
            // chunk reader; they do NOT terminate the chunk.
            *idx += 1;
        } else if line.starts_with(' ') || line.starts_with('\t') {
            if !chunk.is_empty() {
                chunk.push('\n');
            }
            // Verbatim — ztst.zsh:249-251 keeps the line as-is.
            chunk.push_str(line);
            *idx += 1;
        } else {
            // Blank line or unindented content — chunk ends
            // (ztst.zsh:249 loop condition fails on both).
            break;
        }
    }

    if chunk.is_empty() {
        None
    } else {
        Some(chunk)
    }
}

/// Parse a complete test block: code chunk + status line + redirections
fn read_test_block(lines: &[&str], idx: &mut usize) -> Option<TestBlock> {
    // Skip blank and comment lines
    while *idx < lines.len() {
        let line = lines[*idx];
        if line.starts_with('#') || line.trim().is_empty() {
            *idx += 1;
            continue;
        }
        if line.starts_with('%') {
            return None;
        }
        break;
    }

    // Read code chunk
    let code = read_code_chunk(lines, idx)?;

    // Now expect a status line: NUMBER[FLAGS]:message
    // Skip comments between code and status
    while *idx < lines.len() && lines[*idx].starts_with('#') {
        *idx += 1;
    }

    if *idx >= lines.len() {
        return None;
    }

    let status_line = lines[*idx];
    let (expected_status, flags, message) = parse_status_line(status_line)?;
    *idx += 1;

    let mut expected_stdout = String::new();
    let mut expected_stderr = String::new();
    let mut stdin_data = String::new();
    let mut stdout_pattern = false;
    let mut stderr_pattern = false;

    // Read redirections: > for stdout, ? for stderr, < for stdin
    // Also *> for pattern stdout, *? for pattern stderr
    // Also F: for failure messages (ignored by runner)
    while *idx < lines.len() {
        let line = lines[*idx];
        if let Some(rest) = line.strip_prefix("*>") {
            stdout_pattern = true;
            append_redir_line(&mut expected_stdout, rest);
            *idx += 1;
            // Continue reading > lines as part of same stdout block.
            // A `#` comment between them is invisible to ztst.zsh (see the
            // outer loop's comment arm), so skip it and keep reading.
            while *idx < lines.len() {
                if lines[*idx].starts_with('#') {
                    *idx += 1;
                    continue;
                }
                let Some(rest) = lines[*idx].strip_prefix('>') else {
                    break;
                };
                append_redir_line(&mut expected_stdout, rest);
                *idx += 1;
            }
        } else if let Some(rest) = line.strip_prefix('>') {
            append_redir_line(&mut expected_stdout, rest);
            *idx += 1;
        } else if let Some(rest) = line.strip_prefix("*?") {
            stderr_pattern = true;
            append_redir_line(&mut expected_stderr, rest);
            *idx += 1;
            while *idx < lines.len() {
                if lines[*idx].starts_with('#') {
                    *idx += 1;
                    continue;
                }
                let Some(rest) = lines[*idx].strip_prefix('?') else {
                    break;
                };
                append_redir_line(&mut expected_stderr, rest);
                *idx += 1;
            }
        } else if let Some(rest) = line.strip_prefix('?') {
            append_redir_line(&mut expected_stderr, rest);
            *idx += 1;
        } else if let Some(rest) = line.strip_prefix('<') {
            append_redir_line(&mut stdin_data, rest);
            *idx += 1;
        } else if line.starts_with("F:") {
            // Failure hint — skip
            *idx += 1;
        } else if line.starts_with('#') {
            // ztst.zsh:211-216 — ZTST_getline skips comment lines at the
            // READ level (`[[ $ZTST_curline == \#* ]] || return 0`), so a
            // comment is invisible to the block parser no matter where it
            // sits. This loop had no comment arm and broke out instead, so
            // a `#` between the status line and its `>`/`?` lines truncated
            // the expected output and the chunk failed with a mismatch even
            // though the shell's output was correct (C01arith.ztst:57 and
            // :74, Z02zmathfunc.ztst:6 — all three verified byte-identical
            // to `zsh -f`).
            *idx += 1;
        } else {
            break;
        }
    }

    Some(TestBlock {
        message,
        code,
        expected_status,
        flags,
        expected_stdout,
        expected_stderr,
        stdin_data,
        stdout_pattern,
        stderr_pattern,
    })
}

fn append_redir_line(buf: &mut String, content: &str) {
    // ztst.zsh's ZTST_getredir writes each `>`/`<` line with `print -r --`,
    // i.e. content PLUS a newline, unconditionally. The previous
    // `if !buf.is_empty()` separator logic silently dropped a LEADING empty
    // line: a block starting with a bare `>` left buf == "" so the next
    // line got no separator, and the expected side lost its opening blank
    // line while the shell correctly emitted one (V09datetime.ztst:1).
    buf.push_str(content);
    buf.push('\n');
}

/// Parse "NUMBER[FLAGS]:message" — returns (status, flags, message)
fn parse_status_line(line: &str) -> Option<(Option<i32>, String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with('%') {
        return None;
    }

    // Format: [-]NUMBER[FLAGS]:message
    // Or: -:message (dash means don't check status)
    let mut chars = line.chars().peekable();
    let mut num_str = String::new();
    let mut flags = String::new();

    // Read number (may be negative or just '-')
    if chars.peek() == Some(&'-') {
        num_str.push('-');
        chars.next();
    }
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            num_str.push(c);
            chars.next();
        } else {
            break;
        }
    }

    // Read flags (alphabetic chars before ':')
    while let Some(&c) = chars.peek() {
        if c == ':' {
            chars.next();
            break;
        } else if c.is_ascii_alphabetic() {
            flags.push(c);
            chars.next();
        } else {
            return None;
        }
    }

    // Rest is message
    let message = chars.collect::<String>().trim().to_string();

    let status = if num_str == "-" {
        None
    } else {
        num_str.parse::<i32>().ok()
    };

    Some((status, flags, message))
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

fn find_zshrs() -> PathBuf {
    // Check env override first
    if let Ok(p) = env::var("ZSHRS") {
        let path = PathBuf::from(p);
        if path.exists() {
            return path;
        }
    }

    // Try relative to workspace
    let candidates = [
        "target/debug/zshrs",
        "target/release/zshrs",
        "../target/debug/zshrs",
        "../target/release/zshrs",
        "../../target/debug/zshrs",
        "../../target/release/zshrs",
    ];
    for c in &candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return p.canonicalize().unwrap();
        }
    }

    panic!(
        "zshrs binary not found. Build it first with `cargo build -p zsh` \
         or set ZSHRS=/path/to/zshrs"
    );
}

fn find_test_corpus() -> PathBuf {
    let candidates = ["zsh/test_corpus", "test_corpus", "../test_corpus"];
    for c in &candidates {
        let p = PathBuf::from(c);
        if p.is_dir() {
            return p.canonicalize().unwrap();
        }
    }
    panic!("test_corpus directory not found");
}

struct TestResult {
    message: String,
    passed: bool,
    skipped: bool,
    detail: String,
}

impl fmt::Display for TestResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.skipped {
            write!(f, "SKIP: {}", self.message)
        } else if self.passed {
            write!(f, "PASS: {}", self.message)
        } else {
            write!(f, "FAIL: {}\n      {}", self.message, self.detail)
        }
    }
}

/// Pattern comparison, delegated to real zsh — the faithful port of
/// `ZTST_diff`'s `diff_pat` arm (Test/ztst.zsh:380-398), which runs under
/// `emulate -L zsh; setopt extendedglob` (ztst.zsh:330-331) and compares
/// line-by-line with `[[ ${diff_lines2[i]} != ${~diff_lines1[i]} ]]`.
///
/// This USED to be a hand-rolled Rust matcher supporting only `*`, `?` and
/// `\`. The corpus routinely uses `##`, `(#cN,M)`, `<a-b>` and `[...]`
/// classes, so every expectation using one was compared literally and
/// reported a FALSE FAILURE even when the shell's output was correct
/// (A08time.ztst chunks 2-7 were six such cases). Reimplementing
/// extendedglob here would forever lag the real thing, and using *zshrs*
/// to match would make the shell under test its own oracle. So shell out
/// to the same zsh that `assert_parity` uses.
fn matchpat(pattern: &str, text: &str) -> bool {
    match matchpat_zsh(pattern, text) {
        Some(v) => v,
        None => {
            eprintln!(
                "ztst_runner: WARNING — could not run `zsh` for pattern matching; \
                 falling back to the legacy `*`/`?`-only matcher. Expectations using \
                 `##`, `(#c..)`, `<a-b>` or `[...]` WILL REPORT FALSE FAILURES."
            );
            matchpat_legacy(pattern, text)
        }
    }
}

/// Run ztst.zsh's own comparison loop in a real zsh. Returns None if zsh
/// could not be run at all (so the caller can warn and degrade loudly).
fn matchpat_zsh(pattern: &str, text: &str) -> Option<bool> {
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SEQ: AtomicUsize = AtomicUsize::new(0);

    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = env::temp_dir();
    let pf = dir.join(format!("zshrs_ztst_pat_{}_{}.txt", std::process::id(), n));
    let tf = dir.join(format!("zshrs_ztst_txt_{}_{}.txt", std::process::id(), n));

    // ztst.zsh reads these with `$(<file)`, which strips trailing newlines;
    // write the bodies with a single trailing newline like the harness files.
    let write = |p: &Path, body: &str| -> std::io::Result<()> {
        let mut f = fs::File::create(p)?;
        f.write_all(body.as_bytes())?;
        if !body.ends_with('\n') {
            f.write_all(b"\n")?;
        }
        Ok(())
    };
    if write(&pf, pattern).is_err() || write(&tf, text).is_err() {
        let _ = fs::remove_file(&pf);
        let _ = fs::remove_file(&tf);
        return None;
    }

    // Verbatim port of ztst.zsh:381-397. `$1` is the pattern file
    // (diff_lines1), `$2` the actual-output file (diff_lines2).
    const SCRIPT: &str = r#"
emulate -L zsh
setopt extendedglob
local -a diff_lines1 diff_lines2
integer i
diff_lines1=("${(f@)$(<$1)}")
diff_lines2=("${(f@)$(<$2)}")
if (( ${#diff_lines1} != ${#diff_lines2} )); then
  exit 1
fi
for (( i = 1; i <= ${#diff_lines1}; i++ )); do
  if [[ ${diff_lines2[i]} != ${~diff_lines1[i]} ]]; then
    exit 1
  fi
done
exit 0
"#;

    let out = Command::new("zsh")
        .arg("-f")
        .arg("-c")
        .arg(SCRIPT)
        .arg("ztst_matchpat")
        .arg(&pf)
        .arg(&tf)
        .output();
    let _ = fs::remove_file(&pf);
    let _ = fs::remove_file(&tf);

    match out {
        Ok(o) => o.status.code().map(|c| c == 0),
        Err(_) => None,
    }
}

/// The former matcher, kept ONLY as the degraded fallback when zsh is
/// absent. Supports `*`, `?` and `\` escapes and nothing else.
fn matchpat_legacy(pattern: &str, text: &str) -> bool {
    let pat_lines: Vec<&str> = pattern.lines().collect();
    let txt_lines: Vec<&str> = text.lines().collect();

    if pat_lines.len() != txt_lines.len() {
        return false;
    }

    for (p, t) in pat_lines.iter().zip(txt_lines.iter()) {
        if !glob_line_match(p, t) {
            return false;
        }
    }
    true
}

fn glob_line_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_match_inner(&pat, 0, &txt, 0)
}

fn glob_match_inner(pat: &[char], pi: usize, txt: &[char], ti: usize) -> bool {
    if pi == pat.len() && ti == txt.len() {
        return true;
    }
    if pi == pat.len() {
        return false;
    }
    if pat[pi] == '\\' && pi + 1 < pat.len() {
        return ti < txt.len()
            && pat[pi + 1] == txt[ti]
            && glob_match_inner(pat, pi + 2, txt, ti + 1);
    }
    if pat[pi] == '*' {
        let mut npi = pi;
        while npi < pat.len() && pat[npi] == '*' {
            npi += 1;
        }
        for nti in ti..=txt.len() {
            if glob_match_inner(pat, npi, txt, nti) {
                return true;
            }
        }
        false
    } else if ti < txt.len() && (pat[pi] == '?' || pat[pi] == txt[ti]) {
        glob_match_inner(pat, pi + 1, txt, ti + 1)
    } else {
        false
    }
}

/// Single-quote a literal for embedding in shell source.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn timeout_ms() -> u64 {
    // Default must clear the corpus's deliberate delays — C02cond's `-N`
    // test does `sleep 2` (ztst:151), V09datetime sleeps, etc. At the
    // old 2000ms those legit chunks hit the per-chunk timeout, which
    // WEDGES the file's shell and makes every later chunk "not run" — a
    // single slow chunk masqueraded as ~30 failures. 10s leaves margin
    // while still catching a genuine hang. Override with ZTST_TIMEOUT_MS.
    env::var("ZTST_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10000)
}

/// Compute the $fpath the real harness builds at ztst.zsh:112-114:
///   fpath=( $ZTST_srcdir/../Functions/*~*/CVS(/)
///           $ZTST_srcdir/../Completion
///           $ZTST_srcdir/../Completion/*/*~*/CVS(/) )
/// In a zsh build tree those globs resolve against the source checkout;
/// our corpus dir has no sibling Functions/Completion, so resolve them
/// against the zsh source tree (ZTST_ZSH_SOURCE, default
/// $HOME/forkedRepos/zsh). If the tree is absent the list is empty —
/// same observable result as the globs matching nothing.
fn ztst_zsh_source() -> Option<PathBuf> {
    env::var("ZTST_ZSH_SOURCE")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join("forkedRepos/zsh"))
        })
        .filter(|p| p.is_dir())
}

fn compute_fpath() -> Vec<PathBuf> {
    let Some(root) = ztst_zsh_source() else {
        return Vec::new();
    };

    let subdirs = |dir: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_dir() && p.file_name().map(|n| n != "CVS").unwrap_or(false))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };

    let mut fpath = subdirs(&root.join("Functions"));
    let completion = root.join("Completion");
    if completion.is_dir() {
        fpath.push(completion.clone());
        for group in subdirs(&completion) {
            fpath.extend(subdirs(&group));
        }
    }
    fpath
}

/// Outcome of one chunk payload sent to the persistent shell.
enum ChunkOutcome {
    /// Marker came back: `$ZTST_status` plus the auxiliary field
    /// (ZTST_skip message for tests, ZTST_unimplemented for prep).
    Done { status: i32, aux: String },
    /// No marker within the per-chunk timeout — shell killed.
    Timeout,
    /// Shell exited (EOF on its stdout) before the marker arrived.
    Died,
}

/// One persistent `zshrs --zsh -f` process per .ztst file — the
/// execution model of zsh's own harness, where a single shell sources
/// every chunk so cross-chunk state carries forward (ztst.zsh:294-307).
///
/// Sandbox layout (one fresh tree per file):
///   <root>/Test     — cwd; ZTST_testdir=$PWD per ztst.zsh:70. Chunks
///                     invoke the shell-under-test as
///                     `$ZTST_testdir/../Src/zsh` (A01grammar,
///                     A04redirect.ztst:465-525, E03posix), so:
///   <root>/Src/zsh  — symlink to the zshrs binary
///   <root>/srcdir   — ZTST_srcdir: symlinks to every corpus file, with
///                     <root>/Functions and <root>/Completion siblings
///                     pointing into ZTST_ZSH_SOURCE (Test/comptest:3-5
///                     rebuilds $fpath from $ZTST_srcdir/../*)
///   <root>/home     — $HOME (keeps `~/x` writes out of the real home)
///   <root>/tmp      — $TMPDIR; holds the ztst.zsh:117-129 capture files
///   <root>/cmd.fifo — chunk transport (sourced in a loop by driver.zsh)
struct PersistentShell {
    child: Child,
    pgid: i32,
    lines_rx: mpsc::Receiver<String>,
    fifo: PathBuf,
    sandbox: PathBuf,
    seq: usize,
    nonce: u128,
    dead: bool,
    ztst_in: PathBuf,
    ztst_tout: PathBuf,
    ztst_terr: PathBuf,
    qout: PathBuf,
    qerr: PathBuf,
}

impl PersistentShell {
    fn spawn(zshrs: &Path, file_stem: &str, corpus: &Path) -> std::io::Result<PersistentShell> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let sandbox = env::temp_dir().join(format!(
            "zshrs_ztst_{}_{}_{}",
            std::process::id(),
            file_stem,
            nonce
        ));
        let testdir = sandbox.join("Test");
        let srcdir = sandbox.join("Src");
        let home = sandbox.join("home");
        let tmp = sandbox.join("tmp");
        for d in [&testdir, &srcdir, &home, &tmp] {
            fs::create_dir_all(d)?;
        }
        std::os::unix::fs::symlink(zshrs, srcdir.join("zsh"))?;

        // ZTST_srcdir stand-in. ztst.zsh:104-109 points $ZTST_srcdir at the
        // zsh source tree's Test/ directory, and the .ztst files rely on
        // BOTH of its properties: the helper files that live in it
        // (`. $ZTST_srcdir/comptest` — Y01/Y02/Y03, X02/X03/X05 — and
        // `$ZTST_srcdir/B02typeset.ztst` — V10private) AND its SIBLINGS
        // (Test/comptest:3-5 rebuilds $fpath from
        // `$ZTST_srcdir/../Functions/*~*/CVS(/)` and
        // `$ZTST_srcdir/../Completion`).
        //
        // Pointing $ZTST_srcdir straight at the corpus directory gave the
        // first property and not the second: `<repo>/Functions` and
        // `<repo>/Completion` do not exist, so comptest's array assignment
        // died with "no matches found" and `comptestinit` returned 1 —
        // which is why every comptest-driven file reported a failed %prep
        // and zero passing chunks.
        //
        // So build a per-file stand-in that has both: a directory of
        // symlinks to the corpus files, with Functions/Completion siblings
        // pointing into the zsh source tree already used for $fpath
        // (ZTST_ZSH_SOURCE). Symlinks, not copies, so a corpus edit is
        // picked up and nothing is duplicated on disk. It is a sibling of
        // Test/ rather than Test/ itself so the cwd every chunk globs stays
        // empty.
        let srcdir_stub = sandbox.join("srcdir");
        fs::create_dir_all(&srcdir_stub)?;
        if let Ok(rd) = fs::read_dir(corpus) {
            for entry in rd.filter_map(|e| e.ok()) {
                let _ =
                    std::os::unix::fs::symlink(entry.path(), srcdir_stub.join(entry.file_name()));
            }
        }
        if let Some(zsh_src) = ztst_zsh_source() {
            for name in ["Functions", "Completion", "Misc"] {
                let real = zsh_src.join(name);
                if real.is_dir() {
                    let _ = std::os::unix::fs::symlink(&real, sandbox.join(name));
                }
            }
        }

        let fifo = sandbox.join("cmd.fifo");
        let cpath = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
        if unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) } != 0 {
            return Err(std::io::Error::last_os_error());
        }

        // ZTST_tmp is fixed instead of ${TMPPREFIX}.ztst.$$ (ztst.zsh:117)
        // because the runner must write $ZTST_in before each chunk and
        // can't know the child's $$; per-file sandboxing supplies the
        // uniqueness that $$ provides in the real harness.
        let ztmp = tmp.join("zsh.ztst");
        fs::create_dir_all(&ztmp)?;
        let ztst_in = ztmp.join("ztst.in");
        let ztst_tout = ztmp.join("ztst.tout");
        let ztst_terr = ztmp.join("ztst.terr");
        let qout = ztmp.join("ztst.qout");
        let qerr = ztmp.join("ztst.qerr");

        let fpath = compute_fpath()
            .iter()
            .map(|p| sq(&p.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(" ");

        // Driver = the ztst.zsh environment, minus the section-reading
        // loop (ztst.zsh:397-561), which lives in the Rust parser, plus
        // a FIFO source loop as the chunk transport.
        let driver = format!(
            r#"# Per-file persistent harness shell — mirrors Test/ztst.zsh.
emulate -R zsh                          # ztst.zsh:26
# ztst.zsh:33-43 verbatim
ZTST_find_UTF8 () {{
  setopt multibyte
  local langs=(en_{{US,GB}}.{{UTF-,utf}}8 en.UTF-8
               ${{(M)$(locale -a 2>/dev/null):#*.(utf8|UTF-8)}})
  for LANG in $langs; do
    if [[ é = ? ]]; then
      echo $LANG
      return
    fi
  done
}}
typeset +x WORDCHARS                    # ztst.zsh:46
[[ -d Modules/zsh ]] && module_path=( $PWD/Modules )  # ztst.zsh:50
ZTST_testdir=$PWD                       # ztst.zsh:70
# ztst.zsh:80-100 verbatim
tail() {{
  emulate -L zsh

  if [[ -z $TAIL_SUPPORTS_MINUS_N ]]; then
    local test
    test=$(echo "foo\nbar" | command tail -n 1 2>/dev/null)
    if [[ $test = bar ]]; then
      TAIL_SUPPORTS_MINUS_N=1
    else
      TAIL_SUPPORTS_MINUS_N=0
    fi
  fi

  integer argi=${{argv[(i)-<->]}}

  if [[ $argi -le $# && $TAIL_SUPPORTS_MINUS_N = 1 ]]; then
    argv[$argi]=(-n ${{argv[$argi][2,-1]}})
  fi

  command tail "$argv[@]"
}}
ZTST_srcdir={srcdir}                    # ztst.zsh:104-109 ($0's directory)
# ZTST_exe is set by Test/Makefile.in:56 (ZTST_exe=$(dir_top)/Src/zsh), NOT by
# ztst.zsh, so the runner must supply it or every chunk spawning a fresh shell
# dies with `command not found: -fc` (C03traps.ztst:6,29,30,31,32). An ABSOLUTE
# path is used because the corpus invokes it in incompatible ways: bare from
# $ZTST_testdir, from `(cd ..; $ZTST_exe …)` (C03traps.ztst:460,477,497,514),
# and via `${{${{ZTST_exe##[^/]*}}:-$ZTST_testdir/$ZTST_exe}}` (C03traps.ztst:245).
# Only an absolute path satisfies all three; the Makefile's relative spelling is
# the wart C03traps.ztst:60-61 calls out ("We ought to fix this in ztst.zsh...").
ZTST_exe={ztst_exe}
fpath=( {fpath} )                       # ztst.zsh:112-114 (resolved by runner)
ZTST_tmp={ztmp}                         # ztst.zsh:116-117 (fixed path; see runner)
ZTST_in=${{ZTST_tmp}}/ztst.in           # ztst.zsh:123
ZTST_out=${{ZTST_tmp}}/ztst.out         # ztst.zsh:125
ZTST_err=${{ZTST_tmp}}/ztst.err         # ztst.zsh:126
ZTST_tout=${{ZTST_tmp}}/ztst.tout       # ztst.zsh:128
ZTST_terr=${{ZTST_tmp}}/ztst.terr       # ztst.zsh:129
setopt extendedglob nonomatch           # ztst.zsh:61 (mainopts; preamble only)
rm -rf dummy.tmp *.tmp                  # ztst.zsh:142
exec {{ZTST_fd}}>&1                     # ztst.zsh:198
# ztst.zsh:294-307. The $options save/restore (ZTST_testopts /
# ZTST_mainopts, ztst.zsh:296,303-304) is omitted: zshrs rejects
# assignment to the special `options` array ("can't change type of a
# special parameter" — see fall-over notes). Observable behavior is
# preserved: in ztst.zsh, options a chunk sets are captured into
# ZTST_testopts and re-applied before the next chunk, i.e. they
# persist chunk-to-chunk — which is exactly what happens here with no
# restore at all, since no harness shell code parses between chunks
# (the section reader lives in the Rust runner).
ZTST_execchunk() {{
  setopt localloops # don't let continue & break propagate out
  () {{
      unsetopt localloops
      eval "$ZTST_code"
  }}
  ZTST_status=$?
  return $ZTST_status
}}
# Chunks run under the option state captured at ztst.zsh:59 — the
# `emulate -R zsh` defaults from ztst.zsh:26, BEFORE the ztst.zsh:61
# mainopts setopt. Re-enter that state for the first chunk (later
# chunks inherit whatever earlier chunks set, as via ZTST_testopts).
emulate -R zsh
while true; do
  . {fifo}
done
"#,
            srcdir = sq(&srcdir_stub.to_string_lossy()),
            ztst_exe = sq(&srcdir.join("zsh").to_string_lossy()),
            fpath = fpath,
            ztmp = sq(&ztmp.to_string_lossy()),
            fifo = sq(&fifo.to_string_lossy()),
        );
        let driver_path = sandbox.join("driver.zsh");
        fs::write(&driver_path, driver)?;

        let mut cmd = Command::new(zshrs);
        cmd.arg("--zsh")
            .arg("-f")
            .arg(&driver_path)
            .current_dir(&testdir)
            // ztst.zsh:29-30 — unset LC_*, export LANG=C only.
            .env("LANG", "C")
            .env("HOME", &home)
            .env("TMPDIR", &tmp)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, _) in env::vars() {
            if k.starts_with("LC_") {
                cmd.env_remove(&k);
            }
        }
        unsafe {
            cmd.pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }

        let mut child = cmd.spawn()?;
        let pgid = child.id() as i32;

        // Marker channel — the child's fd 1, i.e. what ZTST_fd dups
        // (ztst.zsh:198). Byte-wise read with lossy conversion so a
        // stray binary write can't kill the reader.
        let stdout = child.stdout.take().expect("stdout piped");
        let (tx, lines_rx) = mpsc::channel::<String>();
        thread::spawn(move || {
            let mut rd = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match rd.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buf);
                        if tx.send(line.trim_end_matches('\n').to_string()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        // Harness stderr (prep chunks write here — ztst.zsh:315 nulls
        // only their stdout). Drain so the pipe never fills.
        let stderr = child.stderr.take().expect("stderr piped");
        thread::spawn(move || {
            let mut sink = Vec::new();
            let mut rd = BufReader::new(stderr);
            let _ = rd.read_to_end(&mut sink);
        });

        let mut shell = PersistentShell {
            child,
            pgid,
            lines_rx,
            fifo,
            sandbox,
            seq: 0,
            nonce,
            dead: false,
            ztst_in,
            ztst_tout,
            ztst_terr,
            qout,
            qerr,
        };

        // Startup handshake — proves the preamble ran (driver exits 1 at
        // the ztst.zsh:118-121 mirror if $ZTST_tmp can't be created).
        // Deadline is independent of the per-chunk timeout: when cargo
        // runs the 70 per-file tests in parallel, a dozen cold zshrs
        // instances (each with a fresh sandbox $HOME and cache) start at
        // once and routinely need more than the 2s chunk budget.
        let handshake = Duration::from_millis(timeout_ms().max(15_000));
        match shell.run_payload("ZTST_status=0", "", handshake) {
            ChunkOutcome::Done { .. } => Ok(shell),
            _ => Err(std::io::Error::other("harness shell failed to start")),
        }
    }

    /// Send one chunk payload and wait for its marker line.
    /// `aux_expr` is interpolated into the marker print (e.g.
    /// `${ZTST_unimplemented:-}` after prep chunks, `$ZTST_mskip` after
    /// test chunks). Backslash-prefixed words so chunk-defined aliases
    /// can't capture the wrapper (functions are still found).
    fn run_payload(&mut self, body: &str, aux_expr: &str, timeout: Duration) -> ChunkOutcome {
        if self.dead {
            return ChunkOutcome::Died;
        }
        self.seq += 1;
        let marker = format!("__ZTST_M_{}_{}__", self.nonce, self.seq);
        let payload = format!(
            "{body}\n\\builtin print -r -u $ZTST_fd -- \"{marker} $ZTST_status {aux_expr}\"\n"
        );

        let deadline = Instant::now() + timeout;

        // The child only opens the FIFO for reading when it's idle at the
        // `. fifo` line, so open with O_NONBLOCK (fails ENXIO until a
        // reader exists) and retry until the deadline.
        let file = loop {
            match fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&self.fifo)
            {
                Ok(f) => break f,
                Err(_) => {
                    if Instant::now() >= deadline {
                        self.kill();
                        return ChunkOutcome::Timeout;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            }
        };
        // Clear O_NONBLOCK so large payloads block instead of EAGAIN.
        unsafe {
            use std::os::fd::AsRawFd;
            let fd = file.as_raw_fd();
            let fl = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, fl & !libc::O_NONBLOCK);
        }
        let mut file = file;
        if file.write_all(payload.as_bytes()).is_err() {
            self.kill();
            return ChunkOutcome::Died;
        }
        drop(file); // EOF ends the child's `.` of this payload

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.kill();
                return ChunkOutcome::Timeout;
            }
            match self.lines_rx.recv_timeout(remaining) {
                Ok(line) => {
                    if let Some(rest) = line.strip_prefix(&marker) {
                        let rest = rest.trim_start();
                        let (status_s, aux) = rest.split_once(' ').unwrap_or((rest, ""));
                        let status = status_s.parse::<i32>().unwrap_or(-1);
                        return ChunkOutcome::Done {
                            status,
                            aux: aux.trim().to_string(),
                        };
                    }
                    // Anything else on the harness fd is a leak from the
                    // chunk; the real harness mixes it into the log too.
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.kill();
                    return ChunkOutcome::Timeout;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.dead = true;
                    let _ = self.child.wait();
                    return ChunkOutcome::Died;
                }
            }
        }
    }

    fn kill(&mut self) {
        if !self.dead {
            unsafe {
                libc::kill(-self.pgid, libc::SIGKILL);
            }
            let _ = self.child.wait();
            self.dead = true;
        }
    }

    /// ZTST_cleanup analog (ztst.zsh:131-134) — ask the shell to exit,
    /// then remove the whole sandbox.
    fn shutdown(mut self) {
        if !self.dead {
            if let Ok(mut f) = fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&self.fifo)
            {
                let _ = f.write_all(b"exit 0\n");
            }
            let waited = Instant::now();
            while waited.elapsed() < Duration::from_millis(1000) {
                if let Ok(Some(_)) = self.child.try_wait() {
                    self.dead = true;
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            self.kill();
        }
        let _ = fs::remove_dir_all(&self.sandbox);
    }
}

impl Drop for PersistentShell {
    fn drop(&mut self) {
        self.kill();
        let _ = fs::remove_dir_all(&self.sandbox);
    }
}

fn run_ztst_file(zshrs: &Path, ztst_path: &Path) -> (usize, usize, usize) {
    let verbose = env::var("ZTST_VERBOSE").map(|v| v != "0").unwrap_or(false);
    let ztst = parse_ztst(ztst_path);

    if ztst.tests.is_empty() {
        return (0, 0, 0);
    }

    let timeout = Duration::from_millis(timeout_ms());
    let corpus = ztst_path.parent().unwrap_or(Path::new("."));
    let stem = ztst_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut shell = match PersistentShell::spawn(zshrs, &stem, corpus) {
        Ok(s) => s,
        Err(e) => panic!("failed to start persistent shell for {}: {}", ztst.name, e),
    };

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;

    // %prep — ZTST_prep (ztst.zsh:309-322): each chunk runs via
    // ZTST_execchunk with stdout nulled (stderr flows to the harness log,
    // not into any comparison). A chunk setting ZTST_unimplemented stops
    // the remaining prep chunks (ztst.zsh:314) and skips every test
    // (ztst.zsh:587,624-626). A nonzero prep status fails the whole file
    // and the tests are not run (ztst.zsh:315-319,594,602).
    let mut unimplemented: Option<String> = None;
    let mut prep_failed: Option<String> = None;
    for chunk in &ztst.prep {
        let body = format!("ZTST_code={}\n\\ZTST_execchunk >/dev/null", sq(chunk));
        match shell.run_payload(&body, "${ZTST_unimplemented:-}", timeout) {
            ChunkOutcome::Done { status, aux } => {
                if !aux.is_empty() {
                    unimplemented = Some(aux);
                    break;
                }
                if status != 0 {
                    // ztst.zsh:316-317 "non-zero status from preparation code"
                    prep_failed = Some(format!(
                        "non-zero status {} from preparation code:\n{}",
                        status, chunk
                    ));
                    break;
                }
            }
            ChunkOutcome::Timeout => {
                prep_failed = Some(format!(
                    "TIMEOUT after {}ms in preparation code",
                    timeout_ms()
                ));
                break;
            }
            ChunkOutcome::Died => {
                prep_failed = Some("shell exited during preparation code".into());
                break;
            }
        }
    }

    if let Some(msg) = unimplemented {
        if verbose {
            eprintln!("  SKIP all {} tests: {}", ztst.tests.len(), msg);
        }
        shell.shutdown();
        return (0, 0, ztst.tests.len());
    }
    if let Some(msg) = prep_failed {
        // ztst.zsh:594,602 — tests are not run; the file is failed.
        eprintln!("  [{}] prep failed: {}", ztst.name, msg);
        shell.shutdown();
        return (0, ztst.tests.len(), 0);
    }

    // %test — ZTST_test (ztst.zsh:397-561) with ZTST_continue=1 semantics
    // (ztst.zsh:21,515,538,552): keep going after a failed case so every
    // chunk is measured. Comparison per chunk is unchanged.
    let mut wedged: Option<&'static str> = None;
    for (i, test) in ztst.tests.iter().enumerate() {
        if let Some(why) = wedged {
            failed += 1;
            eprintln!(
                "  [{}:{}] FAIL: {}\n      not run — {}",
                ztst.name,
                i + 1,
                test.message,
                why
            );
            continue;
        }

        // ztst.zsh:404-405 — fresh $ZTST_in each case. ZTST_getredir
        // writes redir bodies with `print -r --`, which terminates the
        // last line with \n (ztst.zsh:286).
        let mut body = String::new();
        if test.flags.contains('q') && !test.stdin_data.is_empty() {
            // ztst.zsh:282-285 — `q` + `<`: the stdin text gets ${(e)...}
            // expansion in the harness shell before the chunk runs.
            let _ = fs::write(&shell.ztst_in, "");
            body.push_str(&format!(
                "ZTST_redir={}\n\\builtin print -r -- \"${{(e)ZTST_redir}}\" >\"$ZTST_in\"\n",
                sq(&test.stdin_data)
            ));
        } else if test.stdin_data.is_empty() {
            let _ = fs::write(&shell.ztst_in, "");
        } else {
            let _ = fs::write(&shell.ztst_in, format!("{}\n", test.stdin_data));
        }

        // ztst.zsh:484 — the chunk runs in THIS shell via eval, with all
        // three streams redirected to the capture files.
        body.push_str(&format!(
            "ZTST_code={}\n\\ZTST_execchunk <\"$ZTST_in\" >\"$ZTST_tout\" 2>\"$ZTST_terr\"",
            sq(&test.code)
        ));

        // ztst.zsh:524-527,540-543 — `q` flag: expected stdout/stderr get
        // ${(e)...} expansion AFTER the chunk ran, in the SAME shell, so
        // variables set by %prep and earlier chunks are visible.
        let use_q = test.flags.contains('q')
            && (!test.expected_stdout.is_empty() || !test.expected_stderr.is_empty());
        if use_q {
            body.push_str(&format!(
                "\nZTST_expect_out={}\nZTST_expect_err={}\n\
                 \\builtin print -r -- \"${{(e)ZTST_expect_out}}\" >{} 2>/dev/null\n\
                 \\builtin print -r -- \"${{(e)ZTST_expect_err}}\" >{} 2>/dev/null",
                sq(&test.expected_stdout),
                sq(&test.expected_stderr),
                sq(&shell.qout.to_string_lossy()),
                sq(&shell.qerr.to_string_lossy()),
            ));
        }

        // ztst.zsh:486-488 — capture and reset ZTST_skip.
        body.push_str("\nZTST_mskip=\"${ZTST_skip:-}\"\nZTST_skip=");

        match shell.run_payload(&body, "$ZTST_mskip", timeout) {
            ChunkOutcome::Done { status, aux } => {
                if !aux.is_empty() {
                    // ztst.zsh:486-494 "Test case skipped"
                    skipped += 1;
                    if verbose {
                        eprintln!("  SKIP: {} ({})", test.message, aux);
                    }
                    continue;
                }
                let stdout =
                    String::from_utf8_lossy(&fs::read(&shell.ztst_tout).unwrap_or_default())
                        .into_owned();
                let stderr =
                    String::from_utf8_lossy(&fs::read(&shell.ztst_terr).unwrap_or_default())
                        .into_owned();

                // Harvest the (e)-expanded expectations for q-flag tests.
                let test_eff: TestBlock = if use_q {
                    let mut t = test.clone();
                    if let Ok(s) = fs::read_to_string(&shell.qout) {
                        if !test.expected_stdout.is_empty() {
                            t.expected_stdout = s.trim_end_matches('\n').to_string();
                        }
                    }
                    if let Ok(s) = fs::read_to_string(&shell.qerr) {
                        if !test.expected_stderr.is_empty() {
                            t.expected_stderr = s.trim_end_matches('\n').to_string();
                        }
                    }
                    let _ = fs::remove_file(&shell.qout);
                    let _ = fs::remove_file(&shell.qerr);
                    t
                } else {
                    test.clone()
                };
                let result = compare_test(&test_eff, status, &stdout, &stderr);

                if result.skipped {
                    skipped += 1;
                    if verbose {
                        eprintln!("  {}", result);
                    }
                } else if result.passed {
                    passed += 1;
                    if verbose {
                        eprintln!("  {}", result);
                    }
                } else {
                    failed += 1;
                    eprintln!("  [{}:{}] {}", ztst.name, i + 1, result);
                }
            }
            ChunkOutcome::Timeout => {
                // Hang kills the file's shell; remaining chunks can't run
                // (a wedged ztst run loses them the same way).
                failed += 1;
                eprintln!(
                    "  [{}:{}] FAIL: {}\n      TIMEOUT after {}ms",
                    ztst.name,
                    i + 1,
                    test.message,
                    timeout_ms()
                );
                wedged = Some("shell killed after earlier hang");
            }
            ChunkOutcome::Died => {
                failed += 1;
                eprintln!(
                    "  [{}:{}] FAIL: {}\n      shell exited during chunk",
                    ztst.name,
                    i + 1,
                    test.message
                );
                wedged = Some("shell exited during earlier chunk");
            }
        }
    }

    // %clean — ZTST_clean (ztst.zsh:310-322,610-616): chunk status is
    // ignored, stdout nulled. Not reached when the shell is gone.
    if wedged.is_none() {
        for chunk in &ztst.clean {
            let body = format!("ZTST_code={}\n\\ZTST_execchunk >/dev/null", sq(chunk));
            let _ = shell.run_payload(&body, "", timeout);
        }
    }

    shell.shutdown();
    (passed, failed, skipped)
}

fn compare_test(test: &TestBlock, status: i32, stdout: &str, stderr: &str) -> TestResult {
    let expected_fail = test.flags.contains('f');
    // ztst's `>line\n>` block joins to "line\n" — a single trailing
    // empty `>` from `print ${empty}` style cases. zsh's ztst harness
    // ignores the trailing newline-equivalence (one `>` blank vs zero
    // `>` blanks) when the body is otherwise identical. Trim trailing
    // newlines on both sides so the comparison ignores that mismatch.
    let actual_stdout = stdout.trim_end_matches('\n');
    let actual_stderr = stderr.trim_end_matches('\n');
    let expected_stdout_trim = test.expected_stdout.trim_end_matches('\n');
    let expected_stderr_trim = test.expected_stderr.trim_end_matches('\n');

    // Check exit status
    if let Some(expected) = test.expected_status {
        if status != expected {
            return TestResult {
                message: test.message.clone(),
                passed: expected_fail,
                skipped: false,
                detail: format!(
                    "exit status: expected {}, got {}\nstderr: {}",
                    expected, status, actual_stderr
                ),
            };
        }
    }

    // Check stdout (unless 'd' flag)
    // NOTE: no `!expected_stdout_trim.is_empty()` guard. ztst.zsh compares
    // with `command diff -u` (ZTST_do_diff, ztst.zsh:351-358), which fails
    // when the expected file is empty and the actual output is not. Skipping
    // the comparison in that case scored such chunks as PASSING — a false
    // pass (E03posix.ztst 11/12/18 were XPASSing this way).
    if !test.flags.contains('d') {
        let matches = if test.stdout_pattern {
            matchpat(expected_stdout_trim, actual_stdout)
        } else {
            expected_stdout_trim == actual_stdout
        };
        if !matches {
            return TestResult {
                message: test.message.clone(),
                passed: expected_fail,
                skipped: false,
                detail: format!(
                    "stdout mismatch\nexpected:\n{}\nactual:\n{}",
                    expected_stdout_trim, actual_stdout
                ),
            };
        }
    }

    // Check stderr (unless 'D' flag)
    // Same as stdout above: an empty expected stderr is still a real
    // expectation, not a licence to skip the check.
    if !test.flags.contains('D') {
        let matches = if test.stderr_pattern {
            matchpat(expected_stderr_trim, actual_stderr)
        } else {
            expected_stderr_trim == actual_stderr
        };
        if !matches {
            return TestResult {
                message: test.message.clone(),
                passed: expected_fail,
                skipped: false,
                detail: format!(
                    "stderr mismatch\nexpected:\n{}\nactual:\n{}",
                    expected_stderr_trim, actual_stderr
                ),
            };
        }
    }

    // If we get here, test passed — but if expected_fail, that's an xpass
    if expected_fail {
        return TestResult {
            message: test.message.clone(),
            passed: false,
            skipped: false,
            detail: "expected to fail but passed (XPASS)".into(),
        };
    }

    TestResult {
        message: test.message.clone(),
        passed: true,
        skipped: false,
        detail: String::new(),
    }
}

// ---------------------------------------------------------------------------
// Per-chunk replay — used by the extracted ztst-failure unit tests and
// their generator. Mirrors run_ztst_file's execution exactly (persistent
// shell, %prep, the same per-chunk body) but COLLECTS a TestResult per
// %test chunk (in order) instead of counting/printing. A prep that sets
// ZTST_unimplemented marks every chunk skipped; a prep failure / shell
// spawn failure marks every chunk failed (matching run_ztst_file).
// ---------------------------------------------------------------------------

fn run_file_results(zshrs: &Path, ztst_path: &Path) -> Vec<TestResult> {
    let ztst = parse_ztst(ztst_path);
    if ztst.tests.is_empty() {
        return Vec::new();
    }
    let mk = |t: &TestBlock, passed: bool, skipped: bool, detail: String| TestResult {
        message: t.message.clone(),
        passed,
        skipped,
        detail,
    };
    let all = |skipped: bool, detail: &str| -> Vec<TestResult> {
        ztst.tests
            .iter()
            .map(|t| mk(t, false, skipped, detail.to_string()))
            .collect()
    };

    let timeout = Duration::from_millis(timeout_ms());
    let corpus = ztst_path.parent().unwrap_or(Path::new("."));
    let stem = ztst_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut shell = match PersistentShell::spawn(zshrs, &stem, corpus) {
        Ok(s) => s,
        Err(e) => return all(false, &format!("shell spawn failed: {}", e)),
    };

    // %prep
    for chunk in &ztst.prep {
        let body = format!("ZTST_code={}\n\\ZTST_execchunk >/dev/null", sq(chunk));
        match shell.run_payload(&body, "${ZTST_unimplemented:-}", timeout) {
            ChunkOutcome::Done { status, aux } => {
                if !aux.is_empty() {
                    shell.shutdown();
                    return all(true, &format!("prep unimplemented: {}", aux));
                }
                if status != 0 {
                    shell.shutdown();
                    return all(false, &format!("prep failed: non-zero status {}", status));
                }
            }
            ChunkOutcome::Timeout => {
                shell.shutdown();
                return all(false, "prep TIMEOUT");
            }
            ChunkOutcome::Died => {
                shell.shutdown();
                return all(false, "prep shell exited");
            }
        }
    }

    // %test
    let mut results: Vec<TestResult> = Vec::with_capacity(ztst.tests.len());
    let mut wedged: Option<&'static str> = None;
    for test in &ztst.tests {
        if let Some(why) = wedged {
            results.push(mk(test, false, false, format!("not run — {}", why)));
            continue;
        }
        let mut body = String::new();
        if test.flags.contains('q') && !test.stdin_data.is_empty() {
            let _ = fs::write(&shell.ztst_in, "");
            body.push_str(&format!(
                "ZTST_redir={}\n\\builtin print -r -- \"${{(e)ZTST_redir}}\" >\"$ZTST_in\"\n",
                sq(&test.stdin_data)
            ));
        } else if test.stdin_data.is_empty() {
            let _ = fs::write(&shell.ztst_in, "");
        } else {
            let _ = fs::write(&shell.ztst_in, format!("{}\n", test.stdin_data));
        }
        body.push_str(&format!(
            "ZTST_code={}\n\\ZTST_execchunk <\"$ZTST_in\" >\"$ZTST_tout\" 2>\"$ZTST_terr\"",
            sq(&test.code)
        ));
        let use_q = test.flags.contains('q')
            && (!test.expected_stdout.is_empty() || !test.expected_stderr.is_empty());
        if use_q {
            body.push_str(&format!(
                "\nZTST_expect_out={}\nZTST_expect_err={}\n\
                 \\builtin print -r -- \"${{(e)ZTST_expect_out}}\" >{} 2>/dev/null\n\
                 \\builtin print -r -- \"${{(e)ZTST_expect_err}}\" >{} 2>/dev/null",
                sq(&test.expected_stdout),
                sq(&test.expected_stderr),
                sq(&shell.qout.to_string_lossy()),
                sq(&shell.qerr.to_string_lossy()),
            ));
        }
        body.push_str("\nZTST_mskip=\"${ZTST_skip:-}\"\nZTST_skip=");

        match shell.run_payload(&body, "$ZTST_mskip", timeout) {
            ChunkOutcome::Done { status, aux } => {
                if !aux.is_empty() {
                    results.push(mk(test, false, true, format!("skipped: {}", aux)));
                    continue;
                }
                let stdout =
                    String::from_utf8_lossy(&fs::read(&shell.ztst_tout).unwrap_or_default())
                        .into_owned();
                let stderr =
                    String::from_utf8_lossy(&fs::read(&shell.ztst_terr).unwrap_or_default())
                        .into_owned();
                let test_eff: TestBlock = if use_q {
                    let mut t = test.clone();
                    if let Ok(s) = fs::read_to_string(&shell.qout) {
                        if !test.expected_stdout.is_empty() {
                            t.expected_stdout = s.trim_end_matches('\n').to_string();
                        }
                    }
                    if let Ok(s) = fs::read_to_string(&shell.qerr) {
                        if !test.expected_stderr.is_empty() {
                            t.expected_stderr = s.trim_end_matches('\n').to_string();
                        }
                    }
                    let _ = fs::remove_file(&shell.qout);
                    let _ = fs::remove_file(&shell.qerr);
                    t
                } else {
                    test.clone()
                };
                results.push(compare_test(&test_eff, status, &stdout, &stderr));
            }
            ChunkOutcome::Timeout => {
                results.push(mk(
                    test,
                    false,
                    false,
                    format!("TIMEOUT after {}ms", timeout_ms()),
                ));
                wedged = Some("shell killed after earlier hang");
            }
            ChunkOutcome::Died => {
                results.push(mk(test, false, false, "shell exited during chunk".into()));
                wedged = Some("shell exited during earlier chunk");
            }
        }
    }

    shell.shutdown();
    results
}

/// Assertion used by the extracted (ignored) ztst-failure unit tests:
/// replay `filename`'s whole .ztst (so prior-chunk state is correct) and
/// assert that the 1-based `chunk` matches zsh's expectation. A chunk
/// that's currently SKIPPED (e.g. its module isn't built) passes
/// vacuously — the test pins a real OUTPUT/STATUS divergence only.
#[allow(dead_code)]
fn assert_ztst_chunk(filename: &str, chunk: usize) {
    let zshrs = find_zshrs();
    let corpus = find_test_corpus();
    let path = corpus.join(filename);
    let results = run_file_results(&zshrs, &path);
    let idx = chunk - 1;
    let r = results
        .get(idx)
        .unwrap_or_else(|| panic!("ztst {}: no chunk {}", filename, chunk));
    if r.skipped {
        return;
    }
    assert!(
        r.passed,
        "ztst {}:{} — {}\n{}",
        filename, chunk, r.message, r.detail
    );
}

/// Generator (run manually): replays every .ztst, and for each currently
/// FAILING chunk emits an `#[ignore]`d unit test calling assert_ztst_chunk
/// into `tests/gen/ztst_failures.rs` (a subdir so cargo doesn't compile it
/// as its own test binary; it's `include!`d at the bottom of this file).
///   cargo test --test ztst_runner gen_ztst_failures -- --ignored --nocapture
#[test]
#[ignore = "generator — run with --ignored to (re)write tests/gen/ztst_failures.rs"]
fn gen_ztst_failures() {
    let zshrs = find_zshrs();
    let corpus = find_test_corpus();
    let mut files: Vec<PathBuf> = fs::read_dir(&corpus)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "ztst").unwrap_or(false))
        .collect();
    files.sort();

    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = String::new();
    out.push_str(
        "// AUTO-GENERATED by ztst_runner::gen_ztst_failures — DO NOT EDIT.\n\
         // One #[ignore]d test per .ztst chunk that currently diverges from zsh.\n\
         // Each replays its whole .ztst file (prior-chunk state intact) and\n\
         // asserts the chunk's expected stdout/status; un-ignore as fixed.\n\n",
    );
    let mut count = 0usize;
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let results = run_file_results(&zshrs, path);
        for (i, r) in results.iter().enumerate() {
            if r.passed || r.skipped {
                continue;
            }
            let chunk = i + 1;
            // fn-name: lower stem + chunk, deduped.
            let stem: String = name
                .trim_end_matches(".ztst")
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect();
            let mut fname = format!("ztst_{}_{:03}", stem, chunk);
            while !used.insert(fname.clone()) {
                fname.push('x');
            }
            // ignore reason: file:chunk + the (one-line) message.
            let msg: String = r
                .message
                .chars()
                .take(90)
                .map(|c| if c == '"' || c == '\\' { ' ' } else { c })
                .collect();
            out.push_str(&format!(
                "#[test]\n#[ignore = \"ztst gap {}:{} — {}\"]\nfn {}() {{ assert_ztst_chunk(\"{}\", {}); }}\n\n",
                name, chunk, msg.trim(), fname, name, chunk
            ));
            count += 1;
        }
    }

    let gen_dir = Path::new("tests/gen");
    fs::create_dir_all(gen_dir).unwrap();
    fs::write(gen_dir.join("ztst_failures.rs"), out).unwrap();
    eprintln!(
        "generated {} ztst failure tests → tests/gen/ztst_failures.rs",
        count
    );
}

// ---------------------------------------------------------------------------
// Test entry points — one per .ztst file
// ---------------------------------------------------------------------------

fn run_ztst(filename: &str) {
    let zshrs = find_zshrs();
    let corpus = find_test_corpus();
    let path = corpus.join(filename);
    if !path.exists() {
        panic!("{} not found in {}", filename, corpus.display());
    }

    let (passed, failed, skipped) = run_ztst_file(&zshrs, &path);

    eprintln!(
        "  {} — {} passed, {} failed, {} skipped",
        filename, passed, failed, skipped
    );

    // Don't hard-fail yet — we're establishing a baseline.
    // Once the port matures, change this to: assert_eq!(failed, 0);
    if failed > 0 {
        eprintln!(
            "  NOTE: {} failures in {} (baseline mode — not failing CI)",
            failed, filename
        );
    }
}

// Generate a test function for each .ztst file.
// Macro keeps this DRY across all 70 files.
macro_rules! ztst_tests {
    ($($fn_name:ident => $file:expr),* $(,)?) => {
        $(
            #[test]
            fn $fn_name() {
                run_ztst($file);
            }
        )*
    };
}

ztst_tests! {
    // A — Shell Grammar
    a01_grammar          => "A01grammar.ztst",
    a02_alias            => "A02alias.ztst",
    a03_quoting          => "A03quoting.ztst",
    a04_redirect         => "A04redirect.ztst",
    a05_execution        => "A05execution.ztst",
    a06_assign           => "A06assign.ztst",
    a07_control          => "A07control.ztst",
    a08_time             => "A08time.ztst",
    // B — Builtins
    b01_cd               => "B01cd.ztst",
    b02_typeset          => "B02typeset.ztst",
    b03_print            => "B03print.ztst",
    b04_read             => "B04read.ztst",
    b05_eval             => "B05eval.ztst",
    b06_fc               => "B06fc.ztst",
    b07_emulate          => "B07emulate.ztst",
    b08_shift            => "B08shift.ztst",
    b09_hash             => "B09hash.ztst",
    b10_getopts          => "B10getopts.ztst",
    b11_kill             => "B11kill.ztst",
    b12_limit            => "B12limit.ztst",
    b13_whence           => "B13whence.ztst",
    // C — Shell features
    c01_arith            => "C01arith.ztst",
    c02_cond             => "C02cond.ztst",
    c03_traps            => "C03traps.ztst",
    c04_funcdef          => "C04funcdef.ztst",
    c05_debug            => "C05debug.ztst",
    // D — Expansion
    d01_prompt           => "D01prompt.ztst",
    d02_glob             => "D02glob.ztst",
    d03_procsubst        => "D03procsubst.ztst",
    d04_parameter        => "D04parameter.ztst",
    d05_array            => "D05array.ztst",
    d06_subscript        => "D06subscript.ztst",
    d07_multibyte        => "D07multibyte.ztst",
    d08_cmdsubst         => "D08cmdsubst.ztst",
    d09_brace            => "D09brace.ztst",
    d10_nofork           => "D10nofork.ztst",
    // E — Options / emulation
    e01_options          => "E01options.ztst",
    e02_xtrace           => "E02xtrace.ztst",
    e03_posix            => "E03posix.ztst",
    // K — Namerefs / advanced params
    k01_nameref          => "K01nameref.ztst",
    k02_parameter        => "K02parameter.ztst",
    // P — Privileged mode
    p01_privileged       => "P01privileged.ztst",
    // V — Modules
    v01_zmodload         => "V01zmodload.ztst",
    v02_zregexparse      => "V02zregexparse.ztst",
    v03_mathfunc         => "V03mathfunc.ztst",
    v04_features         => "V04features.ztst",
    v05_styles           => "V05styles.ztst",
    v06_parameter        => "V06parameter.ztst",
    v07_pcre             => "V07pcre.ztst",
    v08_zpty             => "V08zpty.ztst",
    v09_datetime         => "V09datetime.ztst",
    v10_private          => "V10private.ztst",
    v11_db_gdbm          => "V11db_gdbm.ztst",
    v12_zparseopts       => "V12zparseopts.ztst",
    v13_zformat          => "V13zformat.ztst",
    v14_system           => "V14system.ztst",
    // W — History / jobs
    w01_history          => "W01history.ztst",
    w02_jobs             => "W02jobs.ztst",
    w03_jobparameters    => "W03jobparameters.ztst",
    // X — ZLE
    x02_zlevi            => "X02zlevi.ztst",
    x03_zlebindkey       => "X03zlebindkey.ztst",
    x04_zlehighlight     => "X04zlehighlight.ztst",
    x05_zleincarg        => "X05zleincarg.ztst",
    x06_termquery        => "X06termquery.ztst",
    // Y — Completion
    y01_completion       => "Y01completion.ztst",
    y02_compmatch        => "Y02compmatch.ztst",
    y03_arguments        => "Y03arguments.ztst",
    // Z — Utility functions
    z01_is_at_least      => "Z01is-at-least.ztst",
    z02_zmathfunc        => "Z02zmathfunc.ztst",
    z03_run_help         => "Z03run-help.ztst",
}

/// Discovery test — finds all .ztst files and reports a summary.
/// Ignored by default: runs ALL files sequentially which duplicates the
/// individual per-file tests. Run explicitly with:
///   cargo test -p zsh --test ztst_runner ztst_summary -- --ignored --nocapture
#[test]
#[ignore]
fn ztst_summary() {
    let zshrs = find_zshrs();
    let corpus = find_test_corpus();

    let mut ztst_files: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("can't read test_corpus")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "ztst").unwrap_or(false))
        .collect();
    ztst_files.sort();

    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut total_skipped = 0usize;

    eprintln!("\n=== zshrs ztst integration test summary ===\n");

    for path in &ztst_files {
        let name = path.file_name().unwrap().to_string_lossy();
        let (p, f, s) = run_ztst_file(&zshrs, path);
        let status = if f == 0 { "OK" } else { "FAIL" };
        eprintln!(
            "  {:30} {:>4} pass {:>4} fail {:>4} skip  [{}]",
            name, p, f, s, status
        );
        total_passed += p;
        total_failed += f;
        total_skipped += s;
    }

    let total = total_passed + total_failed + total_skipped;
    eprintln!(
        "\n  TOTAL: {} tests — {} passed, {} failed, {} skipped",
        total, total_passed, total_failed, total_skipped
    );
    eprintln!(
        "  pass rate: {:.1}%\n",
        if total > 0 {
            total_passed as f64 / (total_passed + total_failed) as f64 * 100.0
        } else {
            0.0
        }
    );
}

// Extracted per-chunk ztst-failure unit tests (auto-generated by
// gen_ztst_failures). In a subdir so cargo doesn't treat it as a separate
// integration-test binary; included here so the tests share this file's
// assert_ztst_chunk / run_file_results helpers.
include!("gen/ztst_failures.rs");
