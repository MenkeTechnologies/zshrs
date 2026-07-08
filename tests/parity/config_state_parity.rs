//! Differential *state* parity — the class of tests the isolated suites
//! miss.
//!
//! Every other parity suite asserts on the STDOUT of a small, self-contained
//! script. But real configs (`p10k`, `zinit`, `zpwr`, oh-my-zsh) break zshrs
//! not on any single feature but on the *accumulated intermediate state* they
//! build up — hundreds of parameters, functions, options, and the special
//! read-only tables that plugins consult. A shell can pass 46k feature tests
//! and still leave `$reswords` empty or emit an unquoted `[#]` from
//! `typeset -p`, and a framework that reads those silently misbehaves.
//!
//! This suite runs a config fragment through BOTH `zsh -f` and `zshrs`, then
//! appends a canonical state dump ([`DUMP`]) and diffs the two dumps
//! section-by-section. The first divergence is a real accumulated-state bug.
//!
//! Fragments must be self-contained and tty-free (no ZLE, no prompt render),
//! so they exercise the state-building path a real config walks without
//! needing an interactive session.

use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
}
fn zsh_path() -> &'static str {
    use std::path::Path;
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}
fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Canonical state dump appended after a fragment. Emits `typeset -p` for
/// every non-volatile parameter (value + type + flags), the read-only
/// special arrays plugins consult, function names, aliases, and the set
/// options. Host/process-volatile names are skipped so two processes agree.
/// Just the `zpwr__dump_state` function DEFINITION (no trailing call), so
/// the staged/cumulative harness can define it once and invoke it after
/// every stage. `DUMP` = this + a single call, preserving the one-shot
/// behaviour existing callers rely on.
const DUMP_FN: &str = r####"
zpwr__dump_state() {
  emulate -L zsh
  setopt no_nomatch extended_glob
  local k
  local -a volatile=(
    RANDOM SECONDS EPOCHREALTIME EPOCHSECONDS
    _ '?' '!' '$' '-' PPID '#' '*' '@' ARGC LINENO funcstack functrace
    funcfiletrace funcsourcetrace ZSH_EVAL_CONTEXT zsh_eval_context
    COLUMNS LINES HISTCMD SHLVL ZSH_SUBSHELL TTY TTYIDLE
    _comp_setup _comps _services _patcomps _postpatcomps
    _lastcomp reply REPLY MATCH MBEGIN MEND match mbegin mend
    pipestatus status ZSH_ARGZERO 0
    commands builtins dis_builtins dis_functions dis_reswords
    dis_patchars dis_galiases dis_saliases dis_aliases nameddirs
    userdirs functions_source dis_functions_source aliases galiases
    saliases functions dis_functions parameters options modules
    jobstates jobdirs jobtexts historywords dirstack history widgets
    termcap terminfo usergroups watch WATCH zsh_scheduled_events
    # zsh/parameter-module autoload stubs: zsh keeps them PM_UNSET
    # (`typeset -p` skips them) until the module is accessed; their
    # VALUES are compared in the SPECIAL_ARRAYS section below instead.
    reswords patchars keymaps zle_bracketed_paste
    OSTYPE MACHTYPE VENDOR CPUTYPE HOST HOSTNAME LOGCHECK WATCHFMT
    ZSHRS_VERSION ZSH_VERSION ZSH_PATCHLEVEL ZSH_NAME
    __CF_USER_TEXT_ENCODING PWD OLDPWD PATH path fpath FPATH cdpath
    manpath MANPATH module_path
  )
  local -A skip
  for k in $volatile; do skip[$k]=1; done
  print -r -- '@@@ PARAMETERS'
  for k in ${(ok)parameters}; do
    (( ${+skip[$k]} )) && continue
    typeset -p -- $k 2>/dev/null
  done
  print -r -- '@@@ SPECIAL_ARRAYS'
  # One element per line: these are read-only SETS whose ORDER is hash-order
  # and not meaningful. The section diff is set-based, so per-line membership
  # comparison is order-independent (and avoids the `(o)`+`(j)` flag quirk).
  # (zle_bracketed_paste omitted: ZLE-init state, only populated in an
  # interactive session, empty in both shells non-interactively.)
  local e
  for e in ${(o)reswords}; do print -r -- "reswords[]=$e"; done
  for e in ${(o)patchars}; do print -r -- "patchars[]=$e"; done
  for e in ${(o)keymaps}; do print -r -- "keymaps[]=$e"; done
  print -r -- '@@@ FUNCTIONS'
  print -rl -- ${(ok)functions:#zpwr__dump_state}
  print -r -- '@@@ ALIASES'
  alias | sort
  print -r -- '@@@ OPTIONS'
  local -a on=()
  for k in ${(ok)options}; do [[ ${options[$k]} == on ]] && on+=$k; done
  print -r -- "on=${(j: :)on}"
  # ── intermediate state real frameworks accumulate beyond params/
  #    functions/aliases/options. Each is set-diffed line-by-line, so
  #    shared defaults cancel and only config-added (or divergent) entries
  #    surface. All six produce byte-identical baseline output in both
  #    shells non-interactively (verified: zero empty-config divergence).
  print -r -- '@@@ BINDKEYS'
  # MAIN keymap only. Iterating every keymap re-introduces a separate
  # vi-keymap `bindkey -L` range-coalescing divergence (zshrs emits
  # `-R a-z` ranges where zsh lists individual keys) that would drown
  # config-level findings; the emacs/main keymap renders identically.
  bindkey -L 2>/dev/null | sort
  print -r -- '@@@ ZSTYLES'
  zstyle -L 2>/dev/null | sort
  print -r -- '@@@ WIDGETS'
  zle -lL 2>/dev/null | sort
  print -r -- '@@@ MATHFUNCS'
  functions -M 2>/dev/null | sort
  print -r -- '@@@ NAMEDDIRS'
  hash -dm '*' 2>/dev/null | sort
  print -r -- '@@@ TRAPS'
  trap 2>/dev/null | sort
}
"####;

/// One-shot dump: define the function and call it once (existing callers).
fn dump_script() -> String {
    format!("{DUMP_FN}\nzpwr__dump_state\n")
}

/// Run `cmd` with args, killing it after `secs` so a hung shell (a real
/// finding for a config fragment) reports instead of blocking the suite.
/// Returns stdout, or `None` if it timed out.
fn run_with_timeout(cmd: &str, args: &[&str], secs: u64) -> Option<String> {
    use std::sync::mpsc;
    use std::time::Duration;
    let mut child = Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .env_remove("ZSHRS_CACHE")
        .spawn()
        .expect("spawn");
    let mut out = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        let _ = tx.send(s);
    });
    let id = child.id();
    let killer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(secs));
        // SIGKILL by pid if still alive; harmless if already gone.
        unsafe {
            libc::kill(id as libc::pid_t, libc::SIGKILL);
        }
    });
    let status = child.wait().ok();
    let stdout = rx.recv_timeout(Duration::from_secs(1)).ok();
    let _ = reader.join();
    let _ = killer.join();
    // If the process was killed by our timeout, treat as timeout.
    let killed = status
        .and_then(|s| s.code().is_none().then_some(()))
        .is_some();
    if killed {
        None
    } else {
        stdout
    }
}

