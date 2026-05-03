#!/usr/bin/env bash
#
# daemon-reset.sh — full reset + rebuild + log-follow loop, for testing.
#
# Per docs/DAEMON.md "Daemon lifecycle" + "Walk lifecycle — first init":
#
#   1. Stop any running daemon (singleton flock + graceful shutdown).
#   2. Wipe ~/.zshrs/ entirely (nuclear, but explicit).
#   3. Spawn a fresh daemon in the foreground with TRACE-level logging
#      streaming to stderr.
#   4. Wait for the singleton socket to come up.
#   5. Trigger first_init on the user's $ZDOTDIR/.zshrc (or the path the
#      caller passed).
#   6. Stay attached so every IPC op, fsnotify event, walk pass, shard
#      atomic-rename, and canonical promotion is visible live.
#   7. On Ctrl-C: SIGTERM the daemon and exit cleanly.
#
# Usage:
#   daemon/scripts/daemon-reset.sh                       # default ~/.zshrc
#   daemon/scripts/daemon-reset.sh ~/.zshrc.test         # custom path
#   ZSHRS_LOG=info daemon/scripts/daemon-reset.sh        # quieter
#   ZSHRS_LOG=trace daemon/scripts/daemon-reset.sh       # firehose
#   daemon/scripts/daemon-reset.sh --no-init             # skip first_init
#   daemon/scripts/daemon-reset.sh --keep-cache          # don't wipe
#
# Output: every daemon log line (ANSI-coloured) to your terminal until you
# Ctrl-C. The actual log file at ~/.zshrs/zshrs.log is also written.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DAEMON_BIN="${REPO_ROOT}/target/debug/zshrs-daemon"
SHELL_BIN="${REPO_ROOT}/target/debug/zshrs"

# Daemon resolves $ZSHRS_HOME, falling back to ~/.zshrs. Single
# top-level dir holding everything; per docs/DAEMON.md.
CACHE_ROOT="${ZSHRS_HOME:-$HOME/.zshrs}"
LOG_LEVEL="${ZSHRS_LOG:-debug}"

ZSHRC="$HOME/.zshrc"
DO_INIT=1
DO_WIPE=1
KEEP_RUNNING=1

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-init)     DO_INIT=0 ;;
        --keep-cache)  DO_WIPE=0 ;;
        --once)        KEEP_RUNNING=0 ;;
        -h|--help)
            head -30 "$0" | sed -e '1d' -e 's/^# \?//'
            exit 0
            ;;
        --) shift; break ;;
        --*)
            echo "daemon-reset: unknown flag $1" >&2
            exit 2
            ;;
        *)
            ZSHRC="$1"
            ;;
    esac
    shift
done

# ---- Build ----
# Always rebuild to make sure we're testing the current source, not a stale
# binary. Dev profile only per CLAUDE.md (never --release for local dev).
echo "==> Building daemon + shell..." >&2
cd "$REPO_ROOT"
cargo build -p zshrs-daemon --bin zshrs-daemon
cargo build -p zshrs --bin zshrs

# ---- 1. Stop any running daemon ----
if [[ -S "$CACHE_ROOT/daemon.sock" ]]; then
    echo "==> Stopping existing daemon..." >&2
    "$SHELL_BIN" -c "zcache daemon stop" 2>/dev/null || true
    sleep 0.2
fi
# Belt + suspenders: any leftover process gets SIGTERM directly.
if pgrep -f zshrs-daemon >/dev/null 2>&1; then
    pkill -TERM -f zshrs-daemon 2>/dev/null || true
    sleep 0.3
fi

# ---- 2. Wipe cache ----
if (( DO_WIPE )); then
    echo "==> Wiping $CACHE_ROOT..." >&2
    command rm -rf "$CACHE_ROOT"
fi

# ---- 3. Spawn daemon foreground with stderr logs ----
echo "==> Spawning daemon (ZSHRS_LOG=$LOG_LEVEL, --log-stderr)..." >&2
ZSHRS_LOG="$LOG_LEVEL" \
ZSHRS_LOG_STDERR=1 \
    "$DAEMON_BIN" --log-stderr &
DAEMON_PID=$!

cleanup() {
    if kill -0 "$DAEMON_PID" 2>/dev/null; then
        echo >&2
        echo "==> Stopping daemon (pid $DAEMON_PID)..." >&2
        kill -TERM "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

# ---- 4. Wait for socket ----
for _ in $(seq 1 100); do
    [[ -S "$CACHE_ROOT/daemon.sock" ]] && break
    sleep 0.05
done
if [[ ! -S "$CACHE_ROOT/daemon.sock" ]]; then
    echo "daemon-reset: socket never appeared at $CACHE_ROOT/daemon.sock" >&2
    exit 1
fi
echo "==> Daemon up (pid $DAEMON_PID, socket at $CACHE_ROOT/daemon.sock)" >&2

# ---- 5. Trigger first_init ----
if (( DO_INIT )); then
    if [[ -f "$ZSHRC" ]]; then
        echo "==> Triggering first_init for $ZSHRC..." >&2
        "$SHELL_BIN" -c "zcache first-init --zshrc '$ZSHRC'" || {
            echo "daemon-reset: first_init failed (daemon still running)" >&2
        }
    else
        echo "==> Skipping first_init: $ZSHRC not found" >&2
    fi
fi

# ---- 6. Stay attached ----
if (( KEEP_RUNNING )); then
    echo "==> Daemon running. Ctrl-C to stop." >&2
    echo "==> Other useful commands while attached:" >&2
    echo "      $SHELL_BIN -c 'zcache info'" >&2
    echo "      $SHELL_BIN -c 'zcache verify'" >&2
    echo "      $SHELL_BIN -c 'zcache view path'" >&2
    echo "      $SHELL_BIN -c 'zcache rebuild'" >&2
    echo "      $SHELL_BIN -c 'zcache doctor' 2>/dev/null || true" >&2
    echo "      tail -f $CACHE_ROOT/zshrs.log" >&2
    wait "$DAEMON_PID"
else
    echo "==> --once mode: stopping daemon now" >&2
    cleanup
fi
