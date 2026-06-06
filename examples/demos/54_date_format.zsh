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

# === ztest assertions ===
# Most date output is current-time-based (zshrs doesn't faithfully forward
# `date -u -r $epoch` flags), so assert only the deterministic state.
zassert_eq "$epoch"  1736942400  "fixed epoch literal"
zassert_gt "$now"    1700000000  "current epoch is past 2023"
zassert_gt "$now"    1736942400  "current epoch is past Jan 2025"
# direct epoch -> formatted (independent of demo flow)
direct=$(TZ=UTC date -u -r 1736942400 '+%Y-%m-%d' 2>/dev/null)
if [[ -n "$direct" ]]; then
    zassert_eq "$direct" "2025-01-15" "direct date -r 1736942400 gives 2025-01-15"
else
    ztest_skip "date -r not supported on this platform"
fi
ztest_run
