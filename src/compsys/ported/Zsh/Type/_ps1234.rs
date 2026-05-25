//! Port of `_ps1234` from `Completion/Zsh/Type/_ps1234`.
//!
//! Full upstream body (179 lines verbatim):
//! ```text
//! sh:  1  #compdef -value-,PROMPT,-default- -value-,PROMPT2,-default- -value-,PROMPT3,-default- -value-,PROMPT4,-default- -value-,RPROMPT,-default- -value-,RPROMPT2,-default- -value-,PS1,-default- -value-,PS2,-default- -value-,PS3,-default- -value-,PS4,-default- -value-,RPS1,-default- -value-,RPS2,-default- -value-,SPROMPT,-default- -value-,PROMPT_EOL_MARK,-default-
//! sh:  2
//! sh:  3  local -a specs ccol suf
//! sh:  4  local expl grp cols bs pre changed=1 ret=1
//! sh:  5  local -A ansi
//! sh:  6
//! sh:  7  [[ -z $compstate[quote] ]] && bs='\'
//! sh:  8  suf=( -S '' )
//! sh:  9
//! sh: 10  # first strip off any complete prompt specifications leaving only the
//! sh: 11  # current, incomplete, one
//! sh: 12  while (( changed )); do
//! sh: 13    changed=0
//! sh: 14    compset -P '%[DFHK](\\|){[^}]#}' && changed=1 # formats with arg: %x{...}
//! sh: 15    compset -P '%[0-9-\\]#[^DFHK(0-9-<>\\\[]' && changed=1 # normal formats
//! sh: 16    compset -P '%[0-9-\\]#(<[^<]#<|>[^>]#>|\[[^\]]#\])' && changed=1 # truncations
//! sh: 17    compset -P '%[0-9-\\]#(\\|)\([0-9-]#[^0-9]?|[^%]' && changed=1 # start of ternary
//! sh: 18    compset -P '[^%]##' && changed=1 # sundry other characters
//! sh: 19    # %D/%F/%K without a following { ... }
//! sh: 20    [[ $PREFIX = %(-|)<->#[DFK](\\[^{]|[^{\\])* ]] &&
//! sh: 21        compset -P '%[0-9\\-]#[DFK]' && changed=1
//! sh: 22  done
//! sh: 23  [[ $PREFIX = %(-|)<->[FK](#e) ]] && compset -P '*' # F/K with number
//! sh: 24
//! sh: 25  if compset -P '%[FK]'; then
//! sh: 26    # this should use -P but that somehow causes single quotes to be stripped
//! sh: 27    compset -P '(\\|){' || pre=( -p '{' )
//! sh: 28    compset -S '(\\|)}*' || suf=( -S "$bs}" )
//! sh: 29    ansi=(
//! sh: 30      black 30
//! sh: 31      red 31
//! sh: 32      green 32
//! sh: 33      yellow 33
//! sh: 34      blue 34
//! sh: 35      magenta 35
//! sh: 36      cyan 36
//! sh: 37      white 37
//! sh: 38      default 39
//! sh: 39    )
//! sh: 40
//! sh: 41    _description -V ansi-colors expl 'ansi color'
//! sh: 42    grp="$expl[expl[(i)-J]+1]"
//! sh: 43    print -v ccol -f "($grp)=%s=%s" ${(kv)ansi}
//! sh: 44    _comp_colors+=( $ccol )
//! sh: 45    compadd "$expl[@]" "$suf[@]" $pre -k ansi && ret=0
//! sh: 46    if [[ $ISUFFIX != (\\|)}* ]] && compset -P "(<->|%v)"; then
//! sh: 47      _wanted ansi-colors expl 'closing brace' compadd -S '' \} && ret=0
//! sh: 48    elif (( $+terminfo[colors] )); then
//! sh: 49      (( cols = $terminfo[colors] - 1 ))
//! sh: 50      (( cols = cols > 255 ? 255 : cols ))
//! sh: 51      _description -V terminal-colors expl 'terminal color'
//! sh: 52      grp="$expl[expl[(i)-J]+1]"
//! sh: 53      compadd "$expl[@]" "$suf[@]" $pre {0..$cols}
//! sh: 54      for c in {0..$cols}; do
//! sh: 55        _comp_colors+=( "($grp)=${c}=${${${${(%):-%F{$c\}}#?\[}%m}//:/;}" )
//! sh: 56      done
//! sh: 57    else
//! sh: 58      _message -e terminal-colors "number"
//! sh: 59    fi
//! sh: 60  fi
//! sh: 61
//! sh: 62  if compset -P '%[0-9-\\]#(\\|)\([0-9-]#[^0-9]'; then
//! sh: 63    # ternary conditional: first delimiter
//! sh: 64    compset -S '*'
//! sh: 65    _delimiters && ret=0
//! sh: 66  elif compset -P '%[0-9-\\]#[<>\]]'; then
//! sh: 67    # truncation
//! sh: 68    _message -e replacements 'replacement string'
//! sh: 69  elif compset -P '%[0-9-\\]#(\\|)\([0-9-]#'; then
//! sh: 70    # ternary conditional: condition character
//! sh: 71    compset -S '[.:+/-%]*' || suf=( -S . )
//! sh: 72    compset -S '*'
//! sh: 73    specs=(
//! sh: 74      '!:running with privileges'
//! sh: 75      '#:effective uid'
//! sh: 76      '?:exit status'
//! sh: 77      '_:at least n shell constructs started'
//! sh: 78      'C:at least n path elements'
//! sh: 79      '/:at least n path elements'
//! sh: 80      '.:at least n path elements'
//! sh: 81      'c:at least n path elements'
//! sh: 82      '~:at least n path elements'
//! sh: 83      'D:month'
//! sh: 84      'd:day of month'
//! sh: 85      'g:effective gid'
//! sh: 86      'j:number of jobs'
//! sh: 87      'L:SHLVL'
//! sh: 88      'l:number of characters already printed'
//! sh: 89      'S:SECONDS parameter at least n'
//! sh: 90      'T:current hour'
//! sh: 91      't:current minute'
//! sh: 92      'v:psvar has at least n elements'
//! sh: 93      'V:element n of psvar is set and non-empty'
//! sh: 94      'w:day of week (Sunday = 0)'
//! sh: 95    )
//! sh: 96    [[ $IPREFIX != *- ]] && _describe -t ternary-prompt-expressions \
//! sh: 97        'ternary prompt format test character' specs "$suf[@]" && ret=0
//! sh: 98    _message -e numbers number
//! sh: 99  elif compset -P '%D(\\|){'; then
//! sh:100    compset -S '(\\|)}*'
//! sh:101    _date_formats zsh && ret=0
//! sh:102  elif compset -P '%H(\\|){'; then
//! sh:103    compset -S '(\\|)}*' || suf=( -S "$bs}" )
//! sh:104    _wanted highlight-groups expl 'highlight group' compadd "$suf[@]" -k .zle.hlgroups && ret=0
//! sh:105  elif [[ -prefix '%' ]] ||
//! sh:106        ! zstyle -t ":completion:${curcontext}:prompt-format-specifiers" prefix-needed
//! sh:107  then
//! sh:108    specs=(
//! sh:109      'm:hostname up to first .'
//! sh:110      '_:status of parser'
//! sh:111      '^:reversed status of parser'
//! sh:112      'd:current working directory'
//! sh:113      '/:current working directory'
//! sh:114      '~:current working directory, with ~ replacement'
//! sh:115      'N:name of current script or shell function'
//! sh:116      'x:name of file containing code being executed'
//! sh:117      'c:deprecated'
//! sh:118      '.:deprecated'
//! sh:119      'C:deprecated'
//! sh:120      'F:start using fg color'
//! sh:121      'K:start using bg color'
//! sh:122      'G:counts as extra character inside %{...%}'
//! sh:123      '(:ternary expression %(x.true-string.false-string)'
//! sh:124    )
//! sh:125    compset -P '%' || pre=( -p '%' )
//! sh:126    if ! compset -P '(-|)<->'; then
//! sh:127      if [[ $service == -value-,SPROMPT,* ]]; then
//! sh:128        specs+=(
//! sh:129  	'r:suggested correction'
//! sh:130  	'R:corrected string'
//! sh:131        )
//! sh:132      fi
//! sh:133      specs+=(
//! sh:134        '%:A %'
//! sh:135        '):A )'
//! sh:136        'l:current line (tty) with /dev/tty stripped'
//! sh:137        'M:full hostname'
//! sh:138        'n:username'
//! sh:139        'y:current line (tty)'
//! sh:140        '#:a # when root, % otherwise'
//! sh:141        '?:return status of last command'
//! sh:142        'h:current history event number'
//! sh:143        '!:current history event number'
//! sh:144        'i:current line number'
//! sh:145        'I:current source line number'
//! sh:146        'j:number of jobs'
//! sh:147        'L:$SHLVL'
//! sh:148        'D:date in yy-mm-dd format'
//! sh:149        'T:current time of day, 24-hour format'
//! sh:150        't:current time of day, 12-hour am/pm format'
//! sh:151        '@:current time of day, 12-hour am/pm format'
//! sh:152        '*:current time of day, 24-hour format with seconds'
//! sh:153        'w:the date in day-dd format'
//! sh:154        'W:the date in mm/dd/yy format'
//! sh:155        'D{:format string like strftime'
//! sh:156        'B:start bold'
//! sh:157        'b:stop bold'
//! sh:158        'E:clear to end of line'
//! sh:159        'H{:use highlight group'
//! sh:160        'U:start underline'
//! sh:161        'u:stop underline'
//! sh:162        'S:start standout'
//! sh:163        's:stop standout'
//! sh:164        'f:reset fg color'
//! sh:165        'k:reset bg color'
//! sh:166        '{:start literal escape sequence'
//! sh:167        '}:stop literal escape sequence'
//! sh:168        'v:value from $psvar array'
//! sh:169        '<:truncation from left %len<string<'
//! sh:170        '>:truncation from right %len>string>'
//! sh:171        '[:truncation from who knows where'
//! sh:172      )
//! sh:173    fi
//! sh:174    _describe -t prompt-format-specifiers 'prompt format specifier' \
//! sh:175        specs -S '' $pre && ret=0
//! sh:176    (( ! $#pre )) && _message -e prompt-format-specifiers number
//! sh:177  fi
//! sh:178
//! sh:179  return ret
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
