#!/usr/bin/env bash
# install-launchd.sh — install com.menketechnologies.zshrs-daemon.plist
# into ~/Library/LaunchAgents with the current user's $HOME templated
# in. Idempotent — safe to re-run after edits.
#
# Usage:
#   ./examples/install-launchd.sh                   # use ~/.cargo/bin/zshrs-daemon
#   ZSHRS_DAEMON_BIN=/opt/homebrew/bin/zshrs-daemon \
#     ./examples/install-launchd.sh
#
# Verify:
#   launchctl list | grep zshrs
#   tail -f ~/.zshrs/zshrs.log

set -euo pipefail

[[ "$(uname -s)" == "Darwin" ]] || {
    echo "install-launchd.sh: macOS only (use examples/systemd/ on Linux)" >&2
    exit 1
}

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/launchd/com.menketechnologies.zshrs-daemon.plist"
DST="$HOME/Library/LaunchAgents/com.menketechnologies.zshrs-daemon.plist"
BIN="${ZSHRS_DAEMON_BIN:-$HOME/.cargo/bin/zshrs-daemon}"

[[ -f "$SRC" ]] || { echo "missing source plist: $SRC" >&2; exit 1; }
[[ -x "$BIN" ]] || { echo "zshrs-daemon binary not found / not executable: $BIN" >&2; exit 1; }

mkdir -p "$HOME/Library/LaunchAgents" "$HOME/.zshrs"

# Template the placeholders. Use a temp file then atomic-rename so a
# failed sed never leaves a half-written plist that launchctl might
# try to load.
TMP="$(mktemp -t zshrs-daemon.plist.XXXXXX)"
trap 'command rm -f "$TMP"' EXIT

# `sed -e 's|<placeholder>|<value>|g'` with `|` as separator since
# $HOME contains `/`.
sed \
    -e "s|/Users/__USER__/.cargo/bin/zshrs-daemon|$BIN|g" \
    -e "s|/Users/__USER__|$HOME|g" \
    "$SRC" > "$TMP"

# Unload existing if present so launchctl picks up the new plist
# cleanly. Ignore errors — first install has nothing to unload.
launchctl unload "$DST" 2>/dev/null || true

mv -f "$TMP" "$DST"
trap - EXIT

launchctl load -w "$DST"

printf 'installed: %s\n' "$DST"
printf 'binary:    %s\n' "$BIN"
printf 'verify:    launchctl list | grep zshrs\n'
printf 'logs:      tail -f $HOME/.zshrs/zshrs.log\n'
