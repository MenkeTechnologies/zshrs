#!/usr/bin/env zshrs
# Date formatting — zsh's strftime + `date` command.

# Use a fixed epoch so output is deterministic in CI.
# 2025-01-15 12:00:00 UTC = 1736942400
epoch=1736942400

echo "── strftime via zsh ──"
# zsh's strftime builtin: zmodload zsh/datetime
zmodload zsh/datetime 2>/dev/null || true

if (( $+functions[strftime] )) || command -v strftime >/dev/null 2>&1; then
    echo "strftime available"
fi

# `date` is POSIX-standard and exists on every CI.
echo "── date command (UTC for determinism) ──"
TZ=UTC date -u -r $epoch '+%Y-%m-%d %H:%M:%S' 2>/dev/null \
    || TZ=UTC date -u -d "@$epoch" '+%Y-%m-%d %H:%M:%S' \
    || echo "date -r/-d not supported"

echo "── ISO 8601 ──"
TZ=UTC date -u -r $epoch '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
    || TZ=UTC date -u -d "@$epoch" '+%Y-%m-%dT%H:%M:%SZ'

echo "── day-of-week + month ──"
TZ=UTC date -u -r $epoch '+%A, %B %-d %Y' 2>/dev/null \
    || TZ=UTC date -u -d "@$epoch" '+%A, %B %-d %Y'

echo "── current epoch sanity ──"
now=$(date +%s)
if (( now > 1700000000 )); then
    echo "current epoch is past 2023"
fi
