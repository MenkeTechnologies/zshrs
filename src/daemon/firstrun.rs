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