fn dump_in_zsh(fragment: &str) -> String {
    let d = dump_script();
    let script = format!("{fragment}\n{d}");
    let o = Command::new(zsh_path())
        .args(["-fc", &script])
        .output()
        .expect("zsh");
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn dump_in_zshrs(fragment: &str) -> String {
    let d = dump_script();
    let script = format!("{fragment}\n{d}");
    // `-f` (NO_RCS) to match `run_zsh`'s `zsh -fc` — otherwise the two
    // shells start from different option defaults (rcs/globalrcs) and every
    // OPTIONS diff is a harness artifact rather than a real bug.
    let o = Command::new(zshrs_bin())
        .args(["-f", "--zsh", "-c", &script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// Split a dump into `@@@ SECTION` → lines. Set-based so an extra/missing
/// entry pinpoints one divergence instead of cascading a line shift through
/// everything after it.
fn sections(dump: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for line in dump.lines() {
        if let Some(name) = line.strip_prefix("@@@ ") {
            out.push((name.to_string(), Vec::new()));
        } else if let Some((_, lines)) = out.last_mut() {
            lines.push(line.to_string());
        }
    }
    out
}

/// Diff two dumps section-wise (set-based). Shared by the synthetic-fragment
/// and real-config paths.
fn divergences_between(z: &str, r: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let zs = sections(z);
    let rs = sections(r);
    let mut report = Vec::new();
    for (name, zlines) in &zs {
        let rlines = rs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, l)| l.clone())
            .unwrap_or_default();
        let zset: BTreeSet<&String> = zlines.iter().collect();
        let rset: BTreeSet<&String> = rlines.iter().collect();
        for only_zsh in zset.difference(&rset) {
            report.push(format!("[{name}] only in zsh  : {only_zsh}"));
        }
        for only_zshrs in rset.difference(&zset) {
            report.push(format!("[{name}] only in zshrs: {only_zshrs}"));
        }
    }
    report
}

/// Collect every accumulated-state divergence (lines present in exactly one
/// shell's dump), grouped by section. Empty means the state matched.
fn state_divergences(fragment: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let zs = sections(&dump_in_zsh(fragment));
    let rs = sections(&dump_in_zshrs(fragment));
    let mut report = Vec::new();
    for (name, zlines) in &zs {
        let rlines = rs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, l)| l.clone())
            .unwrap_or_default();
        let zset: BTreeSet<&String> = zlines.iter().collect();
        let rset: BTreeSet<&String> = rlines.iter().collect();
        for only_zsh in zset.difference(&rset) {
            report.push(format!("[{name}] only in zsh  : {only_zsh}"));
        }
        for only_zshrs in rset.difference(&zset) {
            report.push(format!("[{name}] only in zshrs: {only_zshrs}"));
        }
    }
    report
}

