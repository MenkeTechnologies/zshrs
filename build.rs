//! Build-time PORT.md drift enforcement.
//!
//! Runs the name-presence drift check (the spine of
//! `tests/ported_fn_names_match_c.rs`) on every cargo invocation
//! that touches the ported tree. Catches `pub fn name_not_in_zsh_c`
//! violations BEFORE any test binary compiles, so a bot doing
//! `cargo test --test foo` (which would otherwise skip the drift
//! tests) still gets stopped.
//!
//! This duplicates the core logic of `collect_free_fns` +
//! `load_c_fn_index` from the test. If you change either, mirror
//! the change here. Kept self-contained (std only) so the build
//! script has no extra dependencies.
//!
//! Re-run is gated by `cargo:rerun-if-changed` directives below —
//! unchanged builds skip the scan in the cargo cache.

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    // Only re-run when the inputs change. src/ported/** (the code
    // being checked), the C-name snapshot, and the allowlist files.
    println!("cargo:rerun-if-changed=src/ported");
    println!("cargo:rerun-if-changed=tests/data/zsh_c_fn_names.txt");
    println!("cargo:rerun-if-changed=tests/data/fake_fn_allowlist.txt");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/zsh/Completion");
    println!("cargo:rerun-if-changed=src/zsh/Functions");
    println!("cargo:rerun-if-changed=completions");
    bundle_zsh_functions();
    println!("cargo:rerun-if-changed=src/zsh/Config/version.mk");

    // Parse `src/zsh/Config/version.mk` for VERSION + VERSION_DATE
    // and emit them as compile-time constants. Replaces hardcoded
    // `"5.9"` / `"zsh-5.9-0-g73d3173"` literals in `src/vm_helper`
    // with values derived from the vendored zsh source so future
    // version bumps land automatically.
    let version_mk = manifest_dir.join("src/zsh/Config/version.mk");
    let (zsh_version, zsh_version_date) = parse_version_mk(&version_mk);
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("zsh_version.rs");
    let generated = format!(
        "/// Vendored zsh source `Config/version.mk` `VERSION=...` value.\n\
         /// `ZSH_VERSION` constant.
         pub const ZSH_VERSION: &str = {:?};\n\
         /// Vendored zsh source `Config/version.mk` `VERSION_DATE=...` value.\n\
         pub const ZSH_VERSION_DATE: &str = {:?};\n\
         /// `$ZSH_PATCHLEVEL` default — C `Src/params.c:43` sets `\"unknown\"`\n\
         /// when no custom value is configured. The vendored zsh tarball\n\
         /// doesn't ship a patchlevel hash; emit `\"unknown\"` to match\n\
         /// upstream's no-CUSTOM_PATCHLEVEL build.\n\
         pub const ZSH_PATCHLEVEL: &str = \"unknown\";\n",
        zsh_version, zsh_version_date
    );
    fs::write(&dest, generated).expect("write zsh_version.rs");

    // No terminal-handling library is linked. `link_term_lib` used to probe
    // for `ncursesw`/`ncurses`/`tinfow`/`tinfo`/`termcap`/`curses` and emit a
    // `cargo:rustc-link-lib` for whichever answered, reproducing
    // `configure.ac:725-771`. The eighteen symbols it supplied
    // (`tgetent`/`tgetstr`/`tigetstr`/`setupterm`/`tparm`/`tputs`/`tgoto`/…
    // and the `boolnames`/`strcodes` tables) are now pure Rust in
    // `src/extensions/terminfo_db.rs`, `terminfo_caps.rs` and `tparm.rs`,
    // which read the compiled terminfo database directly.
    //
    // That removes `libtinfo.so.6` as an install dependency on Ubuntu and
    // `libtinfo-dev`/`libncurses-dev` as build dependencies everywhere, and
    // it removes the divergence the probe itself could cause: which library
    // answered decided which terminfo database `${(kv)terminfo}` reflected.

    // Enlarge the MAIN-thread stack (binaries only). zshrs runs the
    // whole shell on the main thread to keep signal/job-control
    // semantics intact — but its per-call frames are heavy (fusevm
    // executor state, closures, parse buffers) and real configs
    // (p10k/zinit precmd hooks) nest deeply, so on the default 8 MB
    // main-thread stack a deep or runaway recursion overflowed and
    // SEGFAULTed before the FUNCNEST guard (exec.rs::doshfunc, default
    // 500) could fire its graceful error. macOS honors the Mach-O
    // `-stack_size` load command for the main thread; 256 MB comfortably
    // fits FUNCNEST-deep recursion (pages are committed on demand, so
    // idle RSS is unaffected). `rustc-link-arg-bins` scopes this to the
    // shipped binaries — the Rust test harness runs each test on its own
    // (RUST_MIN_STACK-sized) thread and drives zshrs as a subprocess, so
    // it is unaffected. Bug #643. (Linux governs the main-thread stack
    // via RLIMIT_STACK/ulimit, not a link flag, so this is macOS-only.)
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-arg-bins=-Wl,-stack_size,0x10000000");

    // Emit the `config.h` values that C's ./configure derives on the
    // build host rather than hard-codes. See `emit_config_h_env`.
    emit_config_h_env();

    let ported_root = manifest_dir.join("src/ported");
    let c_index_path = manifest_dir.join("tests/data/zsh_c_fn_names.txt");
    let allowlist_path = manifest_dir.join("tests/data/fake_fn_allowlist.txt");

    // If the ported root or the C-name snapshot is missing, this
    // isn't a configured port checkout — skip silently rather than
    // failing builds for unrelated workspaces.
    if !ported_root.exists() || !c_index_path.exists() {
        return;
    }

    let c_names = match load_c_fn_index(&c_index_path) {
        Ok(n) => n,
        Err(e) => {
            // Snapshot missing or unreadable — warn but don't break
            // the build. The full test will fail loudly if invoked.
            println!(
                "cargo:warning=PORT.md drift: cannot read {} ({})",
                c_index_path.display(),
                e
            );
            return;
        }
    };

    let allowlist: HashSet<String> = fs::read_to_string(&allowlist_path)
        .unwrap_or_default()
        .lines()
        .map(|l| {
            // Strip inline `#` comment so entries like
            // `name   # justification` parse to just `name`.
            let l = match l.find('#') {
                Some(i) => &l[..i],
                None => l,
            };
            l.trim()
        })
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();

    let mut rust_files: Vec<PathBuf> = Vec::new();
    collect_rust_files(&ported_root, &mut rust_files);

    let mut violations: Vec<String> = Vec::new();
    for path in &rust_files {
        let src = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let rel = path
            .strip_prefix(&manifest_dir)
            .unwrap_or(path)
            .display()
            .to_string();
        for (name, lineno) in collect_free_fns(&src) {
            if !allowlist.contains(&name) && !c_names.contains_key(&name) {
                violations.push(format!(
                    "  {}:{}  fn {} — no C counterpart in zsh source",
                    rel, lineno, name,
                ));
            }
        }
    }

    if !violations.is_empty() {
        violations.sort();
        // Emit cargo:warning for visibility in IDE / lint output,
        // then panic to fail the build. Cargo prints panic messages
        // before aborting.
        for v in &violations {
            println!("cargo:warning={}", v.trim());
        }
        panic!(
            "\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
             src/ported/ IS A PORT — NO NEW FUNCTIONS ALLOWED\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\n\
             Every `pub fn` / `fn` under src/ported/ MUST be a faithful\n\
             port of a function that exists in zsh's upstream C source\n\
             (~/forkedRepos/zsh/Src/, snapshotted at\n\
             tests/data/zsh_c_fn_names.txt). Rust-original helpers,\n\
             refactored extractions, convenience wrappers, and\n\
             ad-hoc abstractions DO NOT BELONG HERE — they create\n\
             drift between the port and the spec.\n\n\
             {} fn(s) violate this rule:\n\n\
             {}\n\n\
             To fix:\n\n\
               1. Preferred: inline the body at every call site\n\
                  (it isn't a real port, it shouldn't be a function).\n\
               2. Or: rename to match the actual C function it ports.\n\
                  Cite Src/<file>.c:<line> in the doc comment.\n\
               3. Last resort: add the name to\n\
                  tests/data/fake_fn_allowlist.txt with a comment\n\
                  explaining why no C analog exists (architectural\n\
                  Rust-only helpers like singleton accessors only).\n\n\
             Enforced by build.rs on every `cargo build` / `cargo test`\n\
             / `cargo check` whenever src/ported/** changes. Cannot be\n\
             bypassed by `cargo test --test X`.\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n",
            violations.len(),
            violations.join("\n")
        );
    }
}
/// Emit the `config.h` constants that C's `./configure` *derives* on the
/// build host, as `cargo:rustc-env` values consumed by
/// `src/ported/config_h.rs` via `env!()`.
///
/// C computes these once, at configure time, and freezes the result into
/// `config.h`; `Src/params.c:989-992` is the only reader for the host-triple
/// three. Freezing them as Rust literals reproduces one specific machine's
/// `./configure` run forever, which is wrong on every other machine and on
/// every other target.
///
/// Emitted here (each with its C derivation):
///   * `ZSHRS_CONFIG_OSTYPE`       — `configure.ac:47`  `AC_DEFINE_UNQUOTED(OSTYPE,   "$host_os", ...)`
///   * `ZSHRS_CONFIG_VENDOR`       — `configure.ac:45`  `AC_DEFINE_UNQUOTED(VENDOR,   "$host_vendor", ...)`
///   * `ZSHRS_CONFIG_DEFAULT_PATH` — `configure.ac:1954 AC_DEFINE_UNQUOTED(DEFAULT_PATH, "$zsh_cv_cs_path", ...)`
///   * `ZSHRS_CONFIG_PATH_DEV_FD`  — `configure.ac:1973 AC_DEFINE_UNQUOTED(PATH_DEV_FD, "$zsh_cv_sys_path_dev_fd")`
///
/// `MACHTYPE` (`configure.ac:43`, `"$host_cpu"`) is already derived in
/// `config_h.rs` from `std::env::consts::ARCH` and stays there — it needs no
/// build-script input.
fn emit_config_h_env() {
    let target = env::var("TARGET").unwrap_or_default();
    let host = env::var("HOST").unwrap_or_default();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    println!(
        "cargo:rustc-env=ZSHRS_CONFIG_OSTYPE={}",
        config_guess_host_os(&target, &host, &target_os, &target_env)
    );
    println!(
        "cargo:rustc-env=ZSHRS_CONFIG_VENDOR={}",
        config_guess_host_vendor(&target_os, &target_arch)
    );
    println!("cargo:rustc-env=ZSHRS_CONFIG_DEFAULT_PATH={}", cs_path());
    println!(
        "cargo:rustc-env=ZSHRS_CONFIG_PATH_DEV_FD={}",
        // configure.ac:1969 probes `/proc/self/fd` first, then `/dev/fd`.
        // Linux always satisfies the first (procfs); Darwin has no procfs
        // and satisfies the second.
        if target_os == "linux" || target_os == "android" {
            "/proc/self/fd"
        } else {
            "/dev/fd"
        }
    );
}

