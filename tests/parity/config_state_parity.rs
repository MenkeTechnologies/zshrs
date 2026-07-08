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
const DUMP: &str = r####"
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
  for k in reswords patchars keymaps zle_bracketed_paste; do
    print -r -- "${k}=(${(j: :)${(qq)${(P)k}}})"
  done
  print -r -- '@@@ FUNCTIONS'
  print -rl -- ${(ok)functions:#zpwr__dump_state}
  print -r -- '@@@ ALIASES'
  alias | sort
  print -r -- '@@@ OPTIONS'
  local -a on=()
  for k in ${(ok)options}; do [[ ${options[$k]} == on ]] && on+=$k; done
  print -r -- "on=${(j: :)on}"
}
zpwr__dump_state
"####;

fn dump_in_zsh(fragment: &str) -> String {
    let script = format!("{fragment}\n{DUMP}");
    let o = Command::new(zsh_path())
        .args(["-fc", &script])
        .output()
        .expect("zsh");
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn dump_in_zshrs(fragment: &str) -> String {
    let script = format!("{fragment}\n{DUMP}");
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-c", &script])
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

