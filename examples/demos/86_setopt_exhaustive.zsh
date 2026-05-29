#!/usr/bin/env zshrs
# setopt / unsetopt — enable, disable, query, list.
# Ported from Src/options.c (bin_setopt, optlookup, dosetopt).

echo "── query option state via [[ -o name ]] ──"
for opt in extendedglob shwordsplit globsubst kshtypeset nullglob noclobber; do
    if [[ -o $opt ]]; then
        printf "%-15s ON\n" "$opt"
    else
        printf "%-15s off\n" "$opt"
    fi
done

echo "── enable extendedglob, verify, disable ──"
setopt extendedglob
[[ -o extendedglob ]] && echo "extendedglob: ON"
unsetopt extendedglob
[[ -o extendedglob ]] && echo "ERROR: still on" || echo "extendedglob: off"

echo "── multi-toggle in one call ──"
setopt nullglob noclobber pipefail
for opt in nullglob noclobber pipefail; do
    [[ -o $opt ]] && echo "  $opt: ON"
done
unsetopt nullglob noclobber pipefail

echo "── plus/minus sign on setopt ──"
set -o noclobber
[[ -o noclobber ]] && echo "via -o: ON"
set +o noclobber
[[ -o noclobber ]] && echo "ERROR" || echo "via +o: off"

echo "── option aliases ──"
# Many options have aliases — `glob_subst` vs `globsubst` etc.
setopt glob_subst
[[ -o globsubst ]] && echo "underscored name matched"
unsetopt globsubst

echo "── emulate -L modes (zsh/sh/ksh/bash/csh) ──"
demo_emul() {
    setopt local_options
    emulate -L sh
    # In sh emul, most zsh-isms (extendedglob, *-modifiers etc) off
    [[ -o shwordsplit ]] && echo "  in sh emulate: shwordsplit on"
}
demo_emul
[[ -o shwordsplit ]] && echo "outside: shwordsplit on" || echo "outside: shwordsplit off"