/// `$host_os` — the third field of the canonical triple that
/// `AC_CANONICAL_HOST` obtains from `config.guess` (then `config.sub`,
/// which is the identity on every triple zshrs targets).
///
/// The shape is *platform-specific*, which is why a bare `uname -r` is the
/// wrong derivation:
///
///   * Darwin — `config.guess:1463` `GUESS=aarch64-apple-darwin$UNAME_RELEASE`
///     and `config.guess:1500` `GUESS=$UNAME_PROCESSOR-apple-darwin$UNAME_RELEASE`,
///     with `config.guess:148` `UNAME_RELEASE=`(uname -r) 2>/dev/null``.
///     So `$host_os` is `darwin` immediately followed by the kernel release:
///     `darwin25.5.0`.
///   * Linux — `config.guess:1222` `GUESS=$CPU-pc-linux-$LIBCABI` (x86_64),
///     `config.guess:1009` `GUESS=$CPU-unknown-linux-$LIBCABI` (aarch64),
///     `config.guess:1177` `GUESS=$UNAME_MACHINE-unknown-linux-$LIBC`
///     (riscv32/riscv64). `$host_os` is `linux-$LIBC` — the C *library*, never
///     the kernel version. `$LIBC` is `gnu` (`config.guess:167`), `musl`
///     (`:176`, `:188`), `uclibc` (`:163`), or `android` (`:159`), and
///     `config.guess:193-195` falls back to `gnu` when it cannot tell.
///
/// Cross-compilation: `configure --host=aarch64-apple-darwin23` bypasses
/// `config.guess` entirely and takes `$host_os` straight from the triple, so
/// a versioned `darwinNN` suffix in `$TARGET` wins. The `uname -r` probe is
/// used only when the build host is itself Darwin, i.e. exactly the native
/// case `config.guess` handles.
fn config_guess_host_os(target: &str, host: &str, target_os: &str, target_env: &str) -> String {
    match target_os {
        "macos" => {
            // `aarch64-apple-darwin23` → "darwin23"; `aarch64-apple-darwin`
            // (Rust's unversioned spelling) → fall through to the probe.
            if let Some(rel) = target
                .rsplit('-')
                .next()
                .and_then(|f| f.strip_prefix("darwin"))
            {
                if !rel.is_empty() {
                    return format!("darwin{rel}");
                }
            }
            if host.contains("-darwin") {
                if let Some(rel) = uname_release() {
                    return format!("darwin{rel}");
                }
            }
            // Cross-compiled to macOS from a non-Darwin host with no version
            // in the triple: there is nothing to read a kernel release from.
            // Emit the un-suffixed name rather than a fabricated version.
            "darwin".to_string()
        }
        "linux" => format!(
            "linux-{}",
            match target_env {
                // Rust spells the glibc environment `gnu`, musl `musl`,
                // uClibc `uclibc` — the same tokens config.guess uses.
                "" => "gnu", // config.guess:193-195 — default to glibc
                other => other,
            }
        ),
        // config.guess:158-159 — `#if defined(__ANDROID__) LIBC=android`,
        // fed into the same `linux-$LIBC` shape as the branches above.
        "android" => "linux-android".to_string(),
        // Any other target: emit the triple's OS field verbatim. config.guess
        // appends a release number for several of these (`freebsd14.0`), which
        // is not recoverable from the Rust triple, so the value is the OS name
        // without a version rather than a fabricated one.
        "" => "unknown".to_string(),
        other => other.to_string(),
    }
}

