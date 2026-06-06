#!/usr/bin/env zshrs
# zsh/datetime module — EPOCHSECONDS, EPOCHREALTIME, strftime builtin.
# Ported from zsh's Src/Modules/datetime.c.

zmodload zsh/datetime 2>/dev/null || true

echo "── EPOCHSECONDS — int seconds ──"
now=$EPOCHSECONDS
echo "now epoch sec: $now"
# Sanity bound: post-2020 epoch.
(( now > 1577836800 )) && echo "sanity: past 2020 (OK)"
echo "(int type) $((now + 0))"

echo "── EPOCHREALTIME — fractional sec ──"
real=$EPOCHREALTIME
echo "EPOCHREALTIME=$real"
# It's "sec.fraction" — integer part should match epochseconds.
[[ ${real%%.*} == $now ]] && echo "integer part matches EPOCHSECONDS"

echo "── strftime: known fixed epoch ──"
# 2025-01-15 12:00:00 UTC = 1736942400
fixed=1736942400
TZ=UTC strftime "%Y-%m-%d %H:%M:%S" $fixed
TZ=UTC strftime "%A, %B %-d, %Y" $fixed

echo "── strftime: full ISO 8601 ──"
TZ=UTC strftime "%Y-%m-%dT%H:%M:%SZ" $fixed

echo "── strftime: day-of-year, week, etc ──"
TZ=UTC strftime "DOY=%j Week=%V" $fixed

echo "── timing wrapper using EPOCHREALTIME ──"
time_block() {
    local label=$1
    shift
    local start=$EPOCHREALTIME
    "$@"
    local end=$EPOCHREALTIME
    printf "[%s] %.6fs\n" "$label" $((end - start))
}

# Something measurable but cheap.
time_block "loop-1000" \
    bash -c 'for ((i=0;i<1000;i++)); do :; done'

echo "── parse a static date back (TZ-bound) ──"
# strftime can parse with -r flag in some zsh versions; fall back to TZ=UTC.
TZ=UTC strftime "Decode %F %T from epoch=$fixed" $fixed

# === ztest assertions ===
# EPOCHSECONDS / EPOCHREALTIME / timing are non-deterministic — assert bounds.
zassert_gt "$EPOCHSECONDS"        1577836800  "EPOCHSECONDS > 2020-01-01"
zassert_lt "$EPOCHSECONDS"        4102444800  "EPOCHSECONDS < 2100-01-01"
real=$EPOCHREALTIME
zassert_match '^[0-9]+\.[0-9]+$' "$real"      "EPOCHREALTIME is sec.fraction"
zassert_eq "${real%%.*}"          "$EPOCHSECONDS" "real integer part == EPOCHSECONDS"
fixed=1736942400
zassert_eq "$(TZ=UTC strftime '%Y-%m-%d %H:%M:%S' $fixed)" "2025-01-15 12:00:00" "strftime fixed epoch"
zassert_eq "$(TZ=UTC strftime '%Y-%m-%dT%H:%M:%SZ' $fixed)" "2025-01-15T12:00:00Z" "ISO 8601"
zassert_eq "$(TZ=UTC strftime '%A, %B %-d, %Y' $fixed)"   "Wednesday, January 15, 2025" "day name"
zassert_eq "$(TZ=UTC strftime 'DOY=%j Week=%V' $fixed)"   "DOY=015 Week=03" "DOY+ISO week"
ztest_run

