# zstyle fixture for compsys_parity.py — the user's real completion config.
# Sourced identically into `zsh -f` and `zshrs --zsh -f` so the parity harness
# compares completion under the actual daily-driver zstyle setup, not defaults.
zstyle ':vcs_info:svn*:*' actionformats '%c%u %F{1}| %a%f'
zstyle ':vcs_info:*' actionformats '%b %F{1}| %a%f'
zstyle ':completion:*' auto-description 'Specify: %d'
zstyle ':vcs_info:hg*:*' branchformat ' %b'
zstyle ':completion:*' cache-path /Users/tommy/.zpwr/local/zcompcache
zstyle ':completion:*:*:(f|z|zshz|zpwr-z|zpwr-gitzfordir|zpwr-gitzfordirmain|zpwr-gitzfordirdevelop|zm|zd|zg):*:*' cache-policy zpwrDailyCachingPolicy
zstyle ':completion:daily-cache:*' cache-policy zpwrDailyCachingPolicy
zstyle ':completion:*:(fasd|fasd-file|zdir)' cache-policy zpwrDailyCachingPolicy
zstyle ':completion:*' cache-policy zpwrMonthlyCachingPolicy
zstyle ':vcs_info:*' check-for-changes true
zstyle ':completion:*:("|'"'"''"'"'|)killall("|'"'"''"'"'|):*' command 'ps -o command'
zstyle ':fzf-tab:*' command fzf --ansi --ansi '--expect=$continuous_trigger' '--color=hl:$(( $#headers == 0 ? 108 : 255 ))' '--nth=2,3' '--delimiter=\x00' '--layout=reverse' '--height=${FZF_TMUX_HEIGHT:=100%}' '--tiebreak=begin' -m '--query=$query' '--header-lines=$#headers' --print-query --ansi '--expect=$continuous_trigger' '--color=hl:$(( $#headers == 0 ? 108 : 255 ))' '--nth=2,3' '--delimiter=\x00' '--layout=reverse' '--height=${FZF_TMUX_HEIGHT:=100%}' '--tiebreak=begin' -m '--query=$query' '--header-lines=$#headers' --print-query
zstyle ':completion:*' complete-options on
zstyle ':completion:fasd-complete:*' completer _fasd_zsh_word_complete
zstyle ':completion:fasd-complete-f:*' completer _fasd_zsh_word_complete_f
zstyle ':completion:fasd-complete-d:*' completer _fasd_zsh_word_complete_d
zstyle ':completion:*' completer _expand _ignored _megacomplete _approximate _correct _fasd_zsh_word_complete_trigger
zstyle ':fzf-tab:*' continuous-trigger /
zstyle ':completion:*' delimiters @ / , %
zstyle ':fzf-tab:*' extra-opts
zstyle ':completion:*' extra-verbose on
zstyle ':fzf-tab:*' fake-compadd default
zstyle ':completion:*:descriptions' format $'\C-[[1;31m-<<\C-[[0;34m%d\C-[[1;31m>>-\C-[[0m'
zstyle ':completion:*:corrections' format $'\C-[[1;31m-<<\C-[[0;34m%d (errors: %e)\C-[[1;31m>>-\C-[[0m'
zstyle ':completion:*:messages' format $'\C-[[1;31m-<<\C-[[0;34m%d\C-[[1;31m>>-\C-[[0m'
zstyle ':completion:*:explanations' format $'\C-[[1;31m-<<\C-[[0;34m%d\C-[[1;31m>>-\C-[[0m'
zstyle ':completion:*:warnings' format $'\C-[[1;31m-<<\C-[[0;34mNo Matches for %d\C-[[1;31m>>-\C-[[0m'
zstyle ':completion:*' format $'\C-[[1;31m-<<\C-[[0;34m%d\C-[[1;31m>>-\C-[[0m'
zstyle ':vcs_info:svn*:*' formats %c%u
zstyle ':vcs_info:*' formats %b%c%u%m
zstyle ':vcs_info:hg*:*' get-bookmarks true
zstyle ':vcs_info:hg*:*' get-revision true
zstyle ':vcs_info:*' get-revision false
zstyle :plugin:zconvey greeting none
zstyle ':fzf-tab:*' group-colors $'\C-[[94m' $'\C-[[32m' $'\C-[[33m' $'\C-[[35m' $'\C-[[31m' $'\C-[[38;5;27m' $'\C-[[36m' $'\C-[[38;5;100m' $'\C-[[38;5;98m' $'\C-[[91m' $'\C-[[38;5;80m' $'\C-[[92m' $'\C-[[38;5;214m' $'\C-[[38;5;165m' $'\C-[[38;5;124m' $'\C-[[38;5;120m'
zstyle ':completion:*:*:zinit:*' group-name ''
zstyle ':completion:*' group-name ''
zstyle ':completion:*:*:*:*:(zsh-learn-Zsh-learn-id|zsh-learn-Zsh-learn-text)' group-order zsh-learn-Zsh-learn-id zsh-learn-Zsh-learn-text
zstyle ':completion:*:*:(zpwr-z|zpwr-gitzfordir|zpwr-gitzfordirmain|zpwr-gitzfordirdevelop|zm|zd|zg|z|zs):*:*' group-order options argument-rest globbed-files fasd-file fasd zdir files last-ten
zstyle ':completion:*' group-order zpwr-regen zpwr-clean zpwr-travis zpwr-learn zpwr-search zpwr-env zpwr-update zpwr-cd zpwr-clipboard zpwr-emacs zpwr-vim zpwr-github zpwr-gitrepos zpwr-git zpwr-misc zpwr-send zpwr-forgit zpwr-log zpwr-diag zpwr-monitor c options commands aliases alias global-aliases suffix-aliases functions builtins reserved-words parameters argument-rest strings identifiers hosts commits heads commit-tags heads-local heads-remote recent-branches tags commit-objects remote-branch-names-noprefix corrections packages npm-search npm-cache remote-crate remote-gem remote-pip original globbed-files files fasd-file fasd zdir local-directories tmux contexts last-ten last-clip
zstyle :history-search-multi-word highlight-color 'bg=17'
zstyle ':vcs_info:git*+set-message:*' hooks vcs-detect-changes git-untracked git-aheadbehind git-stash git-remotebranch git-tagname
zstyle ':vcs_info:hg*+set-message:*' hooks vcs-detect-changes
zstyle ':vcs_info:svn*+set-message:*' hooks vcs-detect-changes svn-detect-changes
zstyle ':vcs_info:hg*+gen-hg-bookmark-string:*' hooks hg-bookmarks
zstyle ':fzf-tab:*' ignore false
zstyle ':completion::*:(git-add|git-rm|less|rm|vi|vim|v):*' ignore-line on
zstyle ':completion:*' ignore-parents parent pwd
zstyle ':completion:*:files' ignored-patterns '*.'
zstyle ':completion:*' insert-sections on
zstyle ':fzf-tab:*' insert-space true
zstyle ':completion:*:correct:*' insert-unambiguous on
zstyle ':completion:*' list-prompt $'\C-[[1;31m-<<\C-[[0;34m%SAt %s\C-[[44;37m%M%p\C-[[0;34m%S, Hit TAB for more, or the characters to insert%s\C-[[0;1;31m>>-\C-[[0m'
zstyle ':completion:*' list-separator '<<)(>>'
zstyle :prezto:module:completion loaded 1
zstyle :plugin:zconvey ls_after_rename 0
zstyle ':completion:*:zinit:argument-rest:plugins' matcher 'r:|=** l:|=*'
zstyle ':completion:*' matcher-list '' 'm:{a-z\-}={A-Z\_}' 'r:[^[:alpha:]]||[[:alpha:]]=** r:|=* m:{a-z\-}={A-Z\_}' 'r:|?=** m:{a-z\-}={A-Z\_}'
zstyle ':completion:*:*:*:*:*' menu 'select=0' interactive
zstyle ':completion:fasd-complete:*' menu-select
zstyle ':completion:fasd-complete-f:*' menu-select
zstyle ':completion:fasd-complete-d:*' menu-select
zstyle ':fzf-tab:*' no-group-color $'\C-[[37m'
zstyle :plugin:zconvey output_method feeder
zstyle ':fzf-tab:*' prefix
zstyle ':fzf-tab:*' print-query alt-enter
zstyle ':fzf-tab:*' query-string prefix input first
zstyle ':completion:*' select-prompt $'\C-[[1;31m-<<\C-[[0;34m%SScrolling active: current selection at %s\C-[[37;44m%p\C-[[0;1;31m>>-\C-[[0m'
zstyle ':completion:*:manuals' separate-sections on
zstyle ':fzf-tab:*' show-group full
zstyle ':fzf-tab:*' single-group color header
zstyle ':completion:*:*:zpwr-gitedittag:*:*:commit-tags' sort off
zstyle ':completion:*:*:*:*:*:zdir' sort off
zstyle ':completion:*:*:*:*:*:fasd' sort off
zstyle :completion::megacomplete:zpwr-gitedittag::commit-tags sort off
zstyle ':completion:*:*:(se|see|seee|zsh-learn-Redo|rsql|re|zsh-learn-Searchl|zsh-learn-Searchle|zsh-learn-Searchlee|z|r|zsh-learn-Zsh-learn-get):*:*' sort false
zstyle ':completion:*:*:(zpwr-se|zpwr-see|zpwr-seee|zpwr-redo|zpwr-rsql|zpwr-re|zpwr-searchl|zpwr-searchle|zpwr-searchlee|zpwr-r|zpwr-get):*:*' sort false
zstyle ':completion:*:*:(zpwr-z|zpwr-gitzfordir|zpwr-gitzfordirmain|zpwr-gitzfordirdevelop|zm|zd|zg):*:*' sort off
zstyle ':completion:*' squeeze-slashes on
zstyle ':vcs_info:*' stagedstr ' '
zstyle ':completion:*:*:-subscript-:*' tag-order indexes parameters
zstyle :plugin:fast-syntax-highlighting theme default
zstyle zle-hook types isearch-exit isearch-update line-pre-redraw line-init line-finish history-line-set keymap-select
zstyle ':vcs_info:*' unstagedstr ' '
zstyle ':completion:*' use-cache on
zstyle :plugin:zconvey use_zsystem_flock 1
zstyle zle-line-init widgets 0:user:_zsh_highlight_widget_orig-s000-r58-zle-line-init 1:.hist.format.hook
zstyle zle-line-finish widgets 0:user:_zsh_highlight_widget_orig-s000-r58-zle-line-finish 1:.hist.format.hook
