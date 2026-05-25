//! Port of `_main_complete` from `Completion/Base/Core/_main_complete`.
//!
//! Full upstream body (418 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # The main loop of the completion code. This is what is called when
//! sh:  4  # completion is attempted from the command line.
//! sh:  5
//! sh:  6  # Note that this function is parsed before $_comp_setup is evaluated,
//! sh:  7  # so that it should make conservative assumptions about the setting
//! sh:  8  # of the various options that affect parsing.
//! sh:  9
//! sh: 10  # In case non-standard separators are in use.
//! sh: 11  local IFS=$' \t\n\0'
//! sh: 12
//! sh: 13  # If you want to complete only set or unset options for the unsetopt
//! sh: 14  # and setopt builtin, un-comment these lines:
//! sh: 15  #
//! sh: 16  #   local _options_set _options_unset
//! sh: 17  #
//! sh: 18  #   _options_set=(${(k)options[(R)on]})
//! sh: 19  #   _options_unset=(${(k)options[(R)off]})
//! sh: 20  #
//! sh: 21  # This is needed because completion functions may set options locally
//! sh: 22  # which makes the output of setopt and unsetopt reflect a different
//! sh: 23  # state than the global one for which you are completing.
//! sh: 24
//! sh: 25  eval "$_comp_setup"
//! sh: 26
//! sh: 27  local func funcs ret=1 tmp _compskip format nm call match min max i num\
//! sh: 28        _completers _completer _completer_num curtag _comp_force_list \
//! sh: 29        _matchers _matcher _c_matcher _matcher_num _comp_tags _comp_mesg  \
//! sh: 30        mesg str context state state_descr line opt_args val_args \
//! sh: 31        curcontext="$curcontext" \
//! sh: 32        _last_nmatches=-1 _last_menu_style _def_menu_style _menu_style sel \
//! sh: 33        _tags_level=0 \
//! sh: 34        _saved_exact="${compstate[exact]}" \
//! sh: 35        _saved_lastprompt="${compstate[last_prompt]}" \
//! sh: 36        _saved_list="${compstate[list]}" \
//! sh: 37        _saved_insert="${compstate[insert]}" \
//! sh: 38        _saved_colors="$ZLS_COLORS" \
//! sh: 39        _saved_colors_set=${+ZLS_COLORS} \
//! sh: 40        _ambiguous_color=''
//! sh: 41  # Hide any '_comp_priv_prefix' variable that happens to be defined in the calling scope.
//! sh: 42  local _comp_priv_prefix
//! sh: 43  unset _comp_priv_prefix
//! sh: 44
//! sh: 45  # _precommand sets this to indicate we are following a precommand modifier
//! sh: 46  local -a precommands
//! sh: 47
//! sh: 48  # Precommands which allow their wrapped command to be a builtin.
//! sh: 49  # All of these are necessarily builtins or reserved words themselves,
//! sh: 50  # but not all builtin precommands are listed here:
//! sh: 51  # for one, the 'command' builtin is excluded.
//! sh: 52  local -ar builtin_precommands=(- builtin eval exec nocorrect noglob time)
//! sh: 53
//! sh: 54  typeset -U _lastdescr _comp_ignore _comp_colors
//! sh: 55
//! sh: 56  {
//! sh: 57
//! sh: 58  [[ -z "$curcontext" ]] && curcontext=:::
//! sh: 59
//! sh: 60  zstyle -s ":completion:${curcontext}:" insert-tab tmp || tmp=yes
//! sh: 61
//! sh: 62  if [[ ( "$tmp" = *pending(|[[:blank:]]*) && PENDING -gt 0 ) ||
//! sh: 63        ( "$tmp" = *pending=(#b)([0-9]##)(|[[:blank:]]*) &&
//! sh: 64          PENDING -ge $match[1] ) ]]; then
//! sh: 65    compstate[insert]=tab
//! sh: 66
//! sh: 67    return 0
//! sh: 68  fi
//! sh: 69
//! sh: 70  if [[ "$compstate[insert]" = tab* ]]; then
//! sh: 71    if [[ "$tmp" = (|*[[:blank:]])(yes|true|on|1)(|[[:blank:]]*) ]]; then
//! sh: 72      if [[ "$curcontext" != :* || -z "$compstate[vared]" ]] ||
//! sh: 73        zstyle -t ":completion:vared${curcontext}:" insert-tab; then
//! sh: 74        return 0
//! sh: 75      fi
//! sh: 76    fi
//! sh: 77
//! sh: 78    compstate[insert]="${compstate[insert]//tab /}"
//! sh: 79  fi
//! sh: 80
//! sh: 81  # Second attempt at GLOB_COMPLETE
//! sh: 82
//! sh: 83  if [[ "$compstate[pattern_match]" = "*" &&
//! sh: 84        "$_lastcomp[unambiguous]" = "$PREFIX" &&
//! sh: 85        -n "$_lastcomp[unambiguous_cursor]" ]]; then
//! sh: 86    integer upos="$_lastcomp[unambiguous_cursor]"
//! sh: 87    SUFFIX="$PREFIX[upos,-1]$SUFFIX"
//! sh: 88    PREFIX="$PREFIX[1,upos-1]"
//! sh: 89  fi
//! sh: 90
//! sh: 91  # Special completion contexts after `~' and `='.
//! sh: 92
//! sh: 93  if [[ -z "$compstate[quote]" ]]; then
//! sh: 94    if [[ -o equals ]] && compset -P 1 '='; then
//! sh: 95      compstate[context]=equal
//! sh: 96    elif [[ "$PREFIX" = \~\[[^]]# ]]; then
//! sh: 97      # Inside ~[...] should be treated as a subscript.
//! sh: 98      compset -p 2
//! sh: 99      # To be consistent, we ignore all but the contents of the square brackets.
//! sh:100      compset -S '\]*'
//! sh:101      compstate[context]=subscript
//! sh:102    elif [[ "$PREFIX" = \~[^/]# ]]; then
//! sh:103      compset -p 1
//! sh:104      compstate[context]=tilde
//! sh:105    fi
//! sh:106  fi
//! sh:107
//! sh:108  # Initial setup.
//! sh:109
//! sh:110  _setup default
//! sh:111  _def_menu_style=( "$_last_menu_style[@]"
//! sh:112
//! sh:113  # We can't really do that because the current value of $MENUSELECT
//! sh:114  # may be the one set by this function.
//! sh:115  # There is a similar problem with $ZLS_COLORS in _setup.
//! sh:116
//! sh:117  #                  ${MENUSELECT+select${MENUSELECT:+\=$MENUSELECT}}
//! sh:118
//! sh:119                  )
//! sh:120  _last_menu_style=()
//! sh:121
//! sh:122  if zstyle -s ":completion:${curcontext}:default" list-prompt tmp; then
//! sh:123    LISTPROMPT="$tmp"
//! sh:124    zmodload -i zsh/complist
//! sh:125  fi
//! sh:126  if zstyle -s ":completion:${curcontext}:default" select-prompt tmp; then
//! sh:127    MENUPROMPT="$tmp"
//! sh:128    zmodload -i zsh/complist
//! sh:129  fi
//! sh:130  if zstyle -s ":completion:${curcontext}:default" select-scroll tmp; then
//! sh:131    MENUSCROLL="$tmp"
//! sh:132    zmodload -i zsh/complist
//! sh:133  fi
//! sh:134
//! sh:135  # Get the names of the completers to use in the positional parameters.
//! sh:136
//! sh:137  if (( $# )); then
//! sh:138    if [[ "$1" = - ]]; then
//! sh:139      if [[ $# -lt 3 ]]; then
//! sh:140        _completers=()
//! sh:141      else
//! sh:142        _completers=( "$2" )
//! sh:143        call=yes
//! sh:144      fi
//! sh:145    else
//! sh:146      _completers=( "$@" )
//! sh:147    fi
//! sh:148  else
//! sh:149    zstyle -a ":completion:${curcontext}:" completer _completers ||
//! sh:150        _completers=( _complete _ignored )
//! sh:151  fi
//! sh:152
//! sh:153  # And now just call the completer functions defined.
//! sh:154
//! sh:155  _completer_num=1
//! sh:156
//! sh:157  # We assume localtraps to be in effect here ...
//! sh:158  integer SECONDS=0
//! sh:159  TRAPINT() {
//! sh:160    zle -M "Killed by signal in ${funcstack[2]} after ${SECONDS}s";
//! sh:161    zle -R
//! sh:162    return 130
//! sh:163  }
//! sh:164  TRAPQUIT() {
//! sh:165    zle -M "Killed by signal in ${funcstack[2]} after ${SECONDS}s";
//! sh:166    zle -R
//! sh:167    return 131
//! sh:168  }
//! sh:169
//! sh:170  # Call the pre-functions.
//! sh:171
//! sh:172  funcs=( "$compprefuncs[@]" )
//! sh:173  compprefuncs=()
//! sh:174  for func in "$funcs[@]"; do
//! sh:175    "$func"
//! sh:176  done
//! sh:177
//! sh:178  for tmp in "$_completers[@]"; do
//! sh:179
//! sh:180    if [[ -n "$call" ]]; then
//! sh:181      _completer="${tmp}"
//! sh:182    elif [[ "$tmp" = *:-* ]]; then
//! sh:183      _completer="${${tmp%:*}[2,-1]//_/-}${tmp#*:}"
//! sh:184      tmp="${tmp%:*}"
//! sh:185    elif [[ $tmp = *:* ]]; then
//! sh:186      _completer="${tmp#*:}"
//! sh:187      tmp="${tmp%:*}"
//! sh:188    else
//! sh:189      _completer="${tmp[2,-1]//_/-}"
//! sh:190    fi
//! sh:191
//! sh:192    curcontext="${curcontext/:[^:]#:/:${_completer}:}"
//! sh:193    zstyle -t ":completion:${curcontext}:" show-completer &&
//! sh:194      zle -R "Trying completion for :completion:${curcontext}"
//! sh:195
//! sh:196    zstyle -a ":completion:${curcontext}:" matcher-list _matchers ||
//! sh:197        _matchers=( '' )
//! sh:198
//! sh:199    _matcher_num=1
//! sh:200    _matcher=''
//! sh:201    for _c_matcher in "$_matchers[@]"; do
//! sh:202      if [[ "$_c_matcher" == +* ]]; then
//! sh:203        _matcher="$_matcher $_c_matcher[2,-1]"
//! sh:204      else
//! sh:205        _matcher="$_c_matcher"
//! sh:206      fi
//! sh:207
//! sh:208      _comp_mesg=
//! sh:209      if [[ -n "$call" ]]; then
//! sh:210        if "${(@)argv[3,-1]}"; then
//! sh:211          ret=0
//! sh:212          break 2
//! sh:213        fi
//! sh:214      elif "$tmp"; then
//! sh:215        ret=0
//! sh:216        break 2
//! sh:217      fi
//! sh:218      (( _matcher_num++ ))
//! sh:219    done
//! sh:220    [[ -n "$_comp_mesg" ]] && break
//! sh:221
//! sh:222    (( _completer_num++ ))
//! sh:223  done
//! sh:224
//! sh:225  curcontext="${curcontext/:[^:]#:/::}"
//! sh:226  if [[ $compstate[old_list] = keep ]]; then
//! sh:227    # We are keeping the old list of matches, so keep the
//! sh:228    # number of matches we found last time rather than the
//! sh:229    # number just generated.
//! sh:230    nm=$_lastcomp[nmatches]
//! sh:231  else
//! sh:232    nm=$compstate[nmatches]
//! sh:233  fi
//! sh:234
//! sh:235  if [[ $compstate[old_list] = keep || nm -gt 1 ]]; then
//! sh:236    [[ _last_nmatches -ge 0 && _last_nmatches -ne nm ]] &&
//! sh:237        _menu_style=( "$_last_menu_style[@]" "$_menu_style[@]" )
//! sh:238
//! sh:239    tmp=$(( compstate[list_lines] + BUFFERLINES + 1 ))
//! sh:240
//! sh:241    _menu_style=( "$_menu_style[@]" "$_def_menu_style[@]" )
//! sh:242
//! sh:243    if [[ "$compstate[list]" = *list(| *) && tmp -gt LINES &&
//! sh:244          ( -n "$_menu_style[(r)select=long-list]" ||
//! sh:245            -n "$_menu_style[(r)(yes|true|on|1)=long-list]" ) ]]; then
//! sh:246      compstate[insert]=menu
//! sh:247    elif [[ "$compstate[insert]" = "$_saved_insert" ]]; then
//! sh:248      if [[ -n "$compstate[insert]" &&
//! sh:249            -n "$_menu_style[(r)(yes|true|1|on)=long]" && tmp -gt LINES ]]; then
//! sh:250          compstate[insert]=menu
//! sh:251      else
//! sh:252        sel=( "${(@M)_menu_style:#(yes|true|1|on)*}" )
//! sh:253
//! sh:254        if (( $#sel )); then
//! sh:255  	min=9999999
//! sh:256          for i in "$sel[@]"; do
//! sh:257            if [[ "$i" = *\=[0-9]* ]]; then
//! sh:258    	    num="${i#*\=}"
//! sh:259    	    [[ num -lt 0 ]] && num=0
//! sh:260    	  elif [[ "$i" != *\=* ]]; then
//! sh:261    	    num=0
//! sh:262            else
//! sh:263  	    num=9999999
//! sh:264    	  fi
//! sh:265    	  [[ num -lt min ]] && min="$num"
//! sh:266
//! sh:267  	  (( min )) || break
//! sh:268          done
//! sh:269        fi
//! sh:270        sel=( "${(@M)_menu_style:#(no|false|0|off)*}" )
//! sh:271
//! sh:272        if (( $#sel )); then
//! sh:273  	max=9999999
//! sh:274          for i in "$sel[@]"; do
//! sh:275            if [[ "$i" = *\=[0-9]* ]]; then
//! sh:276    	    num="${i#*\=}"
//! sh:277    	    [[ num -lt 0 ]] && num=0
//! sh:278            elif [[ "$i" != *\=* ]]; then
//! sh:279    	    num=0
//! sh:280    	  else
//! sh:281    	    num=9999999
//! sh:282    	  fi
//! sh:283    	  [[ num -lt max ]] && max="$num"
//! sh:284
//! sh:285  	  (( max )) || break
//! sh:286          done
//! sh:287        fi
//! sh:288        if [[ ( -n "$min" && nm -ge min && ( -z "$max" || nm -lt max ) ) ||
//! sh:289              ( -n "$_menu_style[(r)auto*]" &&
//! sh:290                "$compstate[insert]" = automenu ) ]]; then
//! sh:291          compstate[insert]=menu
//! sh:292        elif [[ -n "$max" && nm -ge max ]]; then
//! sh:293          compstate[insert]=unambiguous
//! sh:294        elif [[ -n "$_menu_style[(r)auto*]" &&
//! sh:295                "$compstate[insert]" != automenu ]]; then
//! sh:296          compstate[insert]=automenu-unambiguous
//! sh:297        fi
//! sh:298      fi
//! sh:299    fi
//! sh:300
//! sh:301    if [[ "$compstate[insert]" = *menu* ]]; then
//! sh:302      [[ "$MENUSELECT" = 00 ]] && MENUSELECT=0
//! sh:303      if [[ -n "$_menu_style[(r)no-select*]" ]]; then
//! sh:304        unset MENUSELECT
//! sh:305      elif [[ -n "$_menu_style[(r)select=long*]" ]]; then
//! sh:306        if [[ tmp -gt LINES ]]; then
//! sh:307          zmodload -i zsh/complist
//! sh:308          MENUSELECT=00
//! sh:309        fi
//! sh:310      fi
//! sh:311      if [[ "$MENUSELECT" != 00 ]]; then
//! sh:312        sel=( "${(@M)_menu_style:#select*}" )
//! sh:313
//! sh:314        if (( $#sel )); then
//! sh:315  	min=9999999
//! sh:316          for i in "$sel[@]"; do
//! sh:317            if [[ "$i" = *\=[0-9]* ]]; then
//! sh:318    	    num="${i#*\=}"
//! sh:319    	    [[ num -lt 0 ]] && num=0
//! sh:320    	  elif [[ "$i" != *\=* ]]; then
//! sh:321    	    num=0
//! sh:322            else
//! sh:323  	    num=9999999
//! sh:324    	  fi
//! sh:325    	  [[ num -lt min ]] && min="$num"
//! sh:326
//! sh:327  	  (( min )) || break
//! sh:328          done
//! sh:329
//! sh:330          zmodload -i zsh/complist
//! sh:331          MENUSELECT="$min"
//! sh:332        else
//! sh:333          unset MENUSELECT
//! sh:334        fi
//! sh:335      fi
//! sh:336      if [[ -n "$MENUSELECT" ]]; then
//! sh:337        if [[ -n "$_menu_style[(r)interactive*]" ]]; then
//! sh:338          MENUMODE=interactive
//! sh:339        elif [[ -n "$_menu_style[(r)search*]" ]]; then
//! sh:340          if [[ -n "$_menu_style[(r)*backward*]" ]]; then
//! sh:341            MENUMODE=search-backward
//! sh:342          else
//! sh:343            MENUMODE=search-forward
//! sh:344          fi
//! sh:345        else
//! sh:346          unset MENUMODE
//! sh:347        fi
//! sh:348      fi
//! sh:349    fi
//! sh:350  elif [[ nm -lt 1 && -n "$_comp_mesg" ]]; then
//! sh:351    compstate[insert]=''
//! sh:352    compstate[list]='list force'
//! sh:353  elif [[ nm -eq 0 && -z "$_comp_mesg" &&
//! sh:354          $#_lastdescr -ne 0 && $compstate[old_list] != keep ]] &&
//! sh:355       zstyle -s ":completion:${curcontext}:warnings" format format; then
//! sh:356
//! sh:357    compstate[list]='list force'
//! sh:358    compstate[insert]=''
//! sh:359
//! sh:360    tmp=( "\`${(@)^_lastdescr:#}'" )
//! sh:361
//! sh:362    case $#tmp in
//! sh:363    1) str="$tmp[1]";;
//! sh:364    2) str="$tmp[1] or $tmp[2]";;
//! sh:365    *) str="${(j:, :)tmp[1,-2]}, or $tmp[-1]";;
//! sh:366    esac
//! sh:367
//! sh:368    _setup warnings
//! sh:369    zformat -f mesg "$format" "d:$str" "D:${(F)${(@)_lastdescr:#}}"
//! sh:370    compadd -x "$mesg"
//! sh:371  fi
//! sh:372
//! sh:373  if [[ -n "$_ambiguous_color" ]]; then
//! sh:374    local toquote='[=\(\)\|~^?*[\]#<>]'
//! sh:375    local prefix=${${compstate[unambiguous]}[1,${compstate[unambiguous_cursor]}-1]}
//! sh:376    [[ -n $prefix ]] &&
//! sh:377      _comp_colors+=( "=(#i)${prefix[1,-2]//?/(}${prefix[1,-2]//(#m)?/${MATCH/$~toquote/\\$MATCH}|)}${prefix[-1]//(#m)$~toquote/\\$MATCH}(#b)(?|)*==$_ambiguous_color" )
//! sh:378  fi
//! sh:379
//! sh:380  [[ "$_comp_force_list" = always ||
//! sh:381     ( "$_comp_force_list" = ?*  && nm -ge _comp_force_list ) ]] &&
//! sh:382      compstate[list]="${compstate[list]//messages} force"
//! sh:383
//! sh:384  } always {
//! sh:385    # Stuff we always do to clean up.
//! sh:386    if [[ "$compstate[old_list]" = keep ]]; then
//! sh:387      if [[ $_saved_colors_set = 1 ]]; then
//! sh:388        ZLS_COLORS="$_saved_colors"
//! sh:389      else
//! sh:390        unset ZLS_COLORS
//! sh:391      fi
//! sh:392    elif (( $#_comp_colors )); then
//! sh:393      ZLS_COLORS="${(j.:.)_comp_colors}"
//! sh:394    else
//! sh:395      unset ZLS_COLORS
//! sh:396    fi
//! sh:397  }
//! sh:398
//! sh:399  # Now call the post-functions.
//! sh:400
//! sh:401  funcs=( "$comppostfuncs[@]" )
//! sh:402  comppostfuncs=()
//! sh:403  for func in "$funcs[@]"; do
//! sh:404    "$func"
//! sh:405  done
//! sh:406
//! sh:407  _lastcomp=( "${(@kv)compstate}" )
//! sh:408  _lastcomp[nmatches]=$nm
//! sh:409  _lastcomp[completer]="$_completer"
//! sh:410  _lastcomp[prefix]="$PREFIX"
//! sh:411  _lastcomp[suffix]="$SUFFIX"
//! sh:412  _lastcomp[iprefix]="$IPREFIX"
//! sh:413  _lastcomp[isuffix]="$ISUFFIX"
//! sh:414  _lastcomp[qiprefix]="$QIPREFIX"
//! sh:415  _lastcomp[qisuffix]="$QISUFFIX"
//! sh:416  _lastcomp[tags]="$_comp_tags"
//! sh:417
//! sh:418  return ret
//! ```
//!
//! Faithful Rust port: walks the configured completer list via a
//! caller-supplied `dispatch` closure. The `pre_funcs` /
//! `post_funcs` shell-side hooks are exposed via
//! `state.prefuncs` / `state.postfuncs` so the caller can register
//! their own. Records `lastcomp[nmatches/completer/prefix/suffix]`
//! after every run — same as shell's `_lastcomp` association.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