/// Assert a fragment leaves identical accumulated state in both shells.
fn assert_state_parity(fragment: &str) {
    if !zsh_available() {
        return;
    }
    let divs = state_divergences(fragment);
    if !divs.is_empty() {
        panic!(
            "{} accumulated-state divergence(s) for fragment:\n{fragment}\n{}",
            divs.len(),
            divs.join("\n")
        );
    }
}

// ─────────────────────────── the corpus ───────────────────────────
//
// Self-contained fragments exercising the state-building idioms real
// frameworks use. Passing means zsh and zshrs agree on ALL resulting
// state, not just one command's stdout.

/// Plain scalars/arrays/assocs + a function + option + alias.
#[test]
fn basic_declarations() {
    assert_state_parity(
        r#"
        typeset -g S=hello
        typeset -i N=42
        typeset -a A=(a b c)
        typeset -A M=(k1 v1 k2 v2)
        f() { print hi; }
        setopt extendedglob
        alias ll='ls -la'
    "#,
    );
}

/// `typeset -p` round-trip fidelity — frameworks (p10k `_p9k_must_init`)
/// build parameter signatures by re-sourcing declarations. Assoc keys that
/// need quoting (`[#]`, `[*]`) must round-trip.
#[test]
fn typeset_p_special_keys_roundtrip() {
    assert_state_parity(
        r#"
        typeset -A H
        H[foo]=1
        H['#']=hash
        H['*']=star
        H['a b']=spaced
    "#,
    );
}

/// add-zsh-hook — the precmd/chpwd/preexec arrays that every prompt uses.
#[test]
fn add_zsh_hook_arrays() {
    assert_state_parity(
        r#"
        autoload -Uz add-zsh-hook
        myprecmd() { :; }
        mychpwd() { :; }
        add-zsh-hook precmd myprecmd
        add-zsh-hook chpwd mychpwd
    "#,
    );
}

/// Associative-array accretion via subscript + `(k)`/`(v)` reads, the
/// zinit-ICE / p10k-state pattern.
#[test]
fn assoc_accretion() {
    assert_state_parity(
        r#"
        typeset -gA ICE
        local -a keys=(a b c d e)
        local k
        for k in $keys; do ICE[$k]="val_$k"; done
        typeset -ga ORDER=("${(@ok)ICE}")
    "#,
    );
}

/// Nested function definitions + local option scoping (`emulate -L`).
#[test]
fn nested_functions_local_options() {
    assert_state_parity(
        r#"
        outer() {
          emulate -L zsh
          setopt localoptions extendedglob
          inner() { print inner; }
        }
        outer
        typeset -g AFTER=$options[extendedglob]
    "#,
    );
}

/// ZLE widgets, zstyles, keybindings, math functions, named dirs, and
/// traps — the "intermediate state" every real framework accumulates
/// beyond params/functions/aliases/options. Exercises the six DUMP
/// sections added for richer real-config coverage. A single semantic
/// divergence (a missing widget, a mis-rendered `zle -N`/`bindkey`
/// line, a dropped zstyle) fails here in isolation rather than only
/// showing up buried in a 500-function real config.
#[test]
fn zle_zstyle_bindkey_math_trap_state() {
    assert_state_parity(
        r#"
        # ZLE widgets (user + aliased-body forms)
        my-widget() { :; }
        zle -N my-widget
        zle -N my-widget-alias my-widget
        # completion styles
        zstyle ':completion:*' menu select
        zstyle ':completion:*:descriptions' format '%d'
        zstyle ':completion:*' matcher-list 'm:{a-z}={A-Z}'
        # main-keymap keybindings
        bindkey '^X^T' transpose-chars
        bindkey '^Xa' beep
        # math functions
        _zmath() { (( REPLY = 1 )) }
        functions -M zmath 1 1 _zmath
        # named directories
        hash -d proj=/tmp/proj
        hash -d cfg=/tmp/cfg
        # traps
        trap 'echo bye' EXIT
        trap ':' USR1
    "#,
    );
}


