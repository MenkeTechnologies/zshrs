#!/usr/bin/env zshrs
# ASCII 7-segment digital clock — render HH:MM:SS.

# Digit map: each digit is 3-wide × 5-tall.
typeset -A DIGITS
DIGITS[0]='
 _
| |
| |
| |
|_|'
DIGITS[1]='

  |
  |
  |
  |'
DIGITS[2]='
 _
 _|
|
|
|_ '
DIGITS[3]='
 _
 _|
 _|
 _|
 _|'
DIGITS[4]='

|_|
  |
  |
  |'
DIGITS[5]='
 _
|_
 _|
 _|
 _|'
DIGITS[6]='
 _
|_
|_|
|_|
|_|'
DIGITS[7]='
 _
  |
  |
  |
  |'
DIGITS[8]='
 _
|_|
|_|
|_|
|_|'
DIGITS[9]='
 _
|_|
 _|
 _|
 _|'
DIGITS[':']='

 .

 .
   '

render_time() {
    local hhmmss=$1
    # Build 5 rows, each digit contributing 3 chars + 1 space.
    typeset -a rows
    rows=("" "" "" "" "")
    local i c digit row r
    for ((i=1; i<=${#hhmmss}; i++)); do
        c=${hhmmss[i]}
        digit=${DIGITS[$c]}
        # Skip first blank line of digit, then take 5 rows.
        # Split digit on \n.
        local IFS_save=$IFS
        IFS=$'\n'
        set -- ${=digit}
        IFS=$IFS_save
        # $1 is empty (leading \n), $2..$6 are rows.
        rows[1]+="$2"
        rows[2]+="$3"
        rows[3]+="$4"
        rows[4]+="$5"
        rows[5]+="$6"
    done
    for r in 1 2 3 4 5; do
        echo "${rows[r]}"
    done
}

echo "── time samples ──"
times=(
    "12:00:00"
    "23:59:59"
    "00:00:00"
    "09:42:15"
    "16:20:33"
    "11:11:11"
)
for t in "${times[@]}"; do
    echo
    echo "$t"
    render_time "$t"
done

echo
echo "── digit gallery (0..9) ──"
render_time "0123456789"

echo
echo "── current time (deterministic, mock 12:34:56) ──"
render_time "12:34:56"

# === ztest assertions ===
zassert_eq "${#DIGITS[@]}" "11" "11 digit cells (0..9 plus :)"
zassert_contains "${DIGITS[0]}" "| |"  "0 has vertical bars"
zassert_contains "${DIGITS[1]}" "|"    "1 has at least one bar"
zassert_contains "${DIGITS[8]}" "|_|"  "8 has rung"
zassert_contains "${DIGITS[':']}" "."  "colon uses dots"
zassert_ok "${functions[render_time]:+1}" "render_time defined"
out=$(render_time "12:00:00")
zassert_ne "$out" "" "render emits something for 12:00:00"
out2=$(render_time "12:00:00")
zassert_eq "$out" "$out2" "render is deterministic"
zassert_eq "${#times[@]}" "6" "6 time samples"
ztest_run
