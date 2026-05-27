#!/bin/sh
# Generate corpus_dash_fc_bulk_* modules for a round range.
# Usage: START_ROUND=101 END_ROUND=200 TESTS_PER_ROUND=40 \
#   ./scripts/gen_parity_bulk_rounds.sh [pool] [out.rs]

set -e
Z=/opt/homebrew/bin/zsh
R="${ZSHRS_BIN:-/Users/wizard/RustroverProjects/zshrs/target/debug/zshrs}"
POOL="${1:-$(dirname "$0")/parity_round_script_pool.txt}"
OUT="${2:-/tmp/parity_bulk_generated.rs}"
MANIFEST_DIR="${CARGO_MANIFEST_DIR:-/Users/wizard/RustroverProjects/zshrs}"
START_ROUND=${START_ROUND:-11}
END_ROUND=${END_ROUND:-100}
TESTS_PER_ROUND=${TESTS_PER_ROUND:-40}

if [ ! -x "$R" ]; then
  (cd "$MANIFEST_DIR" && cargo build --bin zshrs) >/dev/null 2>&1
  R="$MANIFEST_DIR/target/debug/zshrs"
fi

# round 11 -> av; round 101 -> eh; round 200 -> ic
round_suffix() {
  round=$1
  idx=$((round - 11))
  if [ "$idx" -lt 5 ]; then
    printf 'a%c' "$(printf '\\%03o' $((118 + idx)))"
  else
    pair=$((idx - 5))
    g=$((pair / 26))
    s=$((pair % 26))
    printf '%c%c' "$(printf '\\%03o' $((98 + g)))" "$(printf '\\%03o' $((97 + s)))"
  fi
}

probe_one() {
  script=$1
  zo=$($Z -fc "$script" 2>/dev/null; printf '|%s' "$?")
  ro=$($R --zsh -fc "$script" 2>/dev/null; printf '|%s' "$?")
  [ "$zo" = "$ro" ]
}

nlines=$(wc -l < "$POOL" | tr -d ' ')
: > "$OUT"
total_pass=0
total_fail=0

round=$START_ROUND
while [ "$round" -le "$END_ROUND" ]; do
  sfx=$(round_suffix "$round")
  offset=$(((round * 17 + 7) % nlines))
  pass=0
  fail=0
  idx=0

  {
    echo ""
    echo "mod corpus_dash_fc_bulk_${sfx} {"
    echo "    use super::*;"
    echo ""
    echo "    parity_gap_tests! {"
  } >> "$OUT"

  while [ "$pass" -lt "$TESTS_PER_ROUND" ] && [ "$idx" -lt "$nlines" ]; do
    line_num=$(((offset + idx) % nlines + 1))
    script=$(sed -n "${line_num}p" "$POOL")
    idx=$((idx + 1))
    [ -z "$script" ] && continue
    if probe_one "$script"; then
      pass=$((pass + 1))
      id=$(printf '%03d' "$pass")
      if echo "$script" | grep -q '##'; then
        delim='###'
      else
        delim='##'
      fi
      printf '        bulk_%s_fc_row_%s => (r#"bulk %s %s"#, r#%s"%s"%s#);\n' \
        "$sfx" "$id" "$sfx" "$id" "$delim" "$script" "$delim" >> "$OUT"
    else
      fail=$((fail + 1))
    fi
  done

  echo "    }" >> "$OUT"
  echo "}" >> "$OUT"
  total_pass=$((total_pass + pass))
  total_fail=$((total_fail + fail))
  echo "round $round ($sfx): pass=$pass fail=$fail" >&2
  round=$((round + 1))
done

echo "TOTAL pass=$total_pass fail=$total_fail rounds=$START_ROUND-$END_ROUND -> $OUT" >&2
