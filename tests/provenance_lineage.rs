//! End-to-end lineage tests for the `provenance` builtin
//! (`src/extensions/provenance.rs`).
//!
//! The engine's unit tests pin the ledger's own semantics (staleness
//! reaping, chain inheritance, the FIFO bound). These tests pin the part
//! that only a running shell can prove: that the taps fire from real
//! bytecode execution, so a value's chain reflects what the VM actually
//! did — a command substitution's output flowing through a concat into a
//! parameter and out to a command's argv.

use std::process::Command;

fn zshrs_bin() -> String {
    env!("CARGO_BIN_EXE_zshrs").to_string()
}

/// Run `zshrs -f -c <script>` → (stdout, stderr, exit-code). `-f` skips
/// rc files so nothing but the script can arm the engine.
fn run(script: &str) -> (String, String, i32) {
    let out = Command::new(zshrs_bin())
        .args(["-f", "-c", script])
        .output()
        .expect("zshrs failed to spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// Write `body` to a uniquely named file under the test's temp dir and
/// run it with `zshrs -f <file>`, returning (path, stdout).
fn run_file(tag: &str, body: &str) -> (String, String) {
    let path = std::env::temp_dir().join(format!("zshrs_prov_{}_{}.zsh", tag, std::process::id()));
    std::fs::write(&path, body).expect("write script");
    let out = Command::new(zshrs_bin())
        .args(["-f", path.to_str().unwrap()])
        .output()
        .expect("zshrs failed to spawn");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    (path.to_string_lossy().into_owned(), stdout)
}

/// The op names in a `provenance NAME` report, in chain order. Each op
/// line is `  NN. OP  ARGS…  line N`.
fn ops(report: &str) -> Vec<String> {
    report
        .lines()
        .filter_map(|l| {
            let t = l.trim_start();
            let (num, rest) = t.split_once(". ")?;
            num.parse::<usize>().ok()?;
            rest.split_whitespace().next().map(|s| s.to_string())
        })
        .collect()
}

#[test]
fn command_substitution_is_the_origin_of_the_value_it_produces() {
    let (out, _, status) = run("provenance -m OUT\nOUT=$(echo hello world)\nprovenance OUT");
    assert_eq!(status, 0, "provenance OUT must succeed: {out}");
    assert!(
        out.contains(r#"origin: cmdsubst "echo hello world" (line 2, "#),
        "the substitution — not the assignment — is the origin:\n{out}"
    );
    assert_eq!(ops(&out), vec!["assign"], "report was:\n{out}");
}

#[test]
fn lineage_survives_concat_and_a_second_parameter() {
    // `${F}.bak` shares no bytes with `$F`, so this can only pass if the
    // concat bytecode op carried the lineage across.
    let script = "\
provenance -m F
provenance -m G
F=$(echo alpha)
G=${F}.bak
provenance G";
    let (out, _, status) = run(script);
    assert_eq!(status, 0, "{out}");
    assert!(
        out.contains(r#"origin: cmdsubst "echo alpha""#),
        "G must inherit F's origin, not start a new one:\n{out}"
    );
    assert_eq!(
        ops(&out),
        vec!["assign", "expand", "concat", "assign"],
        "report was:\n{out}"
    );
}

#[test]
fn consumption_records_the_command_and_the_argv_slot() {
    let script = "\
provenance -m F
F=$(echo alpha)
/bin/echo one $F > /dev/null
provenance F";
    let (out, _, status) = run(script);
    assert_eq!(status, 0, "{out}");
    assert!(
        out.contains("exec       /bin/echo argv[2]"),
        "the argv slot the value landed in must be recorded:\n{out}"
    );
    // The external command must be recorded exactly once — it reaches
    // both `call_function` and `exec` in the host, and only the latter
    // may claim it.
    assert_eq!(
        out.matches("/bin/echo").count(),
        1,
        "external command double-recorded:\n{out}"
    );
}

#[test]
fn a_shell_function_call_is_recorded_as_call_not_exec() {
    let script = "\
provenance -m F
takes_arg() { : }
F=$(echo alpha)
takes_arg $F
provenance F";
    let (out, _, status) = run(script);
    assert_eq!(status, 0, "{out}");
    let names = ops(&out);
    assert!(
        names.contains(&"call".to_string()),
        "function dispatch must record `call`: {names:?}\n{out}"
    );
    assert!(
        !names.contains(&"exec".to_string()),
        "a shell function must not also record `exec`: {names:?}\n{out}"
    );
}

#[test]
fn untracked_parameters_are_reported_as_such_and_exit_nonzero() {
    let (out, err, status) = run("NOPE=1\nprovenance NOPE");
    assert_eq!(status, 1, "stdout was: {out}");
    assert!(
        err.contains("zshrs: provenance: not tracked: NOPE"),
        "stderr was: {err}"
    );
    assert!(out.is_empty(), "nothing goes to stdout: {out}");
}

#[test]
fn untracking_drops_the_lineage() {
    let script = "\
provenance -m F
F=$(echo alpha)
provenance -u F
provenance F";
    let (_, err, status) = run(script);
    assert_eq!(status, 1);
    assert!(err.contains("not tracked: F"), "stderr was: {err}");
}

#[test]
fn json_output_carries_the_same_chain_as_the_text_report() {
    let script = "\
provenance -m F
F=$(echo alpha)
provenance -j F";
    let (out, _, status) = run(script);
    assert_eq!(status, 0, "{out}");
    assert!(
        out.starts_with(r#"{"name":"F","origin":"cmdsubst"#),
        "{out}"
    );
    assert!(out.contains(r#""op":"assign""#), "{out}");
    assert!(out.contains(r#""origin_line":2"#), "{out}");
    assert!(out.trim_end().ends_with(r#""dropped_ops":0}"#), "{out}");
}

#[test]
fn the_env_kill_switch_refuses_to_arm_the_engine() {
    let out = Command::new(zshrs_bin())
        .env("ZSHRS_PROVENANCE", "0")
        .args(["-f", "-c", "provenance -m F\nF=$(echo alpha)\nprovenance F"])
        .output()
        .expect("zshrs failed to spawn");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("zshrs: provenance: disabled by config"),
        "stderr was: {err}"
    );
    assert!(
        err.contains("not tracked: F"),
        "nothing may be tracked while disabled: {err}"
    );
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn a_bad_option_is_rejected_without_touching_the_ledger() {
    let (out, err, status) = run("provenance -Z");
    assert_eq!(status, 1, "{out}");
    assert!(err.contains("zshrs: provenance: bad option: -Z"), "{err}");
}

#[test]
fn a_script_names_its_file_and_line_on_every_row() {
    let (path, out) = run_file(
        "script",
        "provenance -m OUT\nOUT=$(echo hello)\nprovenance OUT\n",
    );
    assert!(
        out.contains(&format!("origin: cmdsubst \"echo hello\" ({}:2, ", path)),
        "the origin names the file it happened in:\n{out}"
    );
    assert!(
        out.contains(&format!("{}:2", path)),
        "the assign op names it too:\n{out}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_function_body_is_attributed_to_the_file_it_was_defined_in() {
    // The lineage is built inside `build`, defined in a *different* file
    // than the one running: the rows must name the definition file, at
    // the body's real line in it, not the caller's.
    let lib = std::env::temp_dir().join(format!("zshrs_prov_lib_{}.zsh", std::process::id()));
    std::fs::write(
        &lib,
        "build() {\n  STAMP=$(echo 42)\n  NAME=${STAMP}.log\n}\n",
    )
    .expect("write lib");
    let (main, out) = run_file(
        "call",
        &format!(
            "provenance -m NAME\nsource {}\nbuild\nprovenance NAME\n",
            lib.display()
        ),
    );
    assert!(
        out.contains(&format!("{}:2 (build)", lib.display())),
        "origin must be the substitution's line in the defining file:\n{out}"
    );
    assert!(
        out.contains(&format!("{}:3 (build)", lib.display())),
        "the assignment's line in the defining file:\n{out}"
    );
    assert!(
        !out.contains(&main),
        "the calling script is not where any of this happened:\n{out}"
    );
    let _ = std::fs::remove_file(&lib);
    let _ = std::fs::remove_file(&main);
}

#[test]
fn json_carries_the_file_function_and_timestamp_of_every_row() {
    let (path, out) = run_file(
        "json",
        "provenance -m OUT\nOUT=$(echo alpha)\nprovenance -j OUT\n",
    );
    assert!(
        out.contains(&format!(r#""origin_file":"{}""#, path)),
        "{out}"
    );
    assert!(out.contains(r#""origin_function":null"#), "{out}");
    assert!(out.contains(r#""file":"#), "{out}");
    // RFC 3339 local time with milliseconds, e.g. 2026-08-18T11:22:03.908-04:00.
    let stamp = out
        .split(r#""origin_time":""#)
        .nth(1)
        .and_then(|r| r.split('"').next())
        .unwrap_or_default()
        .to_string();
    assert!(
        stamp.len() >= 23 && stamp.contains('T') && stamp.contains('.'),
        "origin_time must be an RFC 3339 instant, got {stamp:?}:\n{out}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn track_all_arms_every_parameter_and_function_without_a_single_m() {
    let (path, out) = run_file(
        "trackall",
        "provenance -a\n\
         greet() {\n\
         \x20 MSG=\"hi $1\"\n\
         }\n\
         greet world\n\
         provenance MSG\n\
         provenance -f greet\n",
    );
    // The parameter armed itself on the write inside the function.
    assert!(
        out.contains("origin: assign \"hi world\""),
        "MSG must have a chain nobody armed:\n{out}"
    );
    // The function armed itself at its definition, and the call is an op.
    assert!(
        out.contains(&format!("greet()\n  origin: function greet ({}:2", path)),
        "the function's origin is its definition site:\n{out}"
    );
    assert!(
        out.contains(&format!(
            "call       greet()                                  {}:5",
            path
        )),
        "the call op stands at the caller's line:\n{out}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_env_switch_arms_everything_from_startup() {
    let path = std::env::temp_dir().join(format!("zshrs_prov_env_{}.zsh", std::process::id()));
    std::fs::write(&path, "V=$(echo x)\nprovenance V\n").expect("write script");
    let out = Command::new(zshrs_bin())
        .env("ZSHRS_PROVENANCE_ALL", "1")
        .args(["-f", path.to_str().unwrap()])
        .output()
        .expect("zshrs failed to spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(r#"origin: cmdsubst "echo x""#),
        "ZSHRS_PROVENANCE_ALL=1 must arm before the script runs:\n{stdout}"
    );

    // Same script, switch off: nothing is tracked.
    let off = Command::new(zshrs_bin())
        .env("ZSHRS_PROVENANCE_ALL", "0")
        .args(["-f", path.to_str().unwrap()])
        .output()
        .expect("zshrs failed to spawn");
    assert!(
        String::from_utf8_lossy(&off.stderr).contains("not tracked: V"),
        "=0 must leave the engine inert: {}",
        String::from_utf8_lossy(&off.stderr)
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_function_and_a_parameter_of_the_same_name_keep_separate_chains() {
    let (path, out) = run_file(
        "namespaces",
        "provenance -a\n\
         path_of() { REPLY=deep; }\n\
         path_of\n\
         provenance -f path_of\n\
         provenance REPLY\n",
    );
    assert!(
        out.contains("path_of()\n  origin: function path_of"),
        "{out}"
    );
    assert!(out.contains("REPLY\n  origin: assign \"deep\""), "{out}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn track_all_can_be_switched_off_again() {
    let (path, out) = run_file(
        "trackoff",
        "provenance -a\n\
         EARLY=1\n\
         provenance -ua\n\
         LATE=1\n\
         provenance -l\n",
    );
    assert!(out.contains("EARLY"), "what was recorded stays:\n{out}");
    // `LATE` is written only after `-ua`, so it never arms: the listing
    // shows `EARLY` and nothing else.
    assert!(!out.contains("LATE"), "no name arms after -ua:\n{out}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn every_redefinition_of_a_function_lands_on_its_chain() {
    // The `redefine` op only proves anything end-to-end: the VM's
    // funcdef opcode is where `f() { … }` actually installs, and an
    // earlier revision tapped only the interpreter path — the chain
    // still had an origin (seeded by the first call) and looked fine
    // while every redefinition went unrecorded.
    let (path, out) = run_file(
        "redef",
        "provenance -a\n\
         greet() { : one; }\n\
         greet\n\
         greet() { : two; }\n\
         greet\n\
         unfunction greet\n\
         provenance -f greet\n",
    );
    let ops = ops(&out);
    assert_eq!(
        ops,
        vec!["call", "redefine", "call", "unfunction"],
        "report was:\n{out}"
    );
    assert!(
        out.contains(&format!(
            "redefine   greet                                    {}:4",
            path
        )),
        "the redefinition stands at the new body's line:\n{out}"
    );
    let _ = std::fs::remove_file(&path);
}
