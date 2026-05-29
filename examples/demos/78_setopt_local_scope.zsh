#!/usr/bin/env zshrs
# setopt local_options — scope option flips to the enclosing function.
# Ported from zsh's Src/options.c bin_setopt + the LOCAL_OPTIONS bit.

# Global glob is on by default; demo flipping it off inside a fn and
# verifying it pops back out.
isolated_no_glob() {
    setopt local_options
    setopt no_glob
    # In here, * should not expand.
    echo "inside (noglob): *"
    if [[ -o glob ]]; then
        echo "  ERROR: glob is on inside!"
    else
        echo "  glob is OFF inside (expected)"
    fi
}

echo "── before ──"
if [[ -o glob ]]; then
    echo "glob ON"
fi

isolated_no_glob

echo "── after ──"
if [[ -o glob ]]; then
    echo "glob ON (restored)"
else
    echo "ERROR: glob did not restore"
fi

echo "── local_traps + trap ──"
outer_trap_demo() {
    setopt local_options
    trap 'echo "inner cleanup"' EXIT
    {
        setopt local_traps
        trap 'echo "innermost cleanup"' EXIT
        echo "innermost zone"
    }   # subshell with local trap
    echo "back in outer trap zone"
}
outer_trap_demo

echo "── float / int locals via typeset ──"
demo_locals() {
    local -i n=42
    local -F 3 f=3.14159
    local s="scalar"
    echo "n=$n f=$f s=$s"
}
demo_locals
echo "outside locals: ${n:-undef} ${f:-undef} ${s:-undef}"

echo "── emulate sh inside fn ──"
emulate_zsh_check() {
    emulate -L sh
    setopt local_options
    # In sh emulation, many zsh-isms are off.
    echo "in sh-emul: globbing literal *"
}
emulate_zsh_check
echo "back in zsh — pwd $PWD recognized"
