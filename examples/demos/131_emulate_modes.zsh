#!/usr/bin/env zshrs
# emulate -L sh / ksh / csh — temporary emulation modes.
# Ported from Src/options.c bin_emulate + per-mode flag flips.

echo "── current mode (zsh) ──"
[[ -o shwordsplit ]] && echo "shwordsplit ON" || echo "shwordsplit OFF"
[[ -o nomatch ]] && echo "nomatch ON" || echo "nomatch OFF"

echo "── inside emulate -L sh ──"
sh_demo() {
    emulate -L sh
    [[ -o shwordsplit ]] && echo "shwordsplit (sh): ON" || echo "shwordsplit (sh): OFF"
    [[ -o globsubst ]] && echo "globsubst (sh): ON" || echo "globsubst (sh): OFF"
    [[ -o noclobber ]] && echo "noclobber (sh): ON" || echo "noclobber (sh): OFF"
}
sh_demo

echo "── inside emulate -L ksh ──"
ksh_demo() {
    emulate -L ksh
    [[ -o kshtypeset ]] && echo "kshtypeset (ksh): ON" || echo "kshtypeset (ksh): OFF"
    [[ -o kshglob ]] && echo "kshglob (ksh): ON" || echo "kshglob (ksh): OFF"
}
ksh_demo

echo "── after emulate scopes ──"
[[ -o shwordsplit ]] && echo "shwordsplit ON (leaked?)" || echo "shwordsplit OFF (restored)"

echo "── word splitting demo (zsh: NO split, sh: yes split) ──"
s="one two three"
zsh_mode() {
    local result=( $s )
    echo "zsh mode count: ${#result[@]}"
}
sh_mode() {
    emulate -L sh
    local result=( $s )
    echo "sh mode count: ${#result[@]}"
}
zsh_mode
sh_mode

echo "── nomatch behavior ──"
zsh_nomatch() {
    setopt local_options
    ls /tmp/*.zsh_nomatch_xyz 2>&1 | head -1
}
zsh_nomatch

sh_nomatch() {
    emulate -L sh
    ls /tmp/*.sh_nomatch_xyz 2>/dev/null
    echo "sh: silently empty"
}
sh_nomatch

echo "── ksh-specific (([[ ]]) vs (())) ──"
ksh_arith() {
    emulate -L ksh
    (( x = 5 + 3 ))
    echo "ksh arith: x=$x"
}
ksh_arith

echo "── snapshot of opts before/after fn with emulate ──"
echo "before: $(echo $-)"
sh_demo
echo "after: $(echo $-)"

# === ztest assertions ===
# emulate -L should locally flip opts and restore on return.
zassert_contains "$(sh_demo)"  "shwordsplit (sh): ON"  "emulate -L sh flips shwordsplit"
zassert_contains "$(sh_demo)"  "globsubst (sh): ON"    "emulate -L sh flips globsubst"
zassert_contains "$(ksh_demo)" "kshglob (ksh): ON"     "emulate -L ksh flips kshglob"
# After fn returns, outer scope must have shwordsplit OFF again.
[[ -o shwordsplit ]] && zassert_ok 0 "outer shwordsplit restored OFF" || zassert_ok 1 "outer shwordsplit restored OFF"
# ksh arith uses (( )) and assigns x in caller scope.
zassert_contains "$(ksh_arith)" "x=8" "ksh arith 5+3"
# zsh_mode (no shwordsplit) keeps "one two three" as one word in $s
# (assigned to result=( $s ) — zsh doesn't word-split unquoted vars).
zassert_eq "$(zsh_mode)" "zsh mode count: 1" "zsh: no shwordsplit"
ztest_run
