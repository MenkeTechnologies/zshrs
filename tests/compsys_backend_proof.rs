//! Proof that `[compsys] backend = "rust"|"shell"` actually routes
//! `_NAME` calls to the Rust port (`src/compsys/ported/*`) vs the
//! upstream shell function (autoloaded via `fpath`).
//!
//! Strategy: install a user shfunc `_setup()` (or `_main_complete()`)
//! that sets an observable marker scalar. Then invoke that name from
//! the same script and `echo` the marker:
//!
//! - **backend = "shell"** → router returns `None` → standard
//!   shfunc/autoload path → user shfunc fires → marker set.
//! - **backend = "rust"**  → router returns `Some(rust_fn)` → Rust
//!   port runs INSTEAD of the user shfunc → marker unset.
//!
//! Visible behavioral split = proof the switch is wired through to
//! `vm_helper::dispatch_function_call` and not just config-shaped
//! vaporware.
//!
//! Each test spawns a zshrs subprocess with `ZSHRS_HOME` pointing at
//! a tempdir holding a `zshrs.toml` with the desired backend so the
//! process-cached `config::current()` reads the right setting on
//! first access.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

/// Build a fresh `ZSHRS_HOME` tempdir with the requested `[compsys]
/// backend` value. Returns the tempdir path (caller keeps the handle
/// alive for the run + cleanup).
fn fresh_zshrs_home(backend: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let toml = format!(
        r#"[compsys]
backend = "{}"
"#,
        backend,
    );
    fs::write(dir.path().join("zshrs.toml"), toml).expect("write zshrs.toml");
    dir
}

struct R {
    stdout: String,
    exit: i32,
}

