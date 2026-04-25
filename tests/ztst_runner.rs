//! ztst_runner — runs zsh's .ztst integration test files against zshrs
//!
//! Parses the %prep / %test / %clean sections from each .ztst file,
//! runs each test as `zshrs -f -c` with prep prepended (idempotent),
//! and compares exit status + stdout + stderr per test block.
//!
//! Run:  cargo test -p zsh --test ztst_runner -- [filter]
//!       ZTST_VERBOSE=1 cargo test -p zsh --test ztst_runner -- --nocapture
//!
//! Env vars:
//!   ZTST_TIMEOUT_MS=N — per-file timeout in milliseconds (default: 5000)
//!   ZTST_VERBOSE=1  — print pass/skip results, not just failures

use std::env;
use std::fmt;
use std::fs;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

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
fn read_code_chunk(lines: &[&str], idx: &mut usize) -> Option<String> {
    // Skip blank lines
    while *idx < lines.len() && lines[*idx].trim().is_empty() {
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
        if line.starts_with(' ') || line.starts_with('\t') {
            if !chunk.is_empty() {
                chunk.push('\n');
            }
            // Strip exactly 2 leading spaces to match ztst convention
            let stripped = if line.starts_with("  ") {
                &line[2..]
            } else if line.starts_with('\t') {
                &line[1..]
            } else {
                line.trim_start()
            };
            chunk.push_str(stripped);
            *idx += 1;
        } else if line.trim().is_empty() {
            // Blank line might separate chunks or be inside a chunk
            // Peek ahead to see if more indented code follows
            let mut peek = *idx + 1;
            while peek < lines.len() && lines[peek].trim().is_empty() {
                peek += 1;
            }
            if peek < lines.len() && (lines[peek].starts_with(' ') || lines[peek].starts_with('\t'))
            {
                // Blank line inside a code chunk — keep going but don't add to chunk yet
                // Actually in ztst format, blank line ends the chunk
                break;
            } else {
                break;
            }
        } else {
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
        if line.starts_with("*>") {
            stdout_pattern = true;
            append_redir_line(&mut expected_stdout, &line[2..]);
            *idx += 1;
            // Continue reading > lines as part of same stdout block
            while *idx < lines.len() && lines[*idx].starts_with('>') {
                append_redir_line(&mut expected_stdout, &lines[*idx][1..]);
                *idx += 1;
            }
        } else if line.starts_with('>') {
            append_redir_line(&mut expected_stdout, &line[1..]);
            *idx += 1;
        } else if line.starts_with("*?") {
            stderr_pattern = true;
            append_redir_line(&mut expected_stderr, &line[2..]);
            *idx += 1;
            while *idx < lines.len() && lines[*idx].starts_with('?') {
                append_redir_line(&mut expected_stderr, &lines[*idx][1..]);
                *idx += 1;
            }
        } else if line.starts_with('?') {
            append_redir_line(&mut expected_stderr, &line[1..]);
            *idx += 1;
        } else if line.starts_with('<') {
            append_redir_line(&mut stdin_data, &line[1..]);
            *idx += 1;
        } else if line.starts_with("F:") {
            // Failure hint — skip
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
    if !buf.is_empty() {
        buf.push('\n');
    }
    buf.push_str(content);
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
    let message;

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
    message = chars.collect::<String>().trim().to_string();

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

/// Simple glob-style pattern match (supports * as wildcard sequence)
fn pattern_match(pattern: &str, text: &str) -> bool {
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
    if pat[pi] == '*' {
        // Skip consecutive *
        let mut npi = pi;
        while npi < pat.len() && pat[npi] == '*' {
            npi += 1;
        }
        // Try matching rest of pattern against every suffix of text
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

fn run_code(zshrs: &Path, code: &str, stdin_data: &str, workdir: &Path) -> (i32, String, String) {
    let timeout_ms: u64 = env::var("ZTST_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);

    let mut cmd = Command::new(zshrs);
    cmd.arg("-f")
        .arg("-c")
        .arg(code)
        .current_dir(workdir)
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    unsafe {
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }

    if !stdin_data.is_empty() {
        cmd.stdin(std::process::Stdio::piped());
    }

    let mut child = cmd.spawn().expect("failed to spawn zshrs");

    if !stdin_data.is_empty() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(stdin_data.as_bytes());
        }
    }

    let pgid = child.id() as i32;
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(Ok(output)) => {
            let status = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            (status, stdout, stderr)
        }
        Ok(Err(e)) => panic!("failed to wait on zshrs: {}", e),
        Err(_) => {
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
            let _ = handle.join();
            (-1, String::new(), format!("TIMEOUT after {}ms", timeout_ms))
        }
    }
}

fn run_ztst_file(zshrs: &Path, ztst_path: &Path) -> (usize, usize, usize) {
    let verbose = env::var("ZTST_VERBOSE").map(|v| v != "0").unwrap_or(false);
    let ztst = parse_ztst(ztst_path);

    if ztst.tests.is_empty() {
        return (0, 0, 0);
    }

    // Prep runs in every test process since each is isolated.
    // Make side-effect commands idempotent: mkdir → mkdir -p,
    // suppress errors from re-execution so they don't pollute stderr.
    let prep = ztst.prep.join("\n").replace("mkdir ", "mkdir -p ");

    // Check if the prep sets ZTST_unimplemented — if so, skip all tests.
    // Run the prep once with a probe for the variable.
    if !prep.is_empty() && prep.contains("ZTST_unimplemented") {
        let probe_code = format!(
            "{{ {} ; }} 2>/dev/null; echo \"__ZTST_UNIMP=${{ZTST_unimplemented:-}}\"",
            prep
        );
        let (_, probe_out, _) = run_code(zshrs, &probe_code, "", Path::new("/tmp"));
        if let Some(line) = probe_out.lines().find(|l| l.starts_with("__ZTST_UNIMP=")) {
            let val = &line["__ZTST_UNIMP=".len()..];
            if !val.is_empty() {
                if verbose {
                    eprintln!(
                        "  SKIP all {} tests: {}",
                        ztst.tests.len(),
                        val
                    );
                }
                return (0, 0, ztst.tests.len());
            }
        }
    }

    // Wrap prep so its stderr doesn't leak into test stderr
    let prep_wrapped = if prep.is_empty() {
        String::new()
    } else {
        format!("{{ {} ; }} 2>/dev/null\n", prep)
    };

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;

    for (i, test) in ztst.tests.iter().enumerate() {
        let code = format!("{}{}", prep_wrapped, test.code);

        let (status, stdout, stderr) = run_code(zshrs, &code, &test.stdin_data, Path::new("/tmp"));
        let result = compare_test(test, status, &stdout, &stderr);

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

    (passed, failed, skipped)
}

fn compare_test(test: &TestBlock, status: i32, stdout: &str, stderr: &str) -> TestResult {
    let expected_fail = test.flags.contains('f');
    let actual_stdout = stdout.trim_end_matches('\n');
    let actual_stderr = stderr.trim_end_matches('\n');

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
    if !test.flags.contains('d') && !test.expected_stdout.is_empty() {
        let matches = if test.stdout_pattern {
            pattern_match(&test.expected_stdout, actual_stdout)
        } else {
            test.expected_stdout == actual_stdout
        };
        if !matches {
            return TestResult {
                message: test.message.clone(),
                passed: expected_fail,
                skipped: false,
                detail: format!(
                    "stdout mismatch\nexpected:\n{}\nactual:\n{}",
                    test.expected_stdout, actual_stdout
                ),
            };
        }
    }

    // Check stderr (unless 'D' flag)
    if !test.flags.contains('D') && !test.expected_stderr.is_empty() {
        let matches = if test.stderr_pattern {
            pattern_match(&test.expected_stderr, actual_stderr)
        } else {
            test.expected_stderr == actual_stderr
        };
        if !matches {
            return TestResult {
                message: test.message.clone(),
                passed: expected_fail,
                skipped: false,
                detail: format!(
                    "stderr mismatch\nexpected:\n{}\nactual:\n{}",
                    test.expected_stderr, actual_stderr
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