/// `$host_vendor` — the second field of the canonical triple. It is a
/// function of (OS, CPU), not of the OS alone, so it cannot be derived from
/// `uname -s`:
///
///   * Darwin → `apple` (`config.guess:1463`, `config.guess:1500`).
///   * Linux x86_64 → `pc` (`config.guess:1222` `GUESS=$CPU-pc-linux-$LIBCABI`);
///     Linux i386..i686 → `pc` (`config.guess:1067`
///     `GUESS=$UNAME_MACHINE-pc-linux-$LIBC`).
///   * Linux s390/s390x → `ibm` (`config.guess:1180`
///     `GUESS=$UNAME_MACHINE-ibm-linux-$LIBC`).
///   * Linux aarch64 → `unknown` (`config.guess:1009`); Linux riscv32/riscv64
///     → `unknown` (`config.guess:1177`); Linux arm* → `unknown`
///     (`config.guess:1037`).
///
/// So on Linux the answer differs between the two architectures zshrs ships:
/// `x86_64-pc-linux-gnu` but `aarch64-unknown-linux-gnu`.
fn config_guess_host_vendor(target_os: &str, target_arch: &str) -> &'static str {
    match (target_os, target_arch) {
        ("macos" | "ios" | "tvos" | "watchos" | "visionos", _) => "apple",
        ("linux" | "android", "x86_64" | "x86") => "pc",
        ("linux" | "android", "s390x") => "ibm",
        // config.guess's dominant vendor field for every other Linux CPU,
        // and the safe default elsewhere.
        _ => "unknown",
    }
}

