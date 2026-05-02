zinit ice wait"0" lucid as"program" pick"bin/foo"
zinit light user/repo
zinit ice wait"!1" depth"1"
zinit snippet OMZ::lib/git.zsh
zinit ice from"gh-r" as"program"
zinit load junegunn/fzf-bin
zinit ice wait lucid atload'_zsh_autosuggest_start' atinit'ZINIT[COMPINIT_OPTS]=-C; zicompinit; zicdreplay'
zinit light zsh-users/zsh-autosuggestions
