//! A corrupt `plugins.db` must heal itself, not stay dead forever.
//!
//! The cache is derived data: it is rebuilt from the plugins the shell
//! sources. Before the fix this test pins, SQLite's `SQLITE_NOTADB`
//! ("file is not a database") was logged once per startup and the cache was
//! then skipped — permanently, for every future shell, because nothing ever
//! removed the bad file. A real `~/.zshrs/plugins.db` sat corrupt for three
//! months that way, its header's leading `SQL` overwritten, with only a log
//! line to say why plugin lookups were slow.

use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("target/debug/zshrs");
    p
}

/// Write a file that is emphatically not a database, in the exact shape the
/// real corruption took: a valid-looking tail with the magic's first bytes
/// clobbered, so SQLite reports NOTADB rather than a short read.
fn write_corrupt_db(path: &PathBuf) {
    let mut bytes = b"hi\n".to_vec();
    bytes.extend_from_slice(b"ite format 3\0");
    bytes.extend_from_slice(&[0u8; 512]);
    std::fs::write(path, bytes).expect("write corrupt db");
}

fn is_sqlite(path: &PathBuf) -> bool {
    match std::fs::read(path) {
        Ok(b) => b.starts_with(b"SQLite format 3\0"),
        Err(_) => false,
    }
}

#[test]
fn corrupt_plugin_cache_is_discarded_and_rebuilt() {
    let bin = zshrs_bin();
    if !bin.exists() {
        eprintln!("skip: {} not built", bin.display());
        return;
    }
    let home = std::env::temp_dir().join(format!("zshrs-pc-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("create ZSHRS_HOME");
    let db = home.join("plugins.db");
    write_corrupt_db(&db);
    assert!(!is_sqlite(&db), "fixture must start corrupt");

    let out = Command::new(&bin)
        .args(["-f", "-c", "print ok"])
        .env("ZSHRS_HOME", &home)
        .output()
        .expect("run zshrs");

    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ok"),
        "shell must still run with a corrupt cache: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        is_sqlite(&db),
        "corrupt plugins.db should have been discarded and rebuilt, header is still {:?}",
        std::fs::read(&db).map(|b| b[..16.min(b.len())].to_vec())
    );

    // Second start on the healed file must be quiet: no discard, no rebuild.
    let out2 = Command::new(&bin)
        .args(["-f", "-c", "print ok2"])
        .env("ZSHRS_HOME", &home)
        .output()
        .expect("run zshrs again");
    assert!(String::from_utf8_lossy(&out2.stdout).contains("ok2"));
    assert!(is_sqlite(&db), "healed cache must survive the next start");

    let _ = std::fs::remove_dir_all(&home);
}

/// A cache that is merely unreadable (no permissions) is NOT corruption:
/// deleting it would be destructive and wrong, so the shell degrades instead.
#[cfg(unix)]
#[test]
fn unreadable_plugin_cache_is_left_alone() {
    use std::os::unix::fs::PermissionsExt;

    let bin = zshrs_bin();
    if !bin.exists() {
        eprintln!("skip: {} not built", bin.display());
        return;
    }
    // root ignores the permission bits, so this would not test anything.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skip: running as root");
        return;
    }
    let home = std::env::temp_dir().join(format!("zshrs-pc-perm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("create ZSHRS_HOME");
    let db = home.join("plugins.db");
    std::fs::write(&db, b"SQLite format 3\0").expect("seed");
    let mut perms = std::fs::metadata(&db).unwrap().permissions();
    perms.set_mode(0o000);
    std::fs::set_permissions(&db, perms).expect("chmod 000");

    let out = Command::new(&bin)
        .args(["-f", "-c", "print ok"])
        .env("ZSHRS_HOME", &home)
        .output()
        .expect("run zshrs");
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok"));
    assert!(db.exists(), "an unreadable cache must not be deleted");

    let mut perms = std::fs::metadata(&db).unwrap().permissions();
    perms.set_mode(0o600);
    let _ = std::fs::set_permissions(&db, perms);
    let _ = std::fs::remove_dir_all(&home);
}