/// `$zsh_cv_cs_path` — `configure.ac:1944-1953`:
///
/// ```text
/// AC_CACHE_VAL(zsh_cv_cs_path,
/// [if getconf _CS_PATH >/dev/null 2>&1; then
///   zsh_cv_cs_path=`getconf _CS_PATH`
/// elif getconf CS_PATH >/dev/null 2>&1; then
///   zsh_cv_cs_path=`getconf CS_PATH`
/// elif getconf PATH >/dev/null 2>&1; then
///   zsh_cv_cs_path=`getconf PATH`
/// else
///   zsh_cv_cs_path="/bin:/usr/bin"
/// fi])
/// ```
///
/// This is a build-host probe in C as well (autoconf runs `getconf` on the
/// build machine even under `--host=`), so running it here reproduces C's
/// behaviour including its cross-compilation limitation. On macOS the first
/// two names are rejected (`getconf: no such configuration parameter
/// '_CS_PATH'`, exit 64) and `getconf PATH` answers
/// `/usr/bin:/bin:/usr/sbin:/sbin`.
fn cs_path() -> String {
    for name in ["_CS_PATH", "CS_PATH", "PATH"] {
        if let Ok(out) = std::process::Command::new("getconf").arg(name).output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return s;
                }
            }
        }
    }
    "/bin:/usr/bin".to_string()
}

