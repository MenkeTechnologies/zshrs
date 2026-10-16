//! End-to-end checks for the strykelang-derived parallel builtins:
//! `puniq`, `psort`, `pfirst`, `pany`, `pmap_chunked`, `par_lines`, `par_walk`.
//! Each case runs the real binary, so it covers argument parsing, the
//! per-chunk subprocess and the fork-safe rayon pool together.

use std::io::Write;
use std::process::{Command, Stdio};

fn zshrs(script: &str, stdin: &str) -> (String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["--zsh", "-f", "-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn zshrs");
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().expect("wait");
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.code().unwrap_or(-1))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("zshrs_par_ops_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn puniq_keeps_first_occurrence_from_operands_and_stdin() {
    assert_eq!(zshrs("puniq b a b c a", "").0, "b\na\nc\n");
    assert_eq!(zshrs("puniq", "x\ny\nx\nz\n").0, "x\ny\nz\n");
}

#[test]
fn psort_flags_and_stdin() {
    assert_eq!(zshrs("psort 10 9 100", "").0, "10\n100\n9\n");
    assert_eq!(zshrs("psort -n 10 9 100", "").0, "9\n10\n100\n");
    assert_eq!(zshrs("psort -nr 10 9 100", "").0, "100\n10\n9\n");
    assert_eq!(zshrs("psort", "b\nc\na\n").0, "a\nb\nc\n");
    assert_eq!(zshrs("psort -x a; echo $?", "").0, "1\n");
}

#[test]
fn pfirst_returns_the_lowest_position_not_the_fastest() {
    // The first matching item sleeps longest; a "first to finish" implementation
    // would print 7. The lowest position (5) must win.
    let (out, rc) = zshrs("pfirst 'if [ {} = 5 ]; then sleep 0.3; fi; test {} -gt 2' 1 5 3 7", "");
    assert_eq!((out.as_str(), rc), ("5\n", 0));
    let (out, rc) = zshrs("pfirst 'test {} -gt 99' 1 2 3", "");
    assert_eq!((out.as_str(), rc), ("", 1));
}

#[test]
fn pany_status_only() {
    assert_eq!(zshrs("pany 'test {} = b' a b c; echo $?", "").0, "0\n");
    assert_eq!(zshrs("pany 'test {} = z' a b c; echo $?", "").0, "1\n");
}

#[test]
fn pmap_chunked_runs_each_chunk_in_one_process_in_item_order() {
    // $$ is the shell pid: 5 items in chunks of 2 are 3 processes, and the
    // items of one chunk share a pid.
    let (out, rc) = zshrs("pmap_chunked 2 'echo {} $$' a b c d e", "");
    assert_eq!(rc, 0);
    let rows: Vec<(String, String)> = out
        .lines()
        .map(|l| {
            let (item, pid) = l.split_once(' ').unwrap();
            (item.to_string(), pid.to_string())
        })
        .collect();
    assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["a", "b", "c", "d", "e"]);
    assert_eq!(rows[0].1, rows[1].1);
    assert_eq!(rows[2].1, rows[3].1);
    assert_ne!(rows[0].1, rows[2].1);
    assert_ne!(rows[2].1, rows[4].1);
    assert_eq!(zshrs("pmap_chunked 0 'echo {}' a", "").1, 1);
    assert_eq!(zshrs("pmap_chunked 2 'test {} != c' a b c d", "").1, 1);
}

#[test]
fn par_lines_runs_per_line_in_file_order_and_keeps_stdin_for_the_command() {
    let dir = scratch("lines");
    let file = dir.join("in.txt");
    let body: String = (0..300).map(|i| format!("line{i}\n")).collect();
    std::fs::write(&file, &body).unwrap();
    let (out, rc) = zshrs(&format!("par_lines 'echo got:{{}}' {}", file.display()), "");
    assert_eq!(rc, 0);
    let want: String = (0..300).map(|i| format!("got:line{i}\n")).collect();
    assert_eq!(out, want);
    // A command that reads stdin must not eat the remaining lines.
    let (out, _) = zshrs(&format!("par_lines 'cat >/dev/null; echo {{}}' {}", file.display()), "ignored\n");
    assert_eq!(out, body);
    // A failing command and a missing file both give status 1.
    assert_eq!(zshrs(&format!("par_lines false {}", file.display()), "").1, 1);
    assert_eq!(zshrs("par_lines 'echo {}' /nonexistent_zshrs_par_lines", "").1, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn par_walk_visits_roots_files_and_directories_in_sorted_walk_order() {
    let dir = scratch("walk");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("b.txt"), "x").unwrap();
    std::fs::write(dir.join("sub").join("a.txt"), "x").unwrap();
    let root = dir.display().to_string();
    let (out, rc) = zshrs(&format!("par_walk 'echo {{}}' {root}"), "");
    assert_eq!(rc, 0);
    let got: Vec<&str> = out.lines().map(|l| l.strip_prefix(root.as_str()).unwrap()).collect();
    assert_eq!(got, ["", "/b.txt", "/sub", "/sub/a.txt"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parallel_builtins_work_in_forked_pipeline_stages_after_the_pool_exists() {
    // The first pmap builds this process's rayon pool; the later stages run in
    // forked children, which used to block forever on the parent's dead pool.
    let script = "pmap 'echo {}' a b >/dev/null; pmap 'echo j{}' a b | cat; \
                  pmap_chunked 1 'echo k{}' a | cat; x=$(puniq a a b); echo \"[$x]\"";
    let (out, rc) = zshrs(script, "");
    assert_eq!(rc, 0);
    assert_eq!(out, "ja\njb\nka\n[a\nb]\n");
}
