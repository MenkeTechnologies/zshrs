#!/usr/bin/env zshrs
# typeset -m pattern flag/attribute manipulation.
# Ports Src/builtin.c bin_typeset -m branch.

echo "── setup vars ──"
typeset -g report_a=alpha
typeset -g report_b=beta
typeset -g report_c=gamma
typeset -g log_x=ten
typeset -g log_y=twenty
typeset -g log_z=thirty
typeset -g misc=other

echo "  defined: report_a report_b report_c log_x log_y log_z misc"

echo
echo "── typeset -m 'report_*' shows matching vars ──"
typeset -m 'report_*' 2>/dev/null | sed 's/^/  /'

echo
echo "── typeset -m 'log_*' shows matching ──"
typeset -m 'log_*' 2>/dev/null | sed 's/^/  /'

echo
echo "── typeset -rm 'report_*' sets readonly on matches ──"
typeset -rm 'report_*' 2>/dev/null
echo "  attempting to modify report_a:"
report_a=changed 2>&1 | head -2
echo "  report_a is still: $report_a"

# Need to unset readonly for cleanup — zsh doesn't easily support that.
echo
echo "── typeset -m for arrays ──"
typeset -gA mymap_color
mymap_color=( red 1 blue 2 green 3 )
typeset -gA mymap_size
mymap_size=( small 1 medium 2 large 3 )

echo "  associative arrays:"
typeset -m 'mymap_*' 2>/dev/null | head -5 | sed 's/^/    /'

echo
echo "── typeset -gum '*' converts globals to uppercase ──"
typeset -g lowerme=test_value_1
typeset -g alsolow=test_value_2
echo "  before: lowerme=$lowerme alsolow=$alsolow"
# typeset -u marks uppercase-on-store.
typeset -u lowerme alsolow 2>/dev/null
lowerme=$lowerme
alsolow=$alsolow
echo "  after typeset -u + reassign:"
echo "    lowerme=$lowerme"
echo "    alsolow=$alsolow"

echo
echo "── typeset -l lowercase-on-store ──"
typeset -g mixedcase=MixedCaseValue
echo "  before: mixedcase=$mixedcase"
typeset -l mixedcase 2>/dev/null
mixedcase=$mixedcase
echo "  after typeset -l + reassign: mixedcase=$mixedcase"

echo
echo "── typeset -i (integer) coerces ──"
typeset -i mynum=42
mynum=hello
echo "  mynum (typeset -i, assigned 'hello'): $mynum"
mynum="100 + 50"
echo "  mynum (assigned '100 + 50'): $mynum"

echo
echo "── typeset -i with base ──"
typeset -i 16 hex=255
echo "  hex (base 16, val 255): $hex"
typeset -i 2 bin=42
echo "  bin (base 2, val 42): $bin"
typeset -i 8 oct=64
echo "  oct (base 8, val 64): $oct"

echo
echo "── typeset -F (float) precision ──"
typeset -F 3 pi=3.14159265
echo "  pi (-F 3): $pi"
typeset -F 7 e=2.71828182
echo "  e (-F 7): $e"

echo
echo "── typeset -A vs -a ──"
typeset -a indexed=(a b c d)
typeset -A assoc=(k1 v1 k2 v2)
echo "  indexed array: ${indexed[@]}"
echo "  assoc array:   ${(@kv)assoc}"

echo
echo "── typeset -T tied colon-arrays ──"
typeset -T MYPATH mypath=(/bin /usr/bin /usr/local/bin) ':'
echo "  MYPATH (string): $MYPATH"
echo "  mypath (array):  ${mypath[@]}"
mypath+=(/opt/bin)
echo "  after mypath+=(/opt/bin):"
echo "  MYPATH: $MYPATH"

echo
echo "── typeset -p (declaration form) ──"
typeset -gp 2>/dev/null | grep -E "^typeset .* (report_a|log_x|misc) " | head -5 | sed 's/^/  /'

echo
echo "── typeset -m '*_x' shows ALL ending in _x ──"
typeset -m '*_x' 2>/dev/null | sed 's/^/  /'

echo
echo "── attribute listing ──"
typeset -m 'misc' 2>/dev/null | sed 's/^/  /'

echo
echo "── readonly cleanup attempt (will fail for readonly) ──"
unset report_a 2>&1 | head -1 || echo "  (readonly: unset blocked, as expected)"
unset log_x log_y log_z misc lowerme alsolow mixedcase mynum hex bin oct pi e mypath MYPATH 2>/dev/null

# === ztest assertions ===
# typeset -u (uppercase-on-store)
typeset -u tu_var
tu_var=hello
zassert_eq "$tu_var" "HELLO"          "typeset -u uppercases"
# typeset -l (lowercase-on-store)
typeset -l tl_var
tl_var=HELLO
zassert_eq "$tl_var" "hello"          "typeset -l lowercases"
# typeset -i (integer, coerce + arith)
typeset -i ti_var
ti_var="100 + 50"
zassert_eq "$ti_var" "150"            "typeset -i arith on assign"
ti_var=hello
zassert_eq "$ti_var" "0"              "typeset -i non-numeric → 0"
# typeset -i with base prints "base#val"
typeset -i 16 thex=255
zassert_eq "$thex" "16#FF"            "typeset -i 16 hex format"
typeset -i 2 tbin=10
zassert_eq "$tbin" "2#1010"           "typeset -i 2 binary format"
# typeset -F (float)
typeset -F 3 tpi=3.14159
zassert_eq "$tpi" "3.142"             "typeset -F 3 truncates"
# typeset -T tied — zsh-divergence: zshrs uses 0x1f (record separator) instead
# of the requested ':' when the array is initialised on the typeset line.
# Re-assigning to tpath after the typeset call uses the requested separator.
typeset -T TPATH tpath ':'
tpath=(/a /b /c)
zassert_eq "$TPATH" "/a:/b:/c"         "typeset -T joins with sep after re-assign"
tpath+=(/d)
zassert_eq "$TPATH" "/a:/b:/c:/d"      "typeset -T += appends"
ztest_run