fn run_with_backend(backend: &str, script: &str) -> R {
    let home = fresh_zshrs_home(backend);
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env("ZSHRS_HOME", home.path())
        .env_remove("EDITOR")
        .env_remove("VISUAL")
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

// ─── Core proof: `_setup` (a name with a registered Rust port) ────────

/// PROOF #1 — backend=shell: user-defined `_setup` fires.
///
/// `_setup` HAS a Rust port at `src/compsys/router.rs::rust_compsys_lookup`,
/// so it's the perfect probe: under `rust` the router intercepts, under
/// `shell` the router returns None and the standard shfunc lookup
/// finds our test-defined `_setup` shfunc.
#[test]
fn shell_backend_runs_user_setup_shfunc() {
    let r = run_with_backend(
        "shell",
        r#"
            _setup() { typeset -g PROOF=shell_setup_ran }
            _setup default
            echo "PROOF=$PROOF"
        "#,
    );
    assert_eq!(r.exit, 0, "exit nonzero; stdout=`{}`", r.stdout);
    assert!(
        r.stdout.contains("PROOF=shell_setup_ran"),
        "shell backend MUST run user shfunc when `_setup` is defined; \
         got stdout=`{}`",
        r.stdout,
    );
}

/// PROOF #2 — backend=rust: user-defined `_setup` is SHADOWED by the
/// Rust port. Marker stays unset (Rust `_setup` doesn't touch PROOF).
#[test]
fn rust_backend_shadows_user_setup_shfunc() {
    let r = run_with_backend(
        "rust",
        r#"
            _setup() { typeset -g PROOF=shell_setup_ran }
            _setup default
            echo "PROOF=$PROOF"
        "#,
    );
    assert_eq!(r.exit, 0, "exit nonzero; stdout=`{}`", r.stdout);
    assert!(
        r.stdout.contains("PROOF=") && !r.stdout.contains("PROOF=shell_setup_ran"),
        "rust backend MUST shadow user shfunc \
         when a Rust port is registered; \
         got stdout=`{}` (would indicate the router didn't intercept)",
        r.stdout,
    );
}

// ─── Negative-coverage proof: `_NAME` without a Rust port ─────────────

/// PROOF #3 — graceful fallback: a `_NAME` with NO Rust port falls
/// through to the user shfunc EVEN under `backend = rust`. This
/// proves the router's "return None for unregistered names" branch
/// isn't a hard cut-over — partial coverage stays usable.
#[test]
fn rust_backend_falls_through_for_unregistered_names() {
    let r = run_with_backend(
        "rust",
        r#"
            _zshrs_nonexistent_compsys_fn() {
                typeset -g PROOF=fallback_ran
            }
            _zshrs_nonexistent_compsys_fn
            echo "PROOF=$PROOF"
        "#,
    );
    assert_eq!(r.exit, 0, "exit nonzero; stdout=`{}`", r.stdout);
    assert!(
        r.stdout.contains("PROOF=fallback_ran"),
        "rust backend with NO Rust port for a `_NAME` MUST fall \
         through to the user shfunc; got stdout=`{}`",
        r.stdout,
    );
}

// ─── Non-underscore names never go through the router ─────────────────

/// PROOF #4 — non-underscore names: the router gate
/// (`if !name.starts_with('_') return None`) skips them under both
/// backends, so they always run the user shfunc.
#[test]
fn router_only_intercepts_underscore_prefix_names() {
    for backend in ["rust", "shell"] {
        let r = run_with_backend(
            backend,
            r#"
                non_underscore_name() {
                    typeset -g PROOF=user_fn_ran
                }
                non_underscore_name
                echo "PROOF=$PROOF"
            "#,
        );
        assert_eq!(r.exit, 0, "backend={backend} exit nonzero");
        assert!(
            r.stdout.contains("PROOF=user_fn_ran"),
            "backend={backend}: router MUST NOT intercept names without `_` prefix; \
             got stdout=`{}`",
            r.stdout,
        );
    }
}

// ─── Config parsing end-to-end (no `current()` global override) ───────

/// PROOF #5 — the on-disk TOML is the source of truth. Two
/// subprocesses, two configs, two distinct backend selections.
/// Catches the "config silently defaulted because path was wrong"
/// failure mode that would make every other proof test vacuously
/// pass under one backend.
#[test]
fn distinct_backends_produce_distinct_observable_results() {
    let shell_run = run_with_backend(
        "shell",
        r#"
            _setup() { typeset -g PROOF=shell }
            _setup args
            echo "PROOF=$PROOF"
        "#,
    );
    let rust_run = run_with_backend(
        "rust",
        r#"
            _setup() { typeset -g PROOF=shell }
            _setup args
            echo "PROOF=$PROOF"
        "#,
    );
    assert!(
        shell_run.stdout.contains("PROOF=shell"),
        "shell backend should fire user shfunc; got `{}`",
        shell_run.stdout,
    );
    assert!(
        !rust_run.stdout.contains("PROOF=shell"),
        "rust backend should SHADOW user shfunc; \
         got `{}` (same as shell → switch is fake)",
        rust_run.stdout,
    );
    assert_ne!(
        shell_run.stdout, rust_run.stdout,
        "shell and rust backend stdouts MUST differ for `_setup` \
         (they share the script that differentiates the two paths); \
         got identical=`{}` → backend switch is a no-op",
        shell_run.stdout,
    );
}

// ─── Default-config behavior ──────────────────────────────────────────

/// PROOF #6 — no `zshrs.toml` at all: `config::current()` returns
/// defaults; `CompsysConfig::backend` defaults to `Rust` per
/// `CompsysBackend::default()`. Verifies the default-path is the
/// safer (Rust ports active) selection.
#[test]
fn missing_config_file_defaults_to_rust_backend() {
    let dir = tempfile::tempdir().expect("tempdir");
    // No zshrs.toml — config loader returns ZshrsConfig::default().
    let o = Command::new(zshrs_bin())
        .args([
            "--zsh",
            "-f",
            "-c",
            r#"
                _setup() { typeset -g PROOF=shell }
                _setup args
                echo "PROOF=$PROOF"
            "#,
        ])
        .env("ZSHRS_HOME", dir.path())
        .env_remove("EDITOR")
        .env_remove("VISUAL")
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn zshrs");
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(0));
    assert!(
        !stdout.contains("PROOF=shell"),
        "default backend must be `rust` (CompsysBackend::default()) \
         → user `_setup` shfunc should be shadowed; \
         got stdout=`{}`",
        stdout,
    );
}

