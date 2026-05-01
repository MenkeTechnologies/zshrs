// zshrs-daemon — standalone daemon binary.
//
// The daemon is normally spawned-on-demand by the zshrs shell via
// `zshrs --daemon` (see docs/DAEMON.md "Daemon lifecycle"). This binary is
// the same logic packaged as its own entrypoint, useful for:
//   - systemd unit / launchd plist deployment without dragging in the
//     shell binary
//   - minimal-footprint cache servers (CI/test infra) where only the
//     daemon is needed
//   - debugging the daemon under a different process name
//
// CLI:
//   zshrs-daemon                           # run with defaults
//   zshrs-daemon --version                 # print version, exit
//   zshrs-daemon --help                    # print help, exit
//   zshrs-daemon --cache-dir <PATH>        # override XDG_CACHE_HOME (this
//                                            session only)
//   zshrs-daemon --log-level <DIRECTIVE>   # override ZSHRS_LOG (this session
//                                            only); same syntax as `zlog level`
//   zshrs-daemon --quiet-first-run         # suppress the 6-line first-run
//                                            stderr block (alias for env
//                                            ZSHRS_QUIET_FIRST_RUN=1)
//
// Personality: this binary is the daemon — there is no POSIX-mode gating
// (POSIX-mode applies to *shells*, which never spawn a daemon). Any client
// running in POSIX mode will simply not contact this daemon.

use std::env;
use std::process::ExitCode;

const HELP: &str = "\
Usage: zshrs-daemon [OPTIONS]

Run the zshrs daemon (singleton, owns ~/.cache/zshrs/).

Options:
  --cache-dir <DIR>          Override the cache root for this session
                             (sets XDG_CACHE_HOME).
  --log-level <DIRECTIVE>    Override ZSHRS_LOG for this session
                             (e.g. info | debug | info,fsnotify=trace).
  --log-stderr               Stream tracing output to stderr in addition to
                             ~/.cache/zshrs/zshrs.log. For live debugging /
                             `daemon-reset.sh`. Same as ZSHRS_LOG_STDERR=1.
  --verbose-init             Per docs/DAEMON.md:899: show daemon work to stderr
                             on every run (not just first). Implies
                             --log-stderr and ZSHRS_LOG=debug. For testing.
  --quiet-first-run          Suppress the 6-line first-run stderr block.
  --version                  Print version, exit.
  -h, --help                 Print this help, exit.

The daemon exits cleanly on SIGTERM, SIGINT, or `zcache daemon stop`.

For client-side commands (zcache, zls, zping, zsubscribe, zjob, zsync,
zask, zhistory, zlog, etc.), use the zshrs shell binary.
";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    let mut iter = args.iter().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--version" => {
                println!("zshrs-daemon {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                print!("{}", HELP);
                return ExitCode::SUCCESS;
            }
            "--cache-dir" => match iter.next() {
                Some(d) => env::set_var("XDG_CACHE_HOME", d),
                None => {
                    eprintln!("zshrs-daemon: --cache-dir requires a path");
                    return ExitCode::from(2);
                }
            },
            "--log-level" => match iter.next() {
                Some(d) => env::set_var("ZSHRS_LOG", d),
                None => {
                    eprintln!("zshrs-daemon: --log-level requires a directive");
                    return ExitCode::from(2);
                }
            },
            "--quiet-first-run" => env::set_var("ZSHRS_QUIET_FIRST_RUN", "1"),
            "--log-stderr" => env::set_var("ZSHRS_LOG_STDERR", "1"),
            "--verbose-init" => {
                // Implies --log-stderr + ZSHRS_LOG=debug. Per docs/DAEMON.md:899
                // intended for testing/diagnosis on every run, not just first.
                env::set_var("ZSHRS_LOG_STDERR", "1");
                if env::var_os("ZSHRS_LOG").is_none() {
                    env::set_var("ZSHRS_LOG", "debug");
                }
                env::set_var("ZSHRS_VERBOSE_INIT", "1");
            }
            other => {
                eprintln!("zshrs-daemon: unknown argument `{}` (try --help)", other);
                return ExitCode::from(2);
            }
        }
    }

    match zshrs_daemon::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(zshrs_daemon::DaemonError::AlreadyRunning(pid)) => {
            // Per docs/DAEMON.md "Singleton enforcement". Exit cleanly so a
            // double-launch in a unit file doesn't error-loop the supervisor.
            eprintln!("zshrs-daemon: another daemon is running (pid {})", pid);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("zshrs-daemon: {}", e);
            ExitCode::FAILURE
        }
    }
}
