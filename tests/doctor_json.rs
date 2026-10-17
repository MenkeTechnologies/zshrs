//! `doctor --json` from inside a running shell prints the doctor report as one JSON object.

use std::process::Command;

fn run_doctor(args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(args)
        .output()
        .expect("spawn zshrs");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("stdout is one JSON object")
}

#[test]
fn builtin_json_has_the_flag_report_sections_without_a_benchmark() {
    let builtin = run_doctor(&["-f", "-c", "doctor --json"]);
    let flag = run_doctor(&["--doctor", "--json"]);
    let keys = |v: &serde_json::Value| -> Vec<String> {
        v.as_object().unwrap().keys().cloned().collect()
    };
    assert_eq!(keys(&builtin), keys(&flag));
    assert!(builtin["startup_benchmark"].is_null());
    assert!(flag["startup_benchmark"]["total_ms"].is_u64());
    assert_eq!(builtin["environment"]["version"], flag["environment"]["version"]);
}