/// `config.guess:148` — ``UNAME_RELEASE=`(uname -r) 2>/dev/null` ||
/// UNAME_RELEASE=unknown``. Returns `None` on the failure branch so the
/// caller can decide what to emit instead.
fn uname_release() -> Option<String> {
    let out = std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Parse `Config/version.mk` for the `VERSION=...` and
/// `VERSION_DATE='...'` lines. The file is a Makefile fragment +
/// shell script (see the file's header comment); the assignments use
/// `KEY=VALUE` with no spaces around the `=`.
fn parse_version_mk(path: &Path) -> (String, String) {
    let s = fs::read_to_string(path).unwrap_or_else(|_| String::new());
    let mut version = String::new();
    let mut version_date = String::new();
    for line in s.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("VERSION=") {
            version = v.trim().trim_matches('\'').trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("VERSION_DATE=") {
            version_date = v.trim().trim_matches('\'').trim_matches('"').to_string();
        }
    }
    if version.is_empty() {
        // version.mk missing or malformed — fall back to a marker so
        // the build still produces a valid const but the wrong value
        // surfaces as a visible "unknown" in `$ZSH_VERSION`.
        version = "unknown".to_string();
    }
    (version, version_date)
}

fn collect_rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Find free `fn NAME(` declarations at module level (depth 0).
/// Skips methods (anything inside `impl`/`trait`) and `mod tests`
/// blocks. Mirror of the test's `collect_free_fns` — keep in sync.
///
/// Brace-counting state machine ignores `{`/`}` that appear inside
/// `// line comments`, `/* block comments */`, `"string literals"`,
/// `'char literals'`, and `r"raw strings"` / `r#"hashed"#` — without
/// this, a test that contains `"{"` in a string literal corrupts the
/// depth tracker and the gate misfires after any reordering.
fn collect_free_fns(src: &str) -> Vec<(String, usize)> {
    let mut fns: Vec<(String, usize)> = Vec::new();
    let mut depth: i32 = 0;
    let mut in_test_mod = false;
    let mut test_mod_depth: i32 = 0;
    let mut in_block_comment: i32 = 0;

    for (lineno, line) in src.lines().enumerate() {
        let lineno = lineno + 1;
        let trimmed = line.trim_start();

        if depth == 0 && (trimmed.starts_with("mod tests {") || trimmed.starts_with("mod test {")) {
            in_test_mod = true;
            test_mod_depth = depth + 1;
        }

        let bytes = line.as_bytes();
        let mut i = 0;
        let mut delta: i32 = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if in_block_comment > 0 {
                if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    in_block_comment -= 1;
                    i += 2;
                    continue;
                }
                if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    in_block_comment += 1;
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }
            match b {
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => break,
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                    in_block_comment += 1;
                    i += 2;
                }
                b'"' => {
                    i += 1;
                    while i < bytes.len() {
                        let c = bytes[i];
                        if c == b'\\' {
                            i += 2;
                            continue;
                        }
                        if c == b'"' {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                }
                b'r' if i + 1 < bytes.len() && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') => {
                    let mut hashes = 0;
                    let mut j = i + 1;
                    while j < bytes.len() && bytes[j] == b'#' {
                        hashes += 1;
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b'"' {
                        i = j + 1;
                        loop {
                            if i >= bytes.len() {
                                break;
                            }
                            if bytes[i] == b'"' {
                                let mut closed = 0;
                                let mut k = i + 1;
                                while k < bytes.len() && bytes[k] == b'#' && closed < hashes {
                                    closed += 1;
                                    k += 1;
                                }
                                if closed >= hashes {
                                    i = k;
                                    break;
                                }
                            }
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                }
                b'\'' => {
                    let mut j = i + 1;
                    let mut found_close = false;
                    let mut escape = false;
                    while j < bytes.len() && j - i < 12 {
                        if !escape && bytes[j] == b'\'' {
                            found_close = true;
                            break;
                        }
                        escape = bytes[j] == b'\\' && !escape;
                        j += 1;
                    }
                    if found_close {
                        i = j + 1;
                    } else {
                        i += 1;
                        while i < bytes.len()
                            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                        {
                            i += 1;
                        }
                    }
                }
                b'{' => {
                    delta += 1;
                    i += 1;
                }
                b'}' => {
                    delta -= 1;
                    i += 1;
                }
                _ => i += 1,
            }
        }
        let pre_depth = depth;
        depth += delta;
        if in_test_mod && depth < test_mod_depth {
            in_test_mod = false;
        }

        if in_test_mod {
            continue;
        }
        if pre_depth != 0 {
            continue;
        }

        let stripped = trimmed
            .strip_prefix("pub(crate) ")
            .or_else(|| trimmed.strip_prefix("pub(super) "))
            .unwrap_or_else(|| trimmed.strip_prefix("pub ").unwrap_or(trimmed));
        let stripped = stripped.strip_prefix("unsafe ").unwrap_or(stripped);
        let stripped = stripped.strip_prefix("async ").unwrap_or(stripped);
        let stripped = stripped.strip_prefix(r#"extern "C" "#).unwrap_or(stripped);

        if let Some(rest) = stripped.strip_prefix("fn ") {
            let name_end = rest
                .find(|c: char| c == '(' || c == '<' || c.is_whitespace())
                .unwrap_or(0);
            if name_end > 0 {
                let name = rest[..name_end].to_string();
                if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    fns.push((name, lineno));
                }
            }
        }
    }
    fns
}

fn load_c_fn_index(path: &Path) -> Result<HashMap<String, HashSet<String>>, std::io::Error> {
    let src = fs::read_to_string(path)?;
    let mut index: HashMap<String, HashSet<String>> = HashMap::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((file, name)) = line.split_once(':') {
            index
                .entry(name.to_string())
                .or_default()
                .insert(file.to_string());
        }
    }
    Ok(index)
}


/// Pack the vendored zsh function tree into a single zstd blob for
/// `crate::bundled_functions` to materialise into `~/.zshrs/functions`.
///
/// zsh's own `make install` FLATTENS `Completion/**` and `Functions/**`
/// into one directory -- /opt/homebrew/Cellar/zsh/5.9.2/share/zsh/functions
/// holds 1235 files and zero subdirectories -- so the bundle is keyed by
/// basename and the installed layout matches it.
///
/// Format: repeated `u32 name_len | name | u32 body_len | body`, all
/// little-endian, then zstd. Deliberately not tar: this avoids a second
/// build-dependency for a payload we both write and read.
fn bundle_zsh_functions() {
    use std::io::Write;
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // `completions/` carries zshrs's OWN completions -- one per builtin in
    // ZSHRS_BUILTIN_NAMES (daemon/builtins.rs) plus the hand-written _zd --
    // so they land in ~/.zshrs/functions alongside zsh's tree.
    for root in ["src/zsh/Completion", "src/zsh/Functions", "completions"] {
        let mut stack = vec![PathBuf::from(root)];
        while let Some(dir) = stack.pop() {
            let rd = match fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(_) => continue,
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let name = match p.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                // Mirror what zsh's install skips: docs and build inputs.
                // completions/ also holds a data file that is not a function.
                if dir.ends_with("completions") && !name.starts_with('_') {
                    continue;
                }
                if name.starts_with('.')
                    || name.starts_with("README")
                    || name == "Makefile"
                    || name.ends_with(".in")
                    || name.ends_with(".mdd")
                    || name.ends_with(".pro")
                {
                    continue;
                }
                if !seen.insert(name.clone()) {
                    continue;
                }
                if let Ok(body) = fs::read(&p) {
                    files.push((name, body));
                }
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut raw: Vec<u8> = Vec::new();
    for (name, body) in &files {
        raw.extend_from_slice(&(name.len() as u32).to_le_bytes());
        raw.extend_from_slice(name.as_bytes());
        raw.extend_from_slice(&(body.len() as u32).to_le_bytes());
        raw.extend_from_slice(body);
    }
    let packed = zstd::encode_all(&raw[..], 19).expect("zstd encode zsh function bundle");
    let dest = out_dir.join("zsh_functions.zst");
    let mut f = fs::File::create(&dest).expect("create zsh_functions.zst");
    f.write_all(&packed).expect("write zsh_functions.zst");
    println!(
        "cargo:warning=bundled {} zsh functions ({} KiB packed)",
        files.len(),
        packed.len() / 1024
    );
}
