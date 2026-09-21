zstyle ':completion:*' completer _expand _ignored _megacomplete _approximate _correct _fasd_zsh_word_complete_trigger
zstyle ':completion:*' insert-sections on
zstyle ':completion:*:*:(f|z|zshz|zpwr-z|zpwr-gitzfordir|zpwr-gitzfordirmain|zpwr-gitzfordirdevelop|zm|zd|zg):*:*' cache-policy zpwrDailyCachingPolicy