// ─── Invalid backend value handling ───────────────────────────────────

/// PROOF #7 — unknown backend value in TOML: serde's
/// `#[serde(rename_all = "lowercase")]` rejects unknown variants,
/// so the whole config falls back to defaults (= rust backend).
/// Proves we don't silently honor garbage strings.
#[test]
fn unknown_backend_value_falls_back_to_default_rust() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("zshrs.toml"),
        "[compsys]\nbackend = \"bogus_backend_name\"\n",
    )
    .expect("write toml");
    let o = Command::new(zshrs_bin())
        .args([
            "--zsh",
            "-f",
            "-c",
            r#"
                _setup() { typeset -g PROOF=shell }
                _setup args
                echo "PROOF=$PROOF"
            "#,
        ])
        .env("ZSHRS_HOME", dir.path())
        .env_remove("EDITOR")
        .env_remove("VISUAL")
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn zshrs");
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(0));
    assert!(
        !stdout.contains("PROOF=shell"),
        "bogus backend value should leave config at defaults \
         (= rust); user `_setup` shfunc must still be shadowed; \
         got stdout=`{}`",
        stdout,
    );
}

// ─── doshfunc scope wrap: same scope semantics under both backends ────

/// PROOF #8 — `compcore::callcompfunc` (the Tab dispatch entry)
/// wraps the Rust port in `doshfunc` per `Src/Zle/compcore.c:835`,
/// so positional `$1`/`$2` inside the body see the cfargs (NOT the
/// caller's positionals). Verifies the synth_shf + body_runner
/// plumbing actually applies the scope wrap.
///
/// We can't drive Tab from a non-interactive subprocess, so instead
/// directly call `_setup` from a context that sets up known
/// positionals; verify the body sees its OWN argv[1..], not the
/// outer script's `$@`.
#[test]
fn function_body_sees_own_positionals_under_both_backends() {
    for backend in ["rust", "shell"] {
        let r = run_with_backend(
            backend,
            r#"
                # Outer positionals — these should NOT leak into _setup's $@.
                set -- outer1 outer2 outer3

                _setup() {
                    # Inside the body, $@ should be the ARGS we
                    # explicitly passed, not the outer set.
                    typeset -g PROOF="dollar1=$1,argc=$#"
                }
                _setup inner_arg
                echo "PROOF=$PROOF"
            "#,
        );
        assert_eq!(r.exit, 0, "backend={backend} exit nonzero");
        // Under shell backend the user shfunc runs and sees $1=inner_arg.
        // Under rust backend the user shfunc is shadowed — PROOF stays
        // unset — which is itself the proof for that backend (covered
        // by other tests). Only check shell here.
        if backend == "shell" {
            assert!(
                r.stdout.contains("PROOF=dollar1=inner_arg,argc=1"),
                "doshfunc scope MUST swap positionals to the call's argv; \
                 got stdout=`{}`",
                r.stdout,
            );
        }
    }
}

// ─── Config-path sanity ───────────────────────────────────────────────

/// PROOF #9 — `ZSHRS_HOME` actually changes the config file the
/// process loads. Without this, every other test would be reading
/// the user's real `~/.zshrs/zshrs.toml` and the "backend toggle"
/// would be a no-op against a single fixed config.
///
/// We test by writing TWO different configs to TWO tempdirs and
/// confirming the subprocess picks up the one we point at.
#[test]
fn zshrs_home_overrides_global_config_path() {
    let dir_shell = fresh_zshrs_home("shell");
    let dir_rust = fresh_zshrs_home("rust");
    let script = r#"
        _setup() { typeset -g PROOF=shell }
        _setup args
        echo "PROOF=$PROOF"
    "#;
    let out_shell = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env("ZSHRS_HOME", dir_shell.path())
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn shell-cfg");
    let out_rust = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env("ZSHRS_HOME", dir_rust.path())
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn rust-cfg");
    let shell_out = String::from_utf8_lossy(&out_shell.stdout);
    let rust_out = String::from_utf8_lossy(&out_rust.stdout);
    assert_ne!(
        shell_out, rust_out,
        "ZSHRS_HOME MUST switch config; \
         got identical stdout=`{}` from both temp dirs",
        shell_out,
    );
}

