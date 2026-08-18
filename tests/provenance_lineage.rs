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
        out.contains(r#"origin: cmdsubst "echo hello world" (line 2)"#),
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
    assert!(out.starts_with(r#"{"name":"F","origin":"cmdsubst"#), "{out}");
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
