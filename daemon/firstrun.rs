// First-run user notification — the one-time exception to the no-banner rule.
//
// Per docs/DAEMON.md "First-run user notification (the one-time exception to no-banner)":
//   First-ever zshrs invocation prints a 6-line block to stderr describing what the daemon
//   is doing and where to look for details. After the prompt appears, the daemon continues
//   building in the background. Subsequent runs are silent.
//
// Detection: no daemon.pid AND no index.rkyv AND no shards in images/.

use std::io::Write;

use super::paths::CachePaths;

/// Print the first-run notification block to stderr if applicable.
///
/// Returns true if the notice was printed (i.e. this *is* a first run), false otherwise.
/// Skipped when ZSHRS_QUIET_FIRST_RUN=1 or `--quiet-first-run`.
pub fn maybe_print(paths: &CachePaths) -> bool {
    if !paths.is_first_run() {
        return false;
    }
    if std::env::var_os("ZSHRS_QUIET_FIRST_RUN").map_or(false, |v| v == "1" || v == "true") {
        return false;
    }

    // Per DAEMON.md:883 — emit an "estimated:" line so the user has a
    // sense of how long this first-run will take. Numbers come from a
    // cheap pre-walk: count $PATH dirs that exist, count $FPATH dirs that
    // exist, sum file counts (capped per dir to keep the pre-walk fast).
    let est = compute_first_run_estimate();

    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(
        stderr,
        "zshrs first-run init — daemon spawning, cold cache building."
    );
    let _ = writeln!(
        stderr,
        "  scope:      ~/.zshrc + transitive sources + $PATH + $FPATH + plugins"
    );
    let _ = writeln!(
        stderr,
        "  estimated:  {} files, ~{} LOC, ~{}s on this machine",
        est.files, est.loc_human, est.duration_s,
    );
    let _ = writeln!(
        stderr,
        "  background: shells work via source-interp until cache is warm"
    );
    let _ = writeln!(stderr, "  log:        {}", paths.log.display());
    let _ = writeln!(
        stderr,
        "  inspect:    zcache info | zcache jobs | zcache view <target>"
    );
    let _ = writeln!(
        stderr,
        "  reset:      zcache clean | rm -rf {}",
        paths.root.display()
    );
    let _ = stderr.flush();
    true
}

#[derive(Debug)]
struct FirstRunEstimate {
    files: usize,
    loc_human: String, // e.g. "1.6M" or "120K" or "8.3K"
    duration_s: u64,
}

/// Quick pre-walk to get rough file/LOC counts for the first-run "estimated"
/// line. Capped to avoid blocking the prompt: max 200 dirs, max 1024 files
/// per dir. Numbers are approximate by design.
fn compute_first_run_estimate() -> FirstRunEstimate {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("PATH") {
        dirs.extend(p.split(':').map(std::path::PathBuf::from));
    }
    if let Ok(p) = std::env::var("FPATH") {
        dirs.extend(p.split(':').map(std::path::PathBuf::from));
    }
    if let Some(home) = dirs::home_dir() {
        for sub in &[
            ".zshrc",
            ".zpwr",
            ".oh-my-zsh",
            ".zinit/plugins",
            ".zinit/snippets",
        ] {
            let p = home.join(sub);
            if p.exists() {
                dirs.push(p);
            }
        }
    }

    let mut files = 0usize;
    let mut loc = 0u64;
    let dir_cap = 200;
    for dir in dirs.iter().take(dir_cap) {
        let mut per_dir_files = 0usize;
        if let Ok(rd) = std::fs::read_dir(dir) {
            for ent in rd.flatten() {
                if per_dir_files >= 1024 {
                    break;
                }
                if let Ok(meta) = ent.metadata() {
                    if meta.is_file() {
                        files += 1;
                        per_dir_files += 1;
                        // Estimate LOC as bytes / 50 (avg shell line length);
                        // cheaper than reading the file.
                        loc += meta.len() / 50;
                    }
                }
            }
        }
    }

    let loc_human = if loc > 1_000_000 {
        format!("{:.1}M", loc as f64 / 1_000_000.0)
    } else if loc > 1_000 {
        format!("{:.0}K", loc as f64 / 1_000.0)
    } else {
        loc.to_string()
    };
    // Rough rate: 30k LOC/sec on a modern SSD daemon walk. Floor at 5s.
    let duration_s = ((loc / 30_000) as u64).max(5);

    FirstRunEstimate {
        files,
        loc_human,
        duration_s,
    }
}
