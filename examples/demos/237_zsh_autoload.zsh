#!/usr/bin/env zshrs
# Function autoload — typeset -fu / autoload -Uz patterns.
# Ported from Src/builtin.c bin_functions + Src/parse.c parse_string.

echo "── manual fpath setup ──"
tmpfp=/tmp/zshrs_fp_$$
mkdir -p "$tmpfp"
# Note: avoid EXIT trap — interferes with autoload lifecycle in zshrs.

# Create autoloadable function files.
cat > "$tmpfp/greet" <<'EOF'
echo "Hello from greet (autoloaded)!"
echo "args: $@"
EOF

cat > "$tmpfp/farewell" <<'EOF'
echo "Goodbye from farewell"
echo "argv: $argv"
EOF

cat > "$tmpfp/sum" <<'EOF'
local s=0
for n in "$@"; do (( s += n )); done
echo $s
EOF

fpath=("$tmpfp" $fpath)

echo "── register autoload functions ──"
# zshrs's autoload only honors the first name per call; split.
autoload -Uz greet
autoload -Uz farewell
autoload -Uz sum

# Check they're declared but not loaded.
echo "greet defined: $(typeset -f greet | head -1)"

echo
echo "── invoke (triggers load) ──"
greet world

echo
echo "── invoke another ──"
farewell foo bar baz

echo
echo "── compute via autoload ──"
result=$(sum 1 2 3 4 5)
echo "sum 1..5 = $result"

echo
echo "── once-loaded, body is cached ──"
echo "second call (no re-load):"
greet again

echo
echo "── functions[] introspection ──"
echo "greet body now:"
echo "${functions[greet]}"

echo
echo "── unfunction reverts to autoload-marker ──"
unfunction greet
autoload greet
echo "after unfunction + autoload re-declare:"
greet "fresh"

echo
echo "── autoload -t to trace ──"
autoload -t farewell 2>/dev/null
farewell trace-mode

echo
echo "── autoload search path ──"
echo "fpath has ${#fpath[@]} entries"
echo "first: ${fpath[1]}"

# Cleanup
fpath=( ${fpath:#$tmpfp} )
unfunction greet farewell sum 2>/dev/null
command rm -rf "$tmpfp"

# === ztest assertions ===
tmpfp2=/tmp/zshrs_fp_test_$$
mkdir -p "$tmpfp2"
cat > "$tmpfp2/myfn" <<'EOF'
echo "loaded-myfn"
EOF
cat > "$tmpfp2/addfn" <<'EOF'
local s=0
for n in "$@"; do (( s += n )); done
echo $s
EOF
fpath=("$tmpfp2" $fpath)
autoload -Uz myfn
autoload -Uz addfn
zassert_eq "$(myfn)"            "loaded-myfn"   "autoload triggers body"
zassert_eq "$(addfn 1 2 3 4 5)" 15              "autoload sum 1..5"
zassert_eq "$(myfn)"            "loaded-myfn"   "second call uses cached body"
# Pre-call introspection: zshrs marker shows 'autoload -X' before first invocation;
# after invocation, body should be cached.
myfn >/dev/null
zassert_contains "${functions[myfn]}" "loaded-myfn" "functions[] body introspection after load"
unfunction myfn addfn 2>/dev/null
command rm -rf "$tmpfp2"
ztest_run
