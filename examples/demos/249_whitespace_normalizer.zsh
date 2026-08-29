#!/usr/bin/env zshrs
# Whitespace normalizer — strip, collapse, expand tabs, etc.

# `[[:space:]]##` is an EXTENDED_GLOB closure — without this setopt the `##`
# is matched literally and strip/collapse return their input unchanged.
setopt extended_glob

# Strip leading + trailing whitespace.
strip() {
    local s=$1
    s="${s##[[:space:]]##}"
    s="${s%%[[:space:]]##}"
    echo "$s"
}

# Collapse internal runs of whitespace to single space.
collapse() {
    local s=$1
    echo "${s//[[:space:]]##/ }"
}

# Expand tabs to N spaces.
expand_tabs() {
    local s=$1 n=${2:-4} sp=""
    local i
    for ((i=0; i<n; i++)); do sp+=" "; done
    echo "${s//$'\t'/$sp}"
}

# Unexpand — collapse runs of 4 spaces to tab.
# NB: `$'...'` is not expanded inside a ${...//pat/repl} pattern or
# replacement — it stays literal there. Build the escapes into variables first.
unexpand_tabs() {
    local s=$1 n=${2:-4} sp=""
    local i tab=$'\t'
    for ((i=0; i<n; i++)); do sp+=" "; done
    echo "${s//$sp/$tab}"
}

# Normalize line endings.
normalize_eol() {
    local s=$1
    local cr=$'\r' lf=$'\n' crlf=$'\r\n'
    s="${s//$crlf/$lf}"
    s="${s//$cr/$lf}"
    echo "$s"
}

# Squash blank lines.
squash_blanks() {
    local s=$1 line out=""
    local prev_blank=0
    while IFS= read -r line; do
        if [[ -z $line ]]; then
            if (( prev_blank == 0 )); then
                out+="$line"$'\n'
                prev_blank=1
            fi
        else
            out+="$line"$'\n'
            prev_blank=0
        fi
    done <<< "$s"
    echo "$out"
}

echo "── strip leading/trailing ──"
samples=(
    "  hello  "
    $'\t\thello\t\t'
    "no_ws"
    "    "
    $'\n  text  \n'
)
for s in "${samples[@]}"; do
    printf "  [%s] → [%s]\n" "$s" "$(strip "$s")"
done

echo
echo "── collapse runs ──"
collapse_samples=(
    "one  two   three"
    $'one\t\ttwo\t\tthree'
    "  too   many    spaces"
)
for s in "${collapse_samples[@]}"; do
    printf "  [%s] → [%s]\n" "$s" "$(collapse "$s")"
done

echo
echo "── expand tabs (width 4) ──"
tab_samples=(
    $'col1\tcol2\tcol3'
    $'\tindented'
    $'mid\tdle'
)
for s in "${tab_samples[@]}"; do
    printf "  [%s] → [%s]\n" "$s" "$(expand_tabs "$s" 4)"
done

echo
echo "── unexpand (4 spaces → tab) ──"
unexpand_samples=(
    "    indented"
    "        2levels"
    "no    middle"
)
for s in "${unexpand_samples[@]}"; do
    printf "  [%s] → [%s]\n" "$s" "$(unexpand_tabs "$s" 4)"
done

echo
echo "── normalize EOL ──"
text=$'line1\r\nline2\rline3\nline4'
printf "  before: %d bytes (mixed)\n" ${#text}
normalized=$(normalize_eol "$text")
printf "  after:  %d bytes (all \\n)\n" ${#normalized}
echo "  result:"
echo "$normalized" | sed 's/^/    /'

echo
echo "── squash blanks ──"
input=$'a\n\n\nb\n\n\n\nc\nd'
printf "  before:\n"
echo "$input" | sed 's/^/    /'
printf "  after:\n"
squash_blanks "$input" | sed 's/^/    /'

# === ztest assertions ===
zassert_eq "$(expand_tabs $'a\tb' 4)" "a    b" "tab expanded to 4 spaces"
zassert_eq "$(expand_tabs $'a\tb' 2)" "a  b"   "tab expanded to 2 spaces"
zassert_eq "$(expand_tabs $'\tx' 4)"  "    x"  "leading tab"
zassert_eq "$(unexpand_tabs '    x' 4)" $'\tx' "4 spaces → tab"
zassert_eq "$(unexpand_tabs 'a    b' 4)" $'a\tb' "interior 4-space → tab"
nor=$(normalize_eol $'a\r\nb\rc\nd')
zassert_eq "$nor" $'a\nb\nc\nd' "EOL normalize CRLF/CR/LF"
sq=$(squash_blanks $'a\n\n\nb')
zassert_contains "$sq" $'a\n\nb' "squash multiple blank lines"
zassert_eq "$(strip '   hi   ')"    "hi"    "strip removes leading/trailing space"
zassert_eq "$(strip 'hi')"          "hi"    "strip leaves a clean string alone"
zassert_eq "$(collapse 'a   b')"    "a b"   "collapse squeezes internal runs"
ztest_run