// ═══════════════════ real-config corpus ═══════════════════
//
// The synthetic fragments above prove the mechanism; these feed it the
// ACTUAL config files installed on this machine (zinit, powerlevel10k,
// zpwr). Sourcing a real file builds the dense accumulated state — hundreds
// of functions, the plugin-manager's associative arrays, hook arrays,
// module special params — that only shows divergences in aggregate. These
// are DISCOVERY tests: they run against the developer's real installation
// and skip cleanly on a machine (e.g. CI) where the file isn't present.

/// Collapse runs of >=4 digits to `<N>` so process-volatile values — temp
/// files named with `$$` (`.temp50093-tommy`), epoch timestamps, socket
/// names — compare equal across two processes. Short numbers are preserved.
/// Applied only to the real-config path (real configs bake the PID into
/// dozens of paths); the synthetic corpus stays exact.
fn normalize_volatile_numbers(dump: &str) -> String {
    let mut out = String::with_capacity(dump.len());
    let mut buf = String::new();
    for ch in dump.chars() {
        if ch.is_ascii_digit() {
            buf.push(ch);
        } else {
            if buf.len() >= 4 {
                out.push_str("<N>");
            } else {
                out.push_str(&buf);
            }
            buf.clear();
            out.push(ch);
        }
    }
    if buf.len() >= 4 {
        out.push_str("<N>");
    } else {
        out.push_str(&buf);
    }
    out
}

/// Normalize a divergence line so a documented known-open baseline stays
/// portable and stable: absolute `$HOME` collapses to the literal `$HOME`
/// (real configs bake the installing user's home into dozens of paths),
/// and the zshrs anonymous-function counter (`_zshrs_anon_35`) collapses to
/// `_zshrs_anon_N` (the exact number shifts as unrelated code changes how
/// many anon functions a source pass creates).
fn normalize_for_baseline(line: &str, home: &str) -> String {
    // fast-syntax-highlighting wraps every widget under a `$RANDOM`-seeded
    // session token `-s<NNN>-r<NNN>-` (e.g. `_zsh_highlight_widget_orig-
    // s000-r656-accept-line`). Both shells produce the wrapper; only the
    // random suffix differs. Collapse it so the wrappers compare equal.
    let with_home = collapse_fsh_token(&line.replace(home, "$HOME"));
    // Collapse the digits in every `_zshrs_anon_<digits>` to a single `N`.
    // Single left-to-right rebuild — no in-place mutation that could
    // re-match its own output.
    const MARK: &str = "_zshrs_anon_";
    let mut out = String::with_capacity(with_home.len());
    let mut rest = with_home.as_str();
    while let Some(pos) = rest.find(MARK) {
        let after = pos + MARK.len();
        out.push_str(&rest[..after]);
        let digits_end = rest[after..]
            .find(|c: char| !c.is_ascii_digit())
            .map(|o| after + o)
            .unwrap_or(rest.len());
        if digits_end > after {
            out.push('N');
        }
        rest = &rest[digits_end..];
    }
    out.push_str(rest);
    out
}

