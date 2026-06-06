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

# === ztest assertions ===
unsetopt extendedglob nullglob noclobber pipefail 2>/dev/null
# enable + verify
setopt extendedglob
if [[ -o extendedglob ]]; then zassert_ok 1 "setopt extendedglob ON"
else                            zassert_ok 0 "setopt extendedglob ON"; fi
unsetopt extendedglob
if [[ -o extendedglob ]]; then zassert_ok 0 "unsetopt extendedglob OFF"
else                            zassert_ok 1 "unsetopt extendedglob OFF"; fi
# multi-toggle
setopt nullglob noclobber pipefail
if [[ -o nullglob   ]]; then zassert_ok 1 "multi-setopt nullglob"
else                          zassert_ok 0 "multi-setopt nullglob"; fi
if [[ -o noclobber  ]]; then zassert_ok 1 "multi-setopt noclobber"
else                          zassert_ok 0 "multi-setopt noclobber"; fi
if [[ -o pipefail   ]]; then zassert_ok 1 "multi-setopt pipefail"
else                          zassert_ok 0 "multi-setopt pipefail"; fi
unsetopt nullglob noclobber pipefail
# set -o / set +o
set -o noclobber
if [[ -o noclobber ]]; then zassert_ok 1 "set -o noclobber"
else                         zassert_ok 0 "set -o noclobber"; fi
set +o noclobber
if [[ -o noclobber ]]; then zassert_ok 0 "set +o noclobber"
else                         zassert_ok 1 "set +o noclobber"; fi
# underscore alias
setopt glob_subst
if [[ -o globsubst ]]; then zassert_ok 1 "underscore alias matches"
else                          zassert_ok 0 "underscore alias matches"; fi
unsetopt globsubst
ztest_run

