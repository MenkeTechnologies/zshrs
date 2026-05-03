# `zle -N` widget definition + `zle -A` widget alias. Both are state
# mutations zinit-report surfaces as part of plugin reports — the
# recorder gives them a dedicated `zle` kind so query-side can answer
# "which plugin installed this widget" with file:line precision.
#
# Note: the underlying handler (the `name() {}` function) fires the
# `function` kind separately. A widget bound to its own name still
# emits BOTH the function record AND the zle record.
fzf-history-widget() { :; }
fzf-cd-widget() { :; }
zle -N fzf-history-widget
zle -N alt-history fzf-history-widget
zle -N fzf-cd-widget
zle -A fzf-history-widget aliased-history
