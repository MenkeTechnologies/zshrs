# Shell-mode-quirk dissolution: only the branch that actually executes
# produces records. Recorder runs in non-interactive, no-fzf mode so
# the if-branch fails and only the elif-branch fires.
if (( $+commands[fzf] )) && [[ -o interactive ]]; then
    alias never1='this never fires'
    alias never2='nor this'
elif true; then
    alias from_elif='caught'
fi
