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
