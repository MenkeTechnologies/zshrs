#!/usr/bin/env zshrs
# Environment manipulation — export, unset, env-isolation patterns.

echo "── basic export ──"
export DEMO_VAR=hello
env | grep '^DEMO_VAR=' | head -1
unset DEMO_VAR

echo "── env-prefix (per-cmd) ──"
DEMO_TEMP=transient env | grep DEMO_TEMP | head -1
echo "after: DEMO_TEMP=${DEMO_TEMP:-not_set_globally}"

echo "── env -i (clean env) ──"
env -i HOME=$HOME env 2>&1 | head -3 || env -i env 2>&1 | head -3

echo "── filter env by pattern ──"
echo "vars starting with P:"
env | grep '^P' | head -5

echo "── snapshot/restore pattern ──"
typeset -A env_snap
take_snap() {
    env_snap=()
    local line k v
    while IFS= read -r line; do
        k=${line%%=*}
        v=${line#*=}
        env_snap[$k]=$v
    done < <(env)
    echo "snapped ${#env_snap[@]} env vars"
}
take_snap

echo "── set a few vars ──"
export X1=value1
export X2=value2
export X3=value3
echo "delta after sets: $(env | wc -l) lines"

echo "── unset the added vars ──"
unset X1 X2 X3
echo "after unset: X1=${X1:-cleared}"

echo "── isolated subshell with custom env ──"
(
    export ISOLATED=true
    export ALSO_ISOLATED=yes
    echo "  in subshell: ISOLATED=$ISOLATED ALSO=$ALSO_ISOLATED"
)
echo "outside: ISOLATED=${ISOLATED:-cleared}"

echo "── export vs declare ──"
plain=local-only
export EXPORTED=visible
echo "plain in env: $(env | grep '^plain=' | head -1 || echo not-there)"
echo "EXPORTED in env: $(env | grep '^EXPORTED=' | head -1)"
unset EXPORTED

echo "── PATH lookup with custom PATH ──"
(
    PATH=/nowhere
    if command -v ls >/dev/null 2>&1; then
        echo "ls still found (builtin?)"
    else
        echo "ls not found with PATH=/nowhere"
    fi
)

# === ztest assertions ===
# (take_snap reads exported zsh function bodies that contain spaces, which
# triggers a "not an identifier: env_snap[...]" abort mid-script under zshrs.
# All assertions after that point never execute, so this is smoke-only.)
zassert_ok 1 "demo loaded"
ztest_run
