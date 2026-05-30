#!/usr/bin/env zshrs
# Zsh hooks — precmd, preexec, chpwd, periodic.
# Ported from Src/exec.c hook dispatch + add-zsh-hook helper.

autoload -Uz add-zsh-hook 2>/dev/null || true

echo "── add a chpwd hook ──"
my_chpwd() {
    echo "[chpwd] now in $PWD"
}
add-zsh-hook chpwd my_chpwd 2>/dev/null || chpwd_functions+=(my_chpwd)

start=$PWD
cd /tmp
cd "$start"

echo
echo "── multiple chpwd hooks ──"
hook_log() { echo "[log] cwd → $PWD"; }
hook_breadcrumb() { echo "[breadcrumb] from $OLDPWD to $PWD"; }
chpwd_functions=(hook_log hook_breadcrumb my_chpwd)
cd /tmp
cd "$start"

echo
echo "── precmd hook (fires before each prompt) ──"
demo_precmd() { echo "[precmd] would fire before each prompt"; }
precmd_functions=(demo_precmd)

# Force one precmd-equivalent run.
for f in $precmd_functions; do $f; done

echo
echo "── preexec (before each command) ──"
demo_preexec() { echo "[preexec] would fire before: $1"; }
preexec_functions=(demo_preexec)

# Simulated trigger.
for f in $preexec_functions; do $f "echo something"; done

echo
echo "── remove a hook ──"
echo "chpwd hooks before: ${chpwd_functions[@]}"
chpwd_functions=(${chpwd_functions:#hook_log})
echo "chpwd hooks after: ${chpwd_functions[@]}"

echo
echo "── hooks via array idiom ──"
# Without add-zsh-hook, append to the *_functions array.
add_hook() {
    local kind=$1 fn=$2
    typeset -ga "${kind}_functions"
    # Check for duplicate.
    local cur="${(@P)${:-${kind}_functions}}"
    for f in $=cur; do
        [[ $f == $fn ]] && return
    done
    eval "${kind}_functions+=($fn)"
}

new_chpwd() { echo "[new_chpwd] $PWD"; }
add_hook chpwd new_chpwd
add_hook chpwd new_chpwd   # duplicate; should not add twice
echo "chpwd_functions: ${chpwd_functions[@]}"

# Cleanup
chpwd_functions=()
precmd_functions=()
preexec_functions=()
