#!/usr/bin/env zshrs
# Pattern matching — [[ glob ]], case patterns, extended globs.

setopt extended_glob

inputs=(
    "hello.txt"
    "image.PNG"
    "log_2026.gz"
    "data.csv"
    "README"
    "main.zsh"
    "test.zshrc"
)

echo "── [[ str == pat ]] ──"
for f in "${inputs[@]}"; do
    if [[ $f == *.txt ]]; then
        echo "$f matches *.txt"
    fi
done

echo "── case dispatch on extension ──"
for f in "${inputs[@]}"; do
    case $f in
        *.txt) echo "$f: text" ;;
        *.PNG|*.png|*.jpg) echo "$f: image" ;;
        *.gz|*.zip) echo "$f: archive" ;;
        *.csv) echo "$f: data" ;;
        *.zsh|*.zshrc) echo "$f: zsh script" ;;
        *) echo "$f: other" ;;
    esac
done

echo "── numeric range ──"
for n in 0 5 10 25 50 99 100; do
    case $n in
        0)        echo "$n: zero" ;;
        [1-9])    echo "$n: single" ;;
        [1-9][0-9])  echo "$n: double" ;;
        *)        echo "$n: triple+" ;;
    esac
done

echo "── extended globs (under setopt extended_glob) ──"
words=(apple banana cherry date apricot)
for w in "${words[@]}"; do
    [[ $w == ap* ]] && echo "ap-prefix: $w"
done

# === ztest assertions ===
classify_ext() {
    case $1 in
        *.txt) echo text ;;
        *.PNG|*.png|*.jpg) echo image ;;
        *.gz|*.zip) echo archive ;;
        *.csv) echo data ;;
        *.zsh|*.zshrc) echo "zsh script" ;;
        *) echo other ;;
    esac
}
zassert_eq "$(classify_ext hello.txt)"   "text"        "txt → text"
zassert_eq "$(classify_ext image.PNG)"   "image"       "PNG → image"
zassert_eq "$(classify_ext log_2026.gz)" "archive"     "gz → archive"
zassert_eq "$(classify_ext data.csv)"    "data"        "csv → data"
zassert_eq "$(classify_ext README)"      "other"       "no-ext → other"
zassert_eq "$(classify_ext test.zshrc)"  "zsh script"  "zshrc → zsh script"
classify_num() {
    case $1 in
        0)        echo zero ;;
        [1-9])    echo single ;;
        [1-9][0-9])  echo double ;;
        *)        echo "triple+" ;;
    esac
}
zassert_eq "$(classify_num 0)"   "zero"     "0 → zero"
zassert_eq "$(classify_num 5)"   "single"   "5 → single"
zassert_eq "$(classify_num 50)"  "double"   "50 → double"
zassert_eq "$(classify_num 100)" "triple+"  "100 → triple+"
[[ apple == ap* ]] && r=1 || r=0
zassert_eq "$r" "1"  "apple matches ap*"
[[ banana == ap* ]] && r=1 || r=0
zassert_eq "$r" "0"  "banana not ap*"
ztest_run
