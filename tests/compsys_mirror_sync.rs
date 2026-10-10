//! compsys.db's `comps` table is a mirror of the shell's `_comps`. A scan
//! (`compinit` without `-C`) fills it; `compinit -C` loads the dump instead and
//! used to leave the mirror at whatever the last scan wrote, so `dbview` and SQL
//! readers saw a fraction of the table the shell was running with.

use std::path::Path;
use std::process::Command;

fn zshrs(home: &Path, script: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["-f", "-c", script])
        .env("ZSHRS_HOME", home)
        .env("HOME", home)
        .output()
        .expect("spawn zshrs");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn mirror_rows(home: &Path) -> i64 {
    let conn = rusqlite::Connection::open_with_flags(
        home.join("compsys.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open compsys.db");
    conn.query_row("SELECT COUNT(*) FROM comps", [], |r| r.get(0))
        .expect("count comps")
}

#[test]
fn dump_load_brings_the_mirror_up_to_the_running_comps() {
    let dir = std::env::temp_dir().join(format!("zshrs-mirror-sync-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let fdir = dir.join("fn");
    std::fs::create_dir_all(&fdir).unwrap();
    for name in ["alpha", "beta", "gamma", "delta", "epsilon"] {
        std::fs::write(fdir.join(format!("_{name}")), format!("#compdef {name}\n_files\n")).unwrap();
    }
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let dump = dir.join("dump");

    // 1. a scan writes the dump and fills the mirror
    let scan = zshrs(
        &home,
        &format!(
            "fpath=({}); autoload -Uz compinit; compinit -u -d {}; print -r -- n=${{#_comps}}; sleep 1",
            fdir.display(),
            dump.display()
        ),
    );
    let n: i64 = scan.trim().strip_prefix("n=").and_then(|s| s.parse().ok()).expect("scan count");
    assert!(n >= 5, "scan registered {n} completers");
    assert_eq!(mirror_rows(&home), n, "a scan mirrors every _comps entry");

    // 2. the mirror goes stale (a partial earlier scan, a wiped table)
    let conn = rusqlite::Connection::open(home.join("compsys.db")).unwrap();
    conn.execute("DELETE FROM comps", []).unwrap();
    conn.execute("DELETE FROM fts_comps", []).unwrap();
    drop(conn);
    assert_eq!(mirror_rows(&home), 0);

    // 3. `-C` loads the dump; the mirror must follow what the shell now runs with
    let loaded = zshrs(
        &home,
        &format!(
            "fpath=({}); autoload -Uz compinit; compinit -C -u -d {}; print -r -- n=${{#_comps}}; sleep 2",
            fdir.display(),
            dump.display()
        ),
    );
    let m: i64 = loaded.trim().strip_prefix("n=").and_then(|s| s.parse().ok()).expect("load count");
    assert_eq!(m, n, "the dump restores the same table");
    assert_eq!(mirror_rows(&home), m, "dump load re-fills the mirror");
    let _ = std::fs::remove_dir_all(&dir);
}
