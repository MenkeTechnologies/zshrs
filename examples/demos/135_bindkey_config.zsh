#!/usr/bin/env zshrs
# bindkey configuration — keymap management API.
# Ported from Src/Zle/zle_keymap.c + zle_bindings.c bin_bindkey.
# (No actual key firing — we're non-interactive.)

echo "── list current keymap names ──"
bindkey -l 2>/dev/null || echo "(bindkey list may need zle)"

echo "── create a new keymap (copy of emacs) ──"
bindkey -N mymap emacs 2>/dev/null && echo "created 'mymap' keymap" \
    || echo "(could not create keymap — non-interactive)"

echo "── add a binding ──"
bindkey -M emacs '^X^A' beginning-of-line 2>/dev/null \
    && echo "bound ^X^A" || echo "(bindkey -M may need zle)"

echo "── query a binding ──"
bindkey -M emacs '^A' 2>/dev/null || echo "(query needs zle)"

echo "── show current emacs bindings (first 5) ──"
bindkey -M emacs 2>/dev/null | head -5

echo "── delete a binding ──"
bindkey -M emacs -r '^X^A' 2>/dev/null && echo "removed ^X^A binding" \
    || echo "(removal requires zle)"

echo "── self-insert as fallback ──"
bindkey -M emacs '^I' complete-word 2>/dev/null
echo "Tab → complete-word"

echo "── declare a widget ──"
my_widget() {
    echo "[widget called]"
}
zle -N my_widget 2>/dev/null && echo "widget 'my_widget' declared" \
    || echo "(zle -N needs interactive shell)"

echo "── bind a widget ──"
bindkey -M emacs '^X^B' my_widget 2>/dev/null \
    && echo "bound ^X^B → my_widget" || echo "(bindkey requires zle setup)"

echo "── note: zle widgets only fire on TTY input ──"
echo "(this demo just exercises the config API; widgets do nothing"
echo " in -c mode because there's no line editor)"

# === ztest assertions ===
km_list=$(bindkey -l 2>/dev/null)
zassert_contains "$km_list" "emacs"   "emacs keymap listed"
zassert_contains "$km_list" "viins"   "viins keymap listed"
zassert_contains "$km_list" "vicmd"   "vicmd keymap listed"
# Bind, query, delete round-trip on emacs '^X^A'.
bindkey -M emacs '^X^Q' beginning-of-line 2>/dev/null
q=$(bindkey -M emacs '^X^Q' 2>/dev/null)
zassert_contains "$q" "beginning-of-line" "bind+query round-trip"
bindkey -M emacs -r '^X^Q' 2>/dev/null
q_after=$(bindkey -M emacs '^X^Q' 2>/dev/null)
zassert_eq "$q_after" '"^X^Q" undefined-key' "removal yields undefined-key"
zassert_contains "$(bindkey -M emacs '^A' 2>/dev/null)" "beginning-of-line" "default ^A binding"
ztest_run
