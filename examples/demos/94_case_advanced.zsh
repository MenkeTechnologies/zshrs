#!/usr/bin/env zshrs
# `case` advanced — pattern alternation, fallthrough (;& ;;& in zsh),
# extended glob patterns.
# Ported from Src/exec.c execcase + Src/pattern.c patmatch.

setopt extended_glob

classify_char() {
    local c=$1
    case $c in
        [0-9])    echo "$c → digit" ;;
        [a-z])    echo "$c → lowercase" ;;
        [A-Z])    echo "$c → uppercase" ;;
        ' '|$'\t') echo "$c → whitespace" ;;
        [-_.,\;:]) echo "$c → punctuation" ;;
        *)        echo "$c → other" ;;
    esac
}

for c in a Z 5 ',' ' ' '!'; do
    classify_char "$c"
done

echo "── alternation with | ──"
greet() {
    case $1 in
        hi|hello|hey) echo "informal greeting" ;;
        good\ morning|good\ evening) echo "polite" ;;
        *) echo "not a greeting" ;;
    esac
}
greet "hi"
greet "hello"
greet "good morning"
greet "asdf"

echo "── range patterns ──"
season() {
    case $1 in
        [0-2]|12) echo "winter" ;;
        [3-5])    echo "spring" ;;
        [6-8])    echo "summer" ;;
        9|10|11)  echo "fall" ;;
        *)        echo "invalid month" ;;
    esac
}
for m in 1 4 7 10 12 13; do
    printf "month %s: " $m
    season $m
done

echo "── fallthrough with ;& (next case runs unconditionally) ──"
test_fallthrough() {
    case $1 in
        a) echo "a"; ;&    # fall through
        b) echo "b"; ;&    # fall through
        c) echo "c" ;;
        d) echo "d" ;;
    esac
}
echo "case a:"; test_fallthrough a
echo "case b:"; test_fallthrough b
echo "case c:"; test_fallthrough c
echo "case d:"; test_fallthrough d

echo "── (zsh ;;& 'continue searching' omitted — currently parses)"

echo "── globbed cases ──"
file_kind() {
    case $1 in
        *.zsh|*.sh|*.bash) echo "shell script" ;;
        *.py)              echo "python" ;;
        *.rs)              echo "rust" ;;
        *.md|*.txt|*.rst)  echo "text" ;;
        *.tar.gz|*.tgz|*.zip|*.bz2) echo "archive" ;;
        Makefile|makefile) echo "make recipe" ;;
        *)                 echo "unknown" ;;
    esac
}
for f in build.zsh main.rs README.md data.tar.gz Makefile script.py log.bin; do
    printf "%-15s → " $f
    file_kind $f
done

# === ztest assertions ===
zassert_eq "$(classify_char 5)"   "5 → digit"        "bracket range [0-9]"
zassert_eq "$(classify_char a)"   "a → lowercase"    "bracket range [a-z]"
zassert_eq "$(classify_char Z)"   "Z → uppercase"    "bracket range [A-Z]"
zassert_eq "$(classify_char ,)"   ", → punctuation"  "escaped ; inside a bracket set"
zassert_eq "$(classify_char ' ')" "  → whitespace"   "quoted-space alternation"
zassert_eq "$(classify_char '!')" "! → other"        "default arm"
zassert_eq "$(greet hi)"             "informal greeting" "| alternation"
zassert_eq "$(greet 'good morning')" "polite"            "escaped space in a pattern"
zassert_eq "$(greet asdf)"           "not a greeting"    "greet default arm"
zassert_eq "$(season 1)"  "winter" "range [0-2]"
zassert_eq "$(season 12)" "winter" "literal 12 alternative"
zassert_eq "$(season 7)"  "summer" "range [6-8]"
zassert_eq "$(season 13)" "invalid month" "season default arm"
zassert_eq "$(test_fallthrough a)" "a
b
c"                                             ";& falls through twice"
zassert_eq "$(test_fallthrough c)" "c"        "no fallthrough into d"
zassert_eq "$(file_kind build.zsh)"    "shell script" "glob alternation *.zsh"
zassert_eq "$(file_kind data.tar.gz)"  "archive"      "multi-suffix glob"
zassert_eq "$(file_kind Makefile)"     "make recipe"  "literal filename arm"
zassert_eq "$(file_kind log.bin)"      "unknown"      "file_kind default arm"
ztest_run

