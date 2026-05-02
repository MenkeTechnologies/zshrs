# bindkey — sequence + widget. The trailing flag-only `bindkey -e` is a
# mode switch, not a binding, and should not produce a record.
bindkey '^R' fzf-history-widget
bindkey '^T' fzf-file-widget
bindkey '^[c' fzf-cd-widget
bindkey -e