/// Collapse fast-syntax-highlighting's `$RANDOM` session token `-s<d>-r<d>-`
/// to `-sN-rN-` so its widget wrappers compare equal across two processes.
fn collapse_fsh_token(s: &str) -> String {
    // Find `-s<digits>-r<digits>-` runs and replace with `-sN-rN-`.
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        // try to match `-s` <digits> `-r` <digits> `-`
        if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b's' {
            let mut j = i + 2;
            let ds = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > ds && j + 1 < bytes.len() && bytes[j] == b'-' && bytes[j + 1] == b'r' {
                let mut k = j + 2;
                let dr = k;
                while k < bytes.len() && bytes[k].is_ascii_digit() {
                    k += 1;
                }
                if k > dr && k < bytes.len() && bytes[k] == b'-' {
                    out.push_str("-sN-rN-");
                    i = k + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Source a real config file (relative to $HOME) through both shells and
/// compare accumulated state against a documented KNOWN-OPEN baseline.
///
/// This is a characterization/regression test, not a pass/fail purity gate:
/// real configs still surface open port-bugs (module-param typeset flags,
/// zinit's `for %…` self-load), so `known_open` records exactly those.
/// The test fails when:
///   * a NEW divergence appears that isn't in `known_open` (a regression), or
///   * a divergence in `known_open` DISAPPEARS (a fix landed → tighten the
///     baseline here so the win can't silently regress later).
/// Skips cleanly if the config file isn't installed (e.g. CI).
fn assert_real_config_parity(rel_path: &str, known_open: &[&str]) {
    use std::collections::BTreeSet;
    if !zsh_available() {
        return;
    }
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return,
    };
    let path = format!("{home}/{rel_path}");
    if !std::path::Path::new(&path).exists() {
        eprintln!("skip: {path} not installed");
        return;
    }
    let d = dump_script();
    let script = format!("source {path} 2>/dev/null\n{d}");
    let z = run_with_timeout(zsh_path(), &["-fc", &script], 45)
        .unwrap_or_else(|| panic!("zsh itself timed out sourcing {path}"));
    let bin = zshrs_bin();
    let r = match run_with_timeout(bin.to_str().unwrap(), &["-f", "--zsh", "-c", &script], 45) {
        Some(s) => s,
        None => panic!(
            "zshrs HUNG sourcing {path} (never returned) — a real finding: \
             the config triggers an unbounded loop / block that zsh handles"
        ),
    };
    let divs = divergences_between(&normalize_volatile_numbers(&z), &normalize_volatile_numbers(&r));
    let actual: BTreeSet<String> = divs
        .iter()
        .map(|l| normalize_for_baseline(l, &home))
        .collect();
    let expected: BTreeSet<String> = known_open.iter().map(|s| s.to_string()).collect();
    let regressions: Vec<&String> = actual.difference(&expected).collect();
    let fixed: Vec<&String> = expected.difference(&actual).collect();
    if !regressions.is_empty() || !fixed.is_empty() {
        let mut msg = format!(
            "real config {path} diverged from its known-open baseline\n\
             ({} known-open, {} actual):\n",
            expected.len(),
            actual.len()
        );
        if !regressions.is_empty() {
            msg.push_str(&format!(
                "\nNEW divergences (REGRESSION — investigate/fix):\n  {}\n",
                regressions
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            ));
        }
        if !fixed.is_empty() {
            msg.push_str(&format!(
                "\nknown-open divergences that DISAPPEARED (a fix landed — \
                 remove these from the baseline so the win is locked in):\n  {}\n",
                fixed
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            ));
        }
        panic!("{msg}");
    }
}

/// zinit core — defines the whole plugin-manager function set + `ZINIT`
/// assoc + registers its `@zinit-scheduler` precmd hook.
///
/// Known-open divergences (see docs/BUGS.md): the zsh/system + zsh/datetime
/// module params render with the wrong `typeset -p` flags (`-h` hidden
/// instead of `-r` readonly), and zinit's final `for %$ZINIT[BIN_DIR]`
/// self-load isn't executed (so `zsh_loaded_plugins` stays empty and its
/// `autoload'zi-browse-symbol'` widget target lands as an anon function).
#[test]
fn real_zinit_core() {
    assert_real_config_parity(
        ".zinit/bin/zinit.zsh",
        &[
            // zsh/system + zsh/datetime module-param typeset -p flags:
            // zshrs emits PM_HIDE (`h`) and mis-handles PM_READONLY (`r`).
            "[PARAMETERS] only in zsh  : typeset -g -Ar sysparams",
            "[PARAMETERS] only in zshrs: typeset -g -Ah sysparams",
            "[PARAMETERS] only in zsh  : typeset -g -ar epochtime",
            "[PARAMETERS] only in zshrs: typeset -g -ahr epochtime",
            "[PARAMETERS] only in zsh  : typeset -g -ar errnos",
            "[PARAMETERS] only in zshrs: typeset -g -ah errnos",
            // zinit `for %$ZINIT[BIN_DIR]` self-load not executed.
            "[PARAMETERS] only in zsh  : typeset -g -a zsh_loaded_plugins=( %$HOME/.zinit/bin )",
            "[PARAMETERS] only in zshrs: typeset -g -a zsh_loaded_plugins=(  )",
            "[FUNCTIONS] only in zsh  : zi-browse-symbol",
        ],
    );
}

/// powerlevel10k icons table — pure data/param population.
#[test]
fn real_p10k_icons() {
    assert_real_config_parity(
        ".zinit/plugins/romkatv---powerlevel10k/internal/icons.zsh",
        &[],
    );
}

/// powerlevel10k core internals — the ~500 `_p9k_*` functions.
#[test]
fn real_p10k_internal() {
    assert_real_config_parity(
        ".zinit/plugins/romkatv---powerlevel10k/internal/p10k.zsh",
        &[],
    );
}

/// zpwr aliases/functions env file.
#[test]
fn real_zpwr_aliases_functions() {
    assert_real_config_parity(".zpwr/env/.shell_aliases_functions.sh", &[]);
}


// ═══════════════════ staged / cumulative real-config chain ═══════════════════
//
// The single-file tests above each source ONE config in isolation with a
// pristine `-f` shell. But a real startup is a CHAIN: the plugin manager
// sources its core, THEN dozens of plugins load *on top of* the state that
// core left behind — reading its assoc arrays, appending to its hook and
// widget tables, binding keys into keymaps a prior stage created. The bugs
// that only bite in production live in that accumulated intermediate state,
// not in any file read cold.
//
// This harness sources an ordered list of REAL, already-installed files in a
// SINGLE shell, cumulatively, and dumps the full accumulated state after each
// stage. zsh and zshrs are diffed stage-by-stage, so a divergence is pinned
// to the exact stage (plugin) that introduced it — and, crucially, the dump
// after stage N carries everything stages 1..N built, so a plugin that
// silently misbehaves *because* an earlier stage left zshrs in a subtly wrong
// state is caught here where the isolated tests pass.

/// A single stage in a cumulative chain: a label for reporting and a shell
/// snippet (typically `source <plugin entry>`) run on top of all prior stages.
struct Stage {
    label: &'static str,
    /// Path relative to `$HOME`; the stage is skipped (whole chain skipped)
    /// if it doesn't exist, so the test degrades cleanly off this machine.
    rel_path: &'static str,
}

/// Build the cumulative script: define the dump fn once, then for each stage
/// `source` it and re-dump under a `@@@@STAGE <label>` marker.
fn chain_script(home: &str, stages: &[Stage]) -> String {
    let mut s = String::from(DUMP_FN);
    s.push('\n');
    for st in stages {
        s.push_str(&format!(
            "source {home}/{path} 2>/dev/null\nprint -r -- '@@@@STAGE {label}'\nzpwr__dump_state\n",
            path = st.rel_path,
            label = st.label,
        ));
    }
    s
}

/// Split a chain dump into `(stage_label, per-stage dump text)` in order.
fn split_stages(dump: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in dump.lines() {
        if let Some(label) = line.strip_prefix("@@@@STAGE ") {
            out.push((label.to_string(), String::new()));
        } else if let Some(last) = out.last_mut() {
            last.1.push_str(line);
            last.1.push('\n');
        }
    }
    out
}

/// Source a real cumulative chain in both shells and diff the accumulated
/// state after EACH stage against the stage's known-open baseline. Skips if
/// any stage's file is missing. `known_open` maps a stage label to its
/// documented (HOME/anon-normalized) divergence lines; a stage absent from
/// the map must be byte-clean.
fn assert_chain_parity(stages: &[Stage], known_open: &[(&str, &[&str])]) {
    use std::collections::{BTreeSet, HashMap};
    if !zsh_available() {
        return;
    }
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return,
    };
    for st in stages {
        let p = format!("{home}/{}", st.rel_path);
        if !std::path::Path::new(&p).exists() {
            eprintln!("skip chain: stage {} file {p} not installed", st.label);
            return;
        }
    }
    let script = chain_script(&home, stages);
    // Chains source many plugins; give them generous headroom over the
    // single-file 45s budget.
    let z = run_with_timeout(zsh_path(), &["-fc", &script], 90)
        .unwrap_or_else(|| panic!("zsh itself timed out running the config chain"));
    let bin = zshrs_bin();
    let r = match run_with_timeout(bin.to_str().unwrap(), &["-f", "--zsh", "-c", &script], 90) {
        Some(s) => s,
        None => panic!(
            "zshrs HUNG running the config chain (never returned) — a real finding: \
             some stage triggers an unbounded loop / block zsh handles"
        ),
    };
    let zst = split_stages(&normalize_volatile_numbers(&z));
    let rst = split_stages(&normalize_volatile_numbers(&r));
    let known: HashMap<&str, &[&str]> = known_open.iter().copied().collect();
    let mut failures: Vec<String> = Vec::new();
    for (i, st) in stages.iter().enumerate() {
        let zd = zst.get(i).map(|(_, d)| d.as_str()).unwrap_or("");
        let rd = rst.get(i).map(|(_, d)| d.as_str()).unwrap_or("");
        let divs = divergences_between(zd, rd);
        let actual: BTreeSet<String> =
            divs.iter().map(|l| normalize_for_baseline(l, &home)).collect();
        let expected: BTreeSet<String> = known
            .get(st.label)
            .map(|v| v.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();
        let regressions: Vec<&String> = actual.difference(&expected).collect();
        let fixed: Vec<&String> = expected.difference(&actual).collect();
        if !regressions.is_empty() || !fixed.is_empty() {
            let mut m = format!("── stage [{}] ({})\n", st.label, st.rel_path);
            if !regressions.is_empty() {
                m.push_str("  NEW divergences (regression):\n");
                for l in &regressions {
                    m.push_str(&format!("    {l}\n"));
                }
            }
            if !fixed.is_empty() {
                m.push_str("  known-open divergences that DISAPPEARED (tighten baseline):\n");
                for l in &fixed {
                    m.push_str(&format!("    {l}\n"));
                }
            }
            failures.push(m);
        }
    }
    if !failures.is_empty() {
        panic!(
            "cumulative config-chain parity diverged at {} stage(s):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}

/// The real zpwr plugin set, loaded on top of zinit's core exactly as the
/// live `.zshrc` does. Each stage sources an actual installed plugin's entry
/// file; the dump after it carries zinit's ICE/hook/registration state PLUS
/// every earlier plugin's aliases, functions, widgets, and keybindings. This
/// is the "actual configs with all this intermediate state" case: a plugin
/// that reads or extends zinit's accumulated state is validated against zsh
/// with that exact state present, not in a vacuum.
#[test]
fn real_plugin_chain_on_zinit() {
    assert_chain_parity(
        &[
            Stage { label: "zinit", rel_path: ".zinit/bin/zinit.zsh" },
            Stage {
                label: "kubectl-aliases",
                rel_path: ".zinit/plugins/MenkeTechnologies---kubectl-aliases/kubectl-aliases.plugin.zsh",
            },
            Stage {
                label: "docker-aliases",
                rel_path: ".zinit/plugins/MenkeTechnologies---zsh-docker-aliases/docker-aliases.plugin.zsh",
            },
            Stage {
                label: "git-acp",
                rel_path: ".zinit/plugins/MenkeTechnologies---zsh-git-acp/zsh-git-acp.plugin.zsh",
            },
            Stage {
                label: "autopair",
                rel_path: ".zinit/plugins/hlissner---zsh-autopair/autopair.plugin.zsh",
            },
        ],
        // zinit's own known-open divergences (see real_zinit_core) carry
        // through every later stage's cumulative dump, so each stage from
        // zinit onward inherits the same baseline.
        &{
            const ZINIT_OPEN: &[&str] = &[
                "[PARAMETERS] only in zsh  : typeset -g -Ar sysparams",
                "[PARAMETERS] only in zshrs: typeset -g -Ah sysparams",
                "[PARAMETERS] only in zsh  : typeset -g -ar epochtime",
                "[PARAMETERS] only in zshrs: typeset -g -ahr epochtime",
                "[PARAMETERS] only in zsh  : typeset -g -ar errnos",
                "[PARAMETERS] only in zshrs: typeset -g -ah errnos",
                "[PARAMETERS] only in zsh  : typeset -g -a zsh_loaded_plugins=( %$HOME/.zinit/bin )",
                "[PARAMETERS] only in zshrs: typeset -g -a zsh_loaded_plugins=(  )",
                "[FUNCTIONS] only in zsh  : zi-browse-symbol",
            ];
            [
                ("zinit", ZINIT_OPEN),
                ("kubectl-aliases", ZINIT_OPEN),
                ("docker-aliases", ZINIT_OPEN),
                ("git-acp", ZINIT_OPEN),
                ("autopair", ZINIT_OPEN),
            ]
        },
    );
}


// ═══════════════ ALL installed plugins — final accumulated state ═══════════════
//
// The per-stage chain above pins divergences to a plugin but is capped at a
// handful of stages. This test DISCOVERS every `*.plugin.zsh` installed under
// ~/.zinit/plugins and sources them ALL on top of zinit in one shell, then
// diffs the final accumulated FUNCTION / ALIAS / WIDGET name sets against a
// documented known-open baseline. It is the broadest "actual config with all
// this intermediate state" check: ~45 real plugins layered together, exactly
// as the live .zshrc loads them. Focuses on the name sets plugins BUILD
// (not the full `typeset -p`, which bakes in machine-specific param values).

/// Discover installed plugin entry files (`<dir>/<*.plugin.zsh>`), sorted.
fn discover_plugin_entries(home: &str) -> Vec<String> {
    let root = format!("{home}/.zinit/plugins");
    let mut entries: Vec<String> = Vec::new();
    let Ok(dirs) = std::fs::read_dir(&root) else {
        return entries;
    };
    let mut dirs: Vec<_> = dirs.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    dirs.sort();
    for d in dirs {
        if !d.is_dir() {
            continue;
        }
        // First `*.plugin.zsh` in the dir (skip `.zwc` compiled copies).
        if let Ok(files) = std::fs::read_dir(&d) {
            let mut cands: Vec<String> = files
                .filter_map(|f| f.ok())
                .map(|f| f.path())
                .filter(|p| {
                    p.to_str()
                        .map(|s| s.ends_with(".plugin.zsh"))
                        .unwrap_or(false)
                })
                .filter_map(|p| p.to_str().map(String::from))
                .collect();
            cands.sort();
            if let Some(first) = cands.into_iter().next() {
                entries.push(first);
            }
        }
    }
    entries
}

#[test]
fn real_all_installed_plugins_final_state() {
    use std::collections::BTreeSet;
    if !zsh_available() {
        return;
    }
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return,
    };
    let zinit = format!("{home}/.zinit/bin/zinit.zsh");
    if !std::path::Path::new(&zinit).exists() {
        eprintln!("skip: zinit not installed");
        return;
    }
    let entries = discover_plugin_entries(&home);
    if entries.len() < 5 {
        eprintln!("skip: only {} plugins installed", entries.len());
        return;
    }
    // Build: source zinit + every plugin, then print name sets tagged.
    let mut script = format!("source {zinit} 2>/dev/null\n");
    for e in &entries {
        script.push_str(&format!("source {e} 2>/dev/null\n"));
    }
    script.push_str(
        "print -r -- '@@@ FUNCTIONS'; print -rl -- ${(ok)functions}\n\
         print -r -- '@@@ ALIASES'; alias | sort\n\
         print -r -- '@@@ WIDGETS'; zle -lL 2>/dev/null | sort\n",
    );
    let z = run_with_timeout(zsh_path(), &["-fc", &script], 120)
        .unwrap_or_else(|| panic!("zsh itself timed out sourcing all plugins"));
    let bin = zshrs_bin();
    let r = match run_with_timeout(bin.to_str().unwrap(), &["-f", "--zsh", "-c", &script], 120) {
        Some(s) => s,
        None => panic!("zshrs HUNG sourcing all {} plugins", entries.len()),
    };
    // fast-syntax-highlighting's `$RANDOM` widget-wrapper token differs per
    // process, so collapse it in the DUMPS (before the diff) — otherwise
    // every wrapper lands in a separate only-zsh / only-zshrs bucket and
    // never cancels. Volatile PIDs are handled the same way.
    // collapse_fsh_token FIRST (on the raw text): normalize_volatile_numbers
    // would otherwise mangle a 4-digit session id (`s1200` → `s<N>`) before
    // the `-s<d>-r<d>-` pattern can match, so one shell's token collapses and
    // the other's doesn't, and the wrappers never cancel.
    let zn = normalize_volatile_numbers(&collapse_fsh_token(&z));
    let rn = normalize_volatile_numbers(&collapse_fsh_token(&r));
    let divs = divergences_between(&zn, &rn);
    let actual: BTreeSet<String> = divs
        .iter()
        .map(|l| normalize_for_baseline(l, &home))
        .collect();
    // Known-open, all deep plugin-internal conditionals (see docs/BUGS.md):
    //   zi-browse-symbol — zinit's `for %…` self-load (not executed).
    //   -fast-highlight-string-process — fsh conditional definition.
    //   -zui_std_* — zui defines a different std-fn set (branch divergence).
    let known: BTreeSet<String> = [
        "[FUNCTIONS] only in zsh  : zi-browse-symbol",
        "[FUNCTIONS] only in zsh  : -fast-highlight-string-process",
        "[FUNCTIONS] only in zsh  : -zui_std_cleanup",
        "[FUNCTIONS] only in zsh  : -zui_std_init",
        "[FUNCTIONS] only in zshrs: -zui_std_fly_array_refresh",
        "[FUNCTIONS] only in zshrs: -zui_std_fly_mod_regen",
        "[FUNCTIONS] only in zshrs: -zui_std_fly_mod_regen_ext",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let regressions: Vec<&String> = actual.difference(&known).collect();
    let fixed: Vec<&String> = known.difference(&actual).collect();
    if !regressions.is_empty() || !fixed.is_empty() {
        let mut msg = format!(
            "all-plugins ({} sourced) final-state diverged from baseline:\n",
            entries.len()
        );
        if !regressions.is_empty() {
            msg.push_str("\nNEW divergences (regression — investigate/fix):\n");
            for l in &regressions {
                msg.push_str(&format!("  {l}\n"));
            }
        }
        if !fixed.is_empty() {
            msg.push_str("\nknown-open that DISAPPEARED (tighten baseline):\n");
            for l in &fixed {
                msg.push_str(&format!("  {l}\n"));
            }
        }
        panic!("{msg}");
    }
}
