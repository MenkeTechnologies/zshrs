zstyle ':completion:*:*:*:*:*' original 1
zstyle ':completion:*:*' matcher-list 'r:[[:alpha:]]|[A-Z0-9]=by m:{a-z-}={A-Z_}'
zstyle ':completion:*:*:*:*:default' list-suffixes false
zstyle ':completion:*:match:grep:*:paths' list-suffixes 1
