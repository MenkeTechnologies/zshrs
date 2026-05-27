#!/bin/sh
# Generate corpus_dash_fc_bulk_{av..eg} for rounds 11-100.
# Probes zsh vs zshrs; emits only passing scripts. No backslash-escaped quotes in r## strings.

set -e
Z=/opt/homebrew/bin/zsh
R="${ZSHRS_BIN:-/Users/wizard/RustroverProjects/zshrs/target/debug/zshrs}"
POOL="${1:-$(dirname "$0")/parity_round_script_pool.txt}"
OUT="${2:-/tmp/parity_bulk_11_100.rs}"
MANIFEST_DIR="${CARGO_MANIFEST_DIR:-/Users/wizard/RustroverProjects/zshrs}"

if [ ! -x "$R" ]; then
  (cd "$MANIFEST_DIR" && cargo build --bin zshrs) >/dev/null 2>&1
  R="$MANIFEST_DIR/target/debug/zshrs"
fi

round_suffix() {
  n=$1
  case $n in
    11) echo av ;; 12) echo aw ;; 13) echo ax ;; 14) echo ay ;; 15) echo az ;;
    16) echo ba ;; 17) echo bb ;; 18) echo bc ;; 19) echo bd ;; 20) echo be ;;
    21) echo bf ;; 22) echo bg ;; 23) echo bh ;; 24) echo bi ;; 25) echo bj ;;
    26) echo bk ;; 27) echo bl ;; 28) echo bm ;; 29) echo bn ;; 30) echo bo ;;
    31) echo bp ;; 32) echo bq ;; 33) echo br ;; 34) echo bs ;; 35) echo bt ;;
    36) echo bu ;; 37) echo bv ;; 38) echo bw ;; 39) echo bx ;; 40) echo by ;;
    41) echo bz ;;
    42) echo ca ;; 43) echo cb ;; 44) echo cc ;; 45) echo cd ;; 46) echo ce ;;
    47) echo cf ;; 48) echo cg ;; 49) echo ch ;; 50) echo ci ;; 51) echo cj ;;
    52) echo ck ;; 53) echo cl ;; 54) echo cm ;; 55) echo cn ;; 56) echo co ;;
    57) echo cp ;; 58) echo cq ;; 59) echo cr ;; 60) echo cs ;; 61) echo ct ;;
    62) echo cu ;; 63) echo cv ;; 64) echo cw ;; 65) echo cx ;; 66) echo cy ;;
    67) echo cz ;;
    68) echo da ;; 69) echo db ;; 70) echo dc ;; 71) echo dd ;; 72) echo de ;;
    73) echo df ;; 74) echo dg ;; 75) echo dh ;; 76) echo di ;; 77) echo dj ;;
    78) echo dk ;; 79) echo dl ;; 80) echo dm ;; 81) echo dn ;; 82) echo do ;;
    83) echo dp ;; 84) echo dq ;; 85) echo dr ;; 86) echo ds ;; 87) echo dt ;;
    88) echo du ;; 89) echo dv ;; 90) echo dw ;; 91) echo dx ;; 92) echo dy ;;
    93) echo dz ;;
    94) echo ea ;; 95) echo eb ;; 96) echo ec ;; 97) echo ed ;; 98) echo ee ;;
    99) echo ef ;; 100) echo eg ;;
    *) echo "xx" ;;
  esac
}

pool_lines() { wc -l < "$POOL" | tr -d ' '; }

probe_one() {
  script=$1
  zo=$($Z -fc "$script" 2>/dev/null; printf '|%s' "$?")
  ro=$($R --zsh -fc "$script" 2>/dev/null; printf '|%s' "$?")
  [ "$zo" = "$ro" ]
}

: > "$OUT"
total_pass=0
total_fail=0
TESTS_PER_ROUND=${TESTS_PER_ROUND:-48}

nlines=$(pool_lines)

round=11
while [ "$round" -le 100 ]; do
  sfx=$(round_suffix "$round")
  offset=$(((round - 11) * 13 % nlines))
  pass=0
  fail=0
  tried=0
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
    tried=$((tried + 1))
    [ -z "$script" ] && continue
    if probe_one "$script"; then
      pass=$((pass + 1))
      id=$(printf '%03d' "$pass")
      # Use r### delimiters if script contains ##
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
  echo "round $round ($sfx): pass=$pass fail=$fail tried=$tried" >&2
  round=$((round + 1))
done

echo "TOTAL pass=$total_pass fail=$total_fail -> $OUT" >&2