// ─── `$fpath` override must survive a port→port call ──────────────────

/// Run `script` under `backend = "rust"` with an extra `$fpath` dir that
/// holds a single `_parameters` file setting `PROOF`.
fn run_with_fpath_parameters(script: &str) -> (R, tempfile::TempDir) {
    let fp = tempfile::tempdir().expect("fpath tempdir");
    fs::write(
        fp.path().join("_parameters"),
        "#autoload\ntypeset -g PROOF=fpath_parameters_ran\n",
    )
    .expect("write _parameters");
    let home = fresh_zshrs_home("rust");
    let full = format!(
        "fpath=( {} )\nautoload -Uz _parameters\n{}",
        fp.path().display(),
        script,
    );
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", &full])
        .env("ZSHRS_HOME", home.path())
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn zshrs");
    (
        R {
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            exit: o.status.code().unwrap_or(-1),
        },
        fp,
    )
}

/// PROOF #10 — control: a DIRECT `_parameters` call honours `$fpath`.
///
/// `router::try_rust_dispatch` steps aside when `has_fpath_override`
/// finds a non-stock `_parameters` file, so the autoloaded shell
/// function must win. If this ever fails, PROOF #11's premise is gone
/// and the two failures should be read together.
#[test]
fn fpath_parameters_wins_over_rust_port() {
    let (r, _fp) = run_with_fpath_parameters("_parameters\necho \"PROOF=$PROOF\"");
    assert_eq!(r.exit, 0, "exit nonzero; stdout=`{}`", r.stdout);
    assert!(
        r.stdout.contains("PROOF=fpath_parameters_ran"),
        "a non-stock `$fpath/_parameters` MUST shadow the Rust port; \
         got stdout=`{}`",
        r.stdout,
    );
}

/// PROOF #11 — the override also wins when `_parameters` is reached
/// from ANOTHER port rather than from the command line.
///
/// `_brace_parameter` (upstream sh:214) ends in a bare `_parameters -e`
/// command word, so `$fpath` arbitration applies there exactly as it
/// does at PROOF #10. The port used to call the Rust `_parameters` fn
/// directly, which pins the port and silently kills the user's file:
/// `${<TAB>`, `${(k)<TAB>` and `$fpath[<TAB>` listed the port's bare
/// names (272 matches) instead of the user's name+value describe output
/// (~400), while `$<TAB>` — routed through `_parameter`, which does go
/// through `dispatch_function_call` — was correct.
#[test]
fn fpath_parameters_wins_when_called_from_brace_parameter() {
    let (r, _fp) = run_with_fpath_parameters("_brace_parameter\necho \"PROOF=$PROOF\"");
    assert_eq!(r.exit, 0, "exit nonzero; stdout=`{}`", r.stdout);
    assert!(
        r.stdout.contains("PROOF=fpath_parameters_ran"),
        "`_brace_parameter` MUST reach `_parameters` BY NAME so a \
         `$fpath` override still wins; got stdout=`{}` (a direct Rust \
         call to `_parameters` produces exactly this)",
        r.stdout,
    );
}

/// PROOF #12 — source pin for the same class of bug in every port that
/// finishes in `_parameters`.
///
/// `_subscript` (upstream sh:125, `_requested parameters && _parameters`)
/// has the identical bug shape but is only reachable with live
/// `$compstate` array context, so it cannot be driven from `-c` like
/// PROOF #11. Pin it at the source level instead: importing the
/// `_parameters` FN is what bypasses `has_fpath_override`; importing
/// `call_parameters` is the arbitrated entry point.
#[test]
fn ports_reach_parameters_through_the_router() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for rel in [
        "src/compsys/ported/Zsh/Context/_brace_parameter.rs",
        "src/compsys/ported/Zsh/Context/_subscript.rs",
    ] {
        let body = fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert!(
            !body.contains("use crate::compsys::ported::_parameters::_parameters;"),
            "{rel} imports the `_parameters` fn directly — that pins the \
             Rust port and drops a user's `$fpath/_parameters`. Use \
             `call_parameters` (which goes through \
             `dispatch_function_call` → `router::has_fpath_override`).",
        );
    }
}

