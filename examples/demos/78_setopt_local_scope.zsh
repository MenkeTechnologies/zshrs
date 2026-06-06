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

# === ztest assertions ===
# Verify glob option restores after the local_options fn exits.
isolated_no_glob >/dev/null
if [[ -o glob ]]; then zassert_ok 1 "glob option restored after fn"
else                   zassert_ok 0 "glob option restored after fn"; fi
# typeset locals must NOT leak.
demo_locals >/dev/null
zassert_eq "${n:-undef}" "undef" "local -i n did not leak"
zassert_eq "${f:-undef}" "undef" "local -F f did not leak"
zassert_eq "${s:-undef}" "undef" "local s did not leak"
# emulate -L sh inside local_options must not affect outer shell — verify PWD eval'd.
zassert_ok "$PWD" "PWD still accessible after emulate -L sh"
out_iso=$(isolated_no_glob)
zassert_contains "$out_iso" "OFF inside (expected)" "no_glob takes effect inside fn"
ztest_run

