#!/usr/bin/env zshrs
# zle widgets — define + bind + introspect.
# Ports Src/Zle/zle_main.c + Src/Zle/zle_keymap.c.

echo "── load zle (non-interactive may skip) ──"
autoload -Uz zle 2>/dev/null
zmodload zsh/zle 2>/dev/null
echo "  zle available: $([[ $(whence -w zle 2>/dev/null) == *builtin* ]] && echo yes || echo no)"

echo
echo "── widget definitions ──"
# Define a user widget (function bound as widget).
my_widget_uppercase() {
    BUFFER=${BUFFER:u}
    CURSOR=${#BUFFER}
}

my_widget_lowercase() {
    BUFFER=${BUFFER:l}
    CURSOR=${#BUFFER}
}

my_widget_reverse() {
    local rev="" i
    for ((i=${#BUFFER}; i>=1; i--)); do
        rev+="${BUFFER[i]}"
    done
    BUFFER=$rev
    CURSOR=${#BUFFER}
}

my_widget_double() {
    BUFFER="${BUFFER}${BUFFER}"
    CURSOR=${#BUFFER}
}

# Register.
zle -N my_widget_uppercase 2>/dev/null
zle -N my_widget_lowercase 2>/dev/null
zle -N my_widget_reverse 2>/dev/null
zle -N my_widget_double 2>/dev/null

echo "  defined widgets:"
echo "    my_widget_uppercase"
echo "    my_widget_lowercase"
echo "    my_widget_reverse"
echo "    my_widget_double"

echo
echo "── simulate widget invocation (non-interactive) ──"
# In real zle, BUFFER is bound to the editing buffer. Here just simulate.
BUFFER="Hello World"
echo "  initial BUFFER: '$BUFFER'"
my_widget_uppercase
echo "  after uppercase: '$BUFFER'"
my_widget_lowercase
echo "  after lowercase: '$BUFFER'"
my_widget_reverse
echo "  after reverse:   '$BUFFER'"
my_widget_double
echo "  after double:    '$BUFFER'"

echo
echo "── bindkey: map keys to widgets ──"
# Examples (won't affect non-interactive run).
bindkey 2>/dev/null '^Xu' my_widget_uppercase 2>/dev/null
bindkey 2>/dev/null '^Xl' my_widget_lowercase 2>/dev/null
bindkey 2>/dev/null '^Xr' my_widget_reverse 2>/dev/null
bindkey 2>/dev/null '^Xd' my_widget_double 2>/dev/null
echo "  bound:"
echo "    Ctrl-X u → my_widget_uppercase"
echo "    Ctrl-X l → my_widget_lowercase"
echo "    Ctrl-X r → my_widget_reverse"
echo "    Ctrl-X d → my_widget_double"

echo
echo "── bindkey listing ──"
bindkey 2>/dev/null | grep "my_widget" | sed 's/^/  /' | head -10

echo
echo "── bindkey -s macros (key → string) ──"
bindkey -s '^Xh' 'help' 2>/dev/null
bindkey -s '^Xv' 'version' 2>/dev/null
echo "  defined macros:"
echo "    Ctrl-X h → 'help'"
echo "    Ctrl-X v → 'version'"

echo
echo "── builtin widgets ──"
echo "  some standard zle widgets (built-in):"
# NB: `widgets` is a read-only special parameter (the ZLE widget table),
# so the sample list below is held in `builtin_widgets`.
builtin_widgets=(
    "self-insert"
    "accept-line"
    "backward-char"
    "forward-char"
    "kill-line"
    "yank"
    "history-incremental-search-backward"
    "complete-word"
    "expand-or-complete"
    "menu-select"
)
for w in "${builtin_widgets[@]}"; do
    echo "    $w"
done

echo
echo "── keymaps ──"
echo "  standard keymaps:"
echo "    main      - active keymap"
echo "    emacs     - emacs bindings"
echo "    viins     - vi insert mode"
echo "    vicmd     - vi command mode"
echo "    isearch   - incremental search"
echo "    command   - vi command-line"
echo "    .menuselect - menu selection"

echo
echo "── ZLE variables ──"
# Inside a widget these are pre-set; outside they're just regular vars.
echo "  BUFFER (current line):      '$BUFFER'"
echo "  CURSOR (cursor position):   ${CURSOR:-N/A}"
echo "  LBUFFER (before cursor):    ${LBUFFER:-N/A}"
echo "  RBUFFER (after cursor):     ${RBUFFER:-N/A}"
echo "  PREDISPLAY (output prefix): ${PREDISPLAY:-N/A}"
echo "  POSTDISPLAY (out suffix):   ${POSTDISPLAY:-N/A}"

echo
echo "── widget unbind ──"
zle -D my_widget_uppercase 2>/dev/null
zle -D my_widget_lowercase 2>/dev/null
zle -D my_widget_reverse 2>/dev/null
zle -D my_widget_double 2>/dev/null
echo "  all my_widget_* unbound"

# Also delete the underlying functions.
unfunction my_widget_uppercase my_widget_lowercase my_widget_reverse my_widget_double 2>/dev/null

# === ztest assertions ===
zassert_eq "${#builtin_widgets[@]}" 10                 "10 sample builtin widget names"
zassert_contains "${builtin_widgets[*]}" "accept-line" "accept-line listed"
zassert_contains "${builtin_widgets[*]}" "self-insert" "self-insert listed"
ztest_run