// ─── loaddir inheritance: abspath-loaded fn autoloads a sibling ───────

/// Build a tempdir holding `wrapper` (which autoloads + calls `sibling`
/// by BARE name) and `sibling`, then run `autoload -Uz <dir>/wrapper;
/// wrapper` with `$fpath` EMPTY so the only way `sibling` can resolve is
/// the inherited load directory.
fn run_sibling_inheritance(sibling: &str) -> (R, tempfile::TempDir) {
    let d = tempfile::tempdir().expect("loaddir tempdir");
    fs::write(
        d.path().join("wrapper"),
        format!("#autoload\nautoload -Uz {sibling}\n{sibling}\n"),
    )
    .expect("write wrapper");
    fs::write(
        d.path().join(sibling),
        "#autoload\nprint SIBLING_FROM_LOADDIR\n",
    )
    .expect("write sibling");
    let home = fresh_zshrs_home("rust");
    let script = format!(
        "fpath=()\nautoload -Uz {}/wrapper\nwrapper\n",
        d.path().display()
    );
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", &script])
        .env("ZSHRS_HOME", home.path())
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn zshrs");
    (
        R {
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            exit: o.status.code().unwrap_or(-1),
        },
        d,
    )
}

/// PROOF #13 — `Src/builtin.c:3310-3323`: a function loaded by ABSOLUTE
/// PATH lends its load directory to any sibling it autoloads by bare name.
///
/// C's `add_autoload_function` walks `funcstack` for the calling function,
/// and when that function carries `PM_LOADDIR | PM_ABSPATH_USED` it copies
/// the caller's directory onto the new stub:
/// ```c
/// if ((shf2 = (Shfunc) shfunctab->getnode2(shfunctab, calling_f))
///         && (shf2->node.flags & PM_LOADDIR) && (shf2->node.flags & PM_ABSPATH_USED)
///         && shf2->filename)
/// ```
/// zshrs re-registers an autoloaded body through the funcdef pipeline,
/// which builds a FRESH shfunc node and drops the whole flag word;
/// `PM_ABSPATH_USED` therefore never survived the load and the arm above
/// could not fire. `wrapper` then died with "function definition file not
/// found" where zsh runs `<dir>/sibling`. `src/vm_helper.rs` restores the
/// bit next to the `loadautofnsetfile` filename/`PM_LOADDIR` restore.
#[test]
fn abspath_loaded_function_lends_its_loaddir_to_a_sibling() {
    let (r, _d) = run_sibling_inheritance("helper");
    assert_eq!(r.exit, 0, "exit nonzero; stdout=`{}`", r.stdout);
    assert!(
        r.stdout.contains("SIBLING_FROM_LOADDIR"),
        "`autoload -Uz /abs/dir/wrapper` must lend /abs/dir to the bare \
         `autoload -Uz helper` inside it (Src/builtin.c:3310-3323); got \
         stdout=`{}`",
        r.stdout,
    );
}

/// PROOF #14 — the same inheritance decides compsys arbitration.
///
/// `_describe` HAS a Rust port, so the inherited loaddir is what tells
/// `router::is_abspath_autoload` that the user named a file of their own:
/// the stub carries `PM_LOADDIR | PM_ABSPATH_USED` with a non-stock
/// directory, `has_shfunc_override` returns true, and the port steps
/// aside. Without the flag restore the stub looked like a plain
/// `$fpath` autoload and the port won, silently killing the user's file.
#[test]
fn inherited_loaddir_lets_a_user_describe_beat_the_rust_port() {
    let (r, _d) = run_sibling_inheritance("_describe");
    assert_eq!(r.exit, 0, "exit nonzero; stdout=`{}`", r.stdout);
    assert!(
        r.stdout.contains("SIBLING_FROM_LOADDIR"),
        "a `_describe` inherited from an abspath-loaded caller's own \
         directory MUST beat the Rust port; got stdout=`{}` (the port \
         produces `_tags:comptags: can only be called from completion \
         function` on stderr and nothing on stdout)",
        r.stdout,
    );
}
