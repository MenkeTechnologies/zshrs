#!/usr/bin/env zshrs
# Padding / formatting parameter flags — (l:N::str:) (r:N::str:) (#).
# Ported from zsh's subst.c paramsubst's pad_flags branch.

echo "── (l:N::pad:) left-pad to width N ──"
v=42; echo "[${(l:8::0:)v}]      # zero-pad to width 8"
h=hello; echo "[${(l:10:: :)h}]  # space-pad to width 10"
x=X; echo "[${(l:8::-:)x}]       # dash-pad"

echo "── (r:N::pad:) right-pad to width N ──"
echo "[${(r:8::*:)h}]   # right-pad with *"
y=x; echo "[${(r:10:: :)y}]      # right-pad with space"

echo "── arrays each padded ──"
arr=(a bb ccc dddd eeeee)
for x in $arr; do
    echo "[${(l:8:: :)x}] [${(r:8:: :)x}]"
done

echo "── numeric formatting ──"
nums=(1 10 100 1000 9999)
for n in $nums; do
    echo "[${(l:6::0:)n}]    # ASCII number padding"
done

echo "── centered banner via manual padding ──"
banner() {
    local txt=$1 w=$2
    local n=${#txt}
    local lpad=$(( (w - n) / 2 ))
    local rpad=$(( w - n - lpad ))
    local left=""; local right=""
    local i
    for ((i = 0; i < lpad; i++)); do left+=" "; done
    for ((i = 0; i < rpad; i++)); do right+=" "; done
    echo "[${left}${txt}${right}]"
}
banner "HELLO" 20
banner "WORLD" 20
banner "zshrs!" 20

# === ztest assertions ===
v=42; zassert_eq "${(l:8::0:)v}"      "00000042" "(l) zero-pad 42 to width 8"
h=hello; zassert_eq "${(l:10:: :)h}"  "     hello" "(l) space-pad hello to 10"
x=X; zassert_eq "${(l:8::-:)x}"       "-------X"  "(l) dash-pad X to 8"
zassert_eq "${(r:8::*:)h}"            "hello***"  "(r) star right-pad"
y=x; zassert_eq "${(r:10:: :)y}"      "x         " "(r) space right-pad"
n=42; zassert_eq "${(l:6::0:)n}"      "000042"    "(l) zero-pad 42 to 6"
zassert_eq "$(banner HELLO 20)"       "[       HELLO        ]" "banner HELLO width 20"
zassert_eq "$(banner ZS 6)"           "[  ZS  ]"   "banner ZS width 6"
ztest_run
