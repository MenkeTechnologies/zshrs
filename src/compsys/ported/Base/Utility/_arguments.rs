//! Port of `_arguments` from `Completion/Base/Utility/_arguments`.
//!
//! Full upstream body (589 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # Complete the arguments of the current command according to the
//! sh:  4  # descriptions given as arguments to this function.
//! sh:  5
//! sh:  6  local long cmd="$words[1]" descr odescr mesg subopts opt opt2 usecc autod
//! sh:  7  local oldcontext="$curcontext" hasopts rawret optarg singopt alwopt
//! sh:  8  local setnormarg start rest
//! sh:  9  local -a match mbegin mend
//! sh: 10  integer opt_args_use_NUL_separators=0
//! sh: 11
//! sh: 12  subopts=()
//! sh: 13  singopt=()
//! sh: 14  while [[ "$1" = -([AMO]*|[0CRSWnsw]) ]]; do
//! sh: 15    case "$1" in
//! sh: 16    -0) opt_args_use_NUL_separators=1; shift ;;
//! sh: 17    -C)  usecc=yes; shift ;;
//! sh: 18    -O)  subopts=( "${(@P)2}" ); shift 2 ;;
//! sh: 19    -O*) subopts=( "${(@P)${1[3,-1]}}" ); shift ;;
//! sh: 20    -R)  rawret=yes; shift;;
//! sh: 21    -n)  setnormarg=yes; NORMARG=-1; shift;;
//! sh: 22    -w)  optarg=yes; shift;;
//! sh: 23    -W)  alwopt=arg; shift;;
//! sh: 24    -[Ss])  singopt+=( $1 ); shift;;
//! sh: 25    -[AM])  singopt+=( $1 $2 ); shift 2 ;;
//! sh: 26    -[AM]*) singopt+=( $1 ); shift ;;
//! sh: 27    esac
//! sh: 28  done
//! sh: 29
//! sh: 30  [[ $1 = ':' ]] && shift
//! sh: 31  singopt+=( ':' )  # always end with ':' to indicate the end of options
//! sh: 32
//! sh: 33  [[ "$PREFIX" = [-+] ]] && alwopt=arg
//! sh: 34
//! sh: 35  long=$argv[(I)--]
//! sh: 36  if (( long )); then
//! sh: 37    local name tmp tmpargv
//! sh: 38
//! sh: 39    tmpargv=( "${(@)argv[1,long-1]}" )  # optspec's before --, if any
//! sh: 40
//! sh: 41    name=${~words[1]} 2>/dev/null
//! sh: 42    [[ "$name" = [^/]*/* ]] && name="$PWD/$name"
//! sh: 43
//! sh: 44    name="_args_cache_${name}"
//! sh: 45    name="${name//[^a-zA-Z0-9_]/_}"
//! sh: 46
//! sh: 47    if (( ! ${(P)+name} )); then
//! sh: 48      local iopts sopts lflag pattern tmpo dir cur cache
//! sh: 49      typeset -Ua lopts
//! sh: 50
//! sh: 51      cache=()
//! sh: 52
//! sh: 53      # We have to build a new long-option cache, get the `-i' and
//! sh: 54      # `-s' options.
//! sh: 55
//! sh: 56      set -- "${(@)argv[long+1,-1]}"
//! sh: 57
//! sh: 58      iopts=()
//! sh: 59      sopts=()
//! sh: 60      while [[ "$1" = -[lis]* ]]; do
//! sh: 61        if [[ "$1" = -l ]]; then
//! sh: 62  	lflag='-l'
//! sh: 63  	shift
//! sh: 64  	continue
//! sh: 65        fi
//! sh: 66        if [[ "$1" = -??* ]]; then
//! sh: 67          tmp="${1[3,-1]}"
//! sh: 68          cur=1
//! sh: 69        else
//! sh: 70          tmp="$2"
//! sh: 71  	cur=2
//! sh: 72        fi
//! sh: 73        if [[ "$tmp[1]" = '(' ]]; then
//! sh: 74  	tmp=( ${=tmp[2,-2]} )
//! sh: 75        else
//! sh: 76  	tmp=( "${(@P)tmp}" )
//! sh: 77        fi
//! sh: 78        if [[ "$1" = -i* ]]; then
//! sh: 79          iopts+=( "$tmp[@]" )
//! sh: 80        else
//! sh: 81          sopts+=( "$tmp[@]" )
//! sh: 82        fi
//! sh: 83        shift cur
//! sh: 84      done
//! sh: 85
//! sh: 86      # Now get the long option names by calling the command with `--help'.
//! sh: 87      # The parameter expansion trickery first gets the lines as separate
//! sh: 88      # array elements. Then we select all lines whose first non-blank
//! sh: 89      # character is a hyphen. Since some commands document more than one
//! sh: 90      # option per line, separated by commas, we convert commas into
//! sh: 91      # newlines and then split the result again at newlines after joining
//! sh: 92      # the old array elements with newlines between them. Then we select
//! sh: 93      # those elements that start with two hyphens, remove anything up to
//! sh: 94      # those hyphens and anything from the space or tab after the
//! sh: 95      # option up to the end.
//! sh: 96
//! sh: 97     tmp=()
//! sh: 98     _call_program $lflag options ${~words[1]} --help 2>&1 |
//! sh: 99       while IFS= read -r opt; do
//! sh:100       if (( ${#tmp} )); then
//! sh:101         # Previous line had no comment.  Is the current one suitable?
//! sh:102         # It's hard to be sure, but if it there was nothing on the
//! sh:103         # previous line and the current one is indented more than
//! sh:104         # a couple of spaces (and isn't completely whitespace or punctuation)
//! sh:105         # there's a pretty good chance.
//! sh:106         if [[ $opt = [[:space:]][[:space:]][[:space:]]*[[:alpha:]]* ]]; then
//! sh:107  	 # Assume so.
//! sh:108  	 opt=${opt##[[:space:]]##}
//! sh:109  	 # Same substitution as below.
//! sh:110  	 lopts+=("${^tmp[@]}":${${${opt//:/-}//\[/(}//\]/)})
//! sh:111  	 tmp=()
//! sh:112  	 # Finished with this line.
//! sh:113  	 continue
//! sh:114         else
//! sh:115  	 # Still no comment, add the previous options anyway.
//! sh:116           # Add a ':' after the option anyways, to make the matching of
//! sh:117           # the options lateron work as intended.
//! sh:118           # It will be removed again later.
//! sh:119  	 lopts+=("${^tmp[@]}":)
//! sh:120  	 tmp=()
//! sh:121         fi
//! sh:122       fi
//! sh:123       while [[ $opt = [,[:space:]]#(#b)(-[^,[:space:]]#)(*) ]]; do
//! sh:124         # We used to remove the brackets from "[=STUFF]",
//! sh:125         # but later the code appears to handle it with the brackets
//! sh:126         # present.  Maybe the problem was that the intervening code
//! sh:127         # didn't.  If it's buggy without removing them, the problem
//! sh:128         # probably is later, not here.
//! sh:129         start=${match[1]}
//! sh:130         rest=${match[2]}
//! sh:131         if [[ -z ${tmp[(r)${start%%[^a-zA-Z0-9_-]#}]} ]]; then
//! sh:132  	 # variant syntax seen in fetchmail:
//! sh:133  	 # --[fetch]all  means --fetchall or --all.
//! sh:134  	 # maybe needs to be more general
//! sh:135  	 if [[ $start = (#b)--\[(*)\](*) ]]; then
//! sh:136  	   tmp+=("--${match[1]}${match[2]}" "--${match[2]}")
//! sh:137  	 else
//! sh:138  	   tmp+=($start)
//! sh:139  	 fi
//! sh:140         fi
//! sh:141         opt=$rest
//! sh:142       done
//! sh:143       # If there's left over text, assume it's a description; it
//! sh:144       # may be truncated but if it's too long it's no use anyway.
//! sh:145       # There's one hiccup: we sometimes get descriptions like
//! sh:146       # --foo fooarg   Do some foo stuff with foo arg
//! sh:147       # and we need to remove fooarg.  Use whitespace for hints.
//! sh:148       opt=${opt## [^[:space:]]##  }
//! sh:149       opt=${opt##[[:space:]]##}
//! sh:150       if [[ -n $opt ]]; then
//! sh:151         # Add description after a ":", converting any : in the description
//! sh:152         # to a -.  Use RCQUOTES to append this to all versions of the option.
//! sh:153         lopts+=("${^tmp[@]}":${${${opt//:/-}//\[/(}//\]/)})
//! sh:154         tmp=()
//! sh:155         # If there's no comment, we'll see if there's one on the
//! sh:156         # next line.
//! sh:157       fi
//! sh:158     done
//! sh:159     # Tidy up any remaining uncommented options.
//! sh:160     if (( ${#tmp} )); then
//! sh:161       lopts+=("${^tmp[@]}":)
//! sh:162     fi
//! sh:163
//! sh:164      # Remove options also described by user-defined specs.
//! sh:165
//! sh:166      tmp=()
//! sh:167      # Ignore any argument and description information when searching
//! sh:168      # the long options array here and below.
//! sh:169      for opt in "${(@)${(@)lopts:#--}%%[\[:=]*}"; do
//! sh:170
//! sh:171        # Using (( ... )) gives a parse error.
//! sh:172
//! sh:173        let "$tmpargv[(I)(|\([^\)]#\))(|\*)${opt}(|[-+]|=(|-))(|\[*\])(|:*)]" ||
//! sh:174            tmp+=( "$lopts[(r)$opt(|[\[:=]*)]" )
//! sh:175      done
//! sh:176      lopts=( "$tmp[@]" )
//! sh:177
//! sh:178      # Now remove all ignored options ...
//! sh:179
//! sh:180      while (( $#iopts )); do
//! sh:181        lopts=( ${lopts:#$~iopts[1](|[\[:=]*)} )
//! sh:182        shift iopts
//! sh:183      done
//! sh:184
//! sh:185      # ... and add "same" options
//! sh:186
//! sh:187      while (( $#sopts )); do
//! sh:188        # This implements adding things like --disable-* based
//! sh:189        # on the existence of --enable-*.
//! sh:190        # TODO: there's no anchoring here, is that correct?
//! sh:191        # If it's not, careful with the [\[:=]* stuff.
//! sh:192        lopts+=( ${lopts/$~sopts[1]/$sopts[2]} )
//! sh:193        shift 2 sopts
//! sh:194      done
//! sh:195
//! sh:196      # Then we walk through the descriptions plus a few builtin ones.
//! sh:197      # The last one matches all options; the `special' description and action
//! sh:198      # makes those options be completed without an argument description.
//! sh:199
//! sh:200      argv+=(
//! sh:201        '*=FILE*:file:_files'
//! sh:202        '*=(DIR|PATH)*:directory:_files -/'
//! sh:203        '*=*:=: '
//! sh:204        '*: :  '
//! sh:205      )
//! sh:206
//! sh:207      while (( $# )); do
//! sh:208
//! sh:209        # First, we get the pattern and the action to use and take them
//! sh:210        # from the positional parameters.
//! sh:211
//! sh:212        # This is the first bit of the arguments in the special form
//! sh:213        # for converting --help texts, taking account of any quoting
//! sh:214        # of colons.
//! sh:215        pattern="${${${(M)1#*[^\\]:}[1,-2]}//\\\\:/:}"
//! sh:216        # Any action specifications that go with it.
//! sh:217        descr="${1#${pattern}}"
//! sh:218        if [[ "$pattern" = *\(-\) ]]; then
//! sh:219  	# This is the special form to disallow arguments
//! sh:220  	# in the next word.
//! sh:221          pattern="$pattern[1,-4]"
//! sh:222  	dir=-
//! sh:223        else
//! sh:224          dir=
//! sh:225        fi
//! sh:226        shift
//! sh:227
//! sh:228        # We get all options matching the pattern and take them from the
//! sh:229        # list we have built. If no option matches the pattern, we
//! sh:230        # continue with the next.
//! sh:231
//! sh:232        # Ignore :descriptions at the ends of lopts for matching this;
//! sh:233        # they aren't in the patterns.
//! sh:234        tmp=("${(@M)lopts:##$~pattern:*}")
//! sh:235        lopts=("${(@)lopts:##$~pattern:*}")
//! sh:236
//! sh:237        (( $#tmp )) || continue
//! sh:238
//! sh:239        opt=''
//! sh:240
//! sh:241        # Clean suffix ':' added earlier
//! sh:242        tmp=("${(@)tmp%:}")
//! sh:243
//! sh:244        # If there are option strings with a `[=', we take these to get an
//! sh:245        # optional argument.
//! sh:246
//! sh:247        tmpo=("${(@M)tmp:#[^:]##\[\=*}")
//! sh:248        if (( $#tmpo )); then
//! sh:249          tmp=("${(@)tmp:#[^:]##\[\=*}")
//! sh:250
//! sh:251  	for opt in "$tmpo[@]"; do
//! sh:252  	  # Look for --option:description and turn it into
//! sh:253  	  # --option[description].  We didn't do that above
//! sh:254  	  # since it could get confused with the [=ARG] stuff.
//! sh:255  	  if [[ $opt = (#b)(*):([^:]#) ]]; then
//! sh:256  	    opt=$match[1]
//! sh:257  	    odescr="[${match[2]}]"
//! sh:258  	  else
//! sh:259  	    odescr=
//! sh:260  	  fi
//! sh:261  	  if [[ $opt = (#b)(*)\[\=* ]]; then
//! sh:262  	    opt2=${${match[1]}//[^a-zA-Z0-9_-]}=-${dir}${odescr}
//! sh:263  	  else
//! sh:264  	    opt2=${${opt}//[^a-zA-Z0-9_-]}=${dir}${odescr}
//! sh:265  	  fi
//! sh:266  	  if [[ "$descr" = :\=* ]]; then
//! sh:267  	    cache+=( "${opt2}::${(L)${opt%\]}#*\=}: " )
//! sh:268  	  elif [[ "$descr" = ::* ]]; then
//! sh:269  	    cache+=( "${opt2}${descr}" )
//! sh:270  	  else
//! sh:271  	    cache+=( "${opt2}:${descr}" )
//! sh:272  	  fi
//! sh:273  	done
//! sh:274        fi
//! sh:275
//! sh:276        # Descriptions with `=': mandatory argument.
//! sh:277        # Basically the same as the foregoing.
//! sh:278        # TODO: could they be combined?
//! sh:279
//! sh:280        tmpo=("${(@M)tmp:#[^:]##\=*}")
//! sh:281        if (( $#tmpo )); then
//! sh:282          tmp=("${(@)tmp:#[^:]##\=*}")
//! sh:283
//! sh:284  	for opt in "$tmpo[@]"; do
//! sh:285  	  if [[ $opt = (#b)(*):([^:]#) ]]; then
//! sh:286  	    opt=$match[1]
//! sh:287  	    odescr="[${match[2]}]"
//! sh:288  	  else
//! sh:289  	    odescr=
//! sh:290  	  fi
//! sh:291  	  opt2="${${opt%%\=*}//[^a-zA-Z0-9_-]}=${dir}${odescr}"
//! sh:292  	  if [[ "$descr" = :\=* ]]; then
//! sh:293  	    cache+=( "${opt2}:${(L)${opt%\]}#*\=}: " )
//! sh:294  	  else
//! sh:295  	    cache+=( "${opt2}${descr}" )
//! sh:296  	  fi
//! sh:297  	done
//! sh:298        fi
//! sh:299
//! sh:300        # Everything else is just added as an option without arguments or
//! sh:301        # as described by $descr.
//! sh:302
//! sh:303        if (( $#tmp )); then
//! sh:304          tmp=(
//! sh:305  	  # commands with a description of the option (as opposed
//! sh:306  	  # to the argument, which is what descr contains): needs to be
//! sh:307  	  # "option[description]".
//! sh:308  	  # Careful: \[ on RHS of substitution keeps the backslash,
//! sh:309  	  # I discovered after about half an hour, so don't do that.
//! sh:310  	  "${(@)^${(@)tmp:#^*:*}//:/[}]"
//! sh:311  	  # commands with no description
//! sh:312  	  "${(@)${(@)tmp:#*:*}//[^a-zA-Z0-9_-]}")
//! sh:313          if [[ -n "$descr" && "$descr" != ': :  ' ]]; then
//! sh:314  	  cache+=( "${(@)^tmp}${descr}" )
//! sh:315          else
//! sh:316  	  cache+=( "$tmp[@]" )
//! sh:317          fi
//! sh:318        fi
//! sh:319      done
//! sh:320      set -A "$name" "${(@)cache:# #}"
//! sh:321    fi
//! sh:322    set -- "$tmpargv[@]" "${(@P)name}"
//! sh:323  fi
//! sh:324
//! sh:325  zstyle -s ":completion:${curcontext}:options" auto-description autod
//! sh:326
//! sh:327  if (( $# )) && comparguments -i "$autod" "$singopt[@]" "$@"; then
//! sh:328    local action noargs aret expl local tried ret=1
//! sh:329    local next direct odirect equal single matcher matched ws tmp1 tmp2 tmp3
//! sh:330    local opts subc tc prefix suffix descrs actions subcs anum
//! sh:331    local origpre="$PREFIX" origipre="$IPREFIX" nm="$compstate[nmatches]"
//! sh:332
//! sh:333    if comparguments -D descrs actions subcs; then
//! sh:334      if comparguments -O next direct odirect equal; then
//! sh:335        opts=yes
//! sh:336        _tags "$subcs[@]" options
//! sh:337      else
//! sh:338        _tags "$subcs[@]"
//! sh:339      fi
//! sh:340    else
//! sh:341      if comparguments -a; then
//! sh:342        noargs='no more arguments'
//! sh:343      else
//! sh:344        noargs='no arguments'
//! sh:345      fi
//! sh:346      if comparguments -O next direct odirect equal; then
//! sh:347        opts=yes
//! sh:348        _tags options
//! sh:349      elif [[ $? -eq 2 ]]; then
//! sh:350          compadd -Q - "${PREFIX}${SUFFIX}"
//! sh:351          return 0
//! sh:352      else
//! sh:353        _message "$noargs"
//! sh:354        return 1
//! sh:355      fi
//! sh:356    fi
//! sh:357
//! sh:358    comparguments -M matcher
//! sh:359
//! sh:360    context=()
//! sh:361    state=()
//! sh:362    state_descr=()
//! sh:363
//! sh:364    while true; do
//! sh:365      while _tags; do
//! sh:366        anum=1
//! sh:367        if [[ -z "$tried" ]]; then
//! sh:368          while [[ anum -le  $#descrs ]]; do
//! sh:369
//! sh:370  	  action="$actions[anum]"
//! sh:371  	  descr="$descrs[anum]"
//! sh:372  	  subc="$subcs[anum++]"
//! sh:373
//! sh:374  	  if [[ $subc = argument* && -n $setnormarg ]]; then
//! sh:375  	    comparguments -n NORMARG
//! sh:376  	  fi
//! sh:377
//! sh:378            if [[ -n "$matched" ]] || _requested "$subc"; then
//! sh:379
//! sh:380              curcontext="${oldcontext%:*}:$subc"
//! sh:381
//! sh:382              _description "$subc" expl "$descr"
//! sh:383
//! sh:384              if [[ "$action" = \=\ * ]]; then
//! sh:385                action="$action[3,-1]"
//! sh:386                words=( "$subc" "$words[@]" )
//! sh:387  	      (( CURRENT++ ))
//! sh:388              fi
//! sh:389
//! sh:390              if [[ "$action" = -\>* ]]; then
//! sh:391  	      action="${${action[3,-1]##[ 	]#}%%[ 	]#}"
//! sh:392  	      if (( ! $state[(I)$action] )); then
//! sh:393                  comparguments -W line opt_args $opt_args_use_NUL_separators
//! sh:394                  state+=( "$action" )
//! sh:395                  state_descr+=( "$descr" )
//! sh:396  	        if [[ -n "$usecc" ]]; then
//! sh:397  	          curcontext="${oldcontext%:*}:$subc"
//! sh:398  	        else
//! sh:399  	          context+=( "$subc" )
//! sh:400  	        fi
//! sh:401                  compstate[restore]=''
//! sh:402                  aret=yes
//! sh:403                fi
//! sh:404              else
//! sh:405                if [[ -z "$local" ]]; then
//! sh:406                  local line
//! sh:407                  typeset -A opt_args
//! sh:408                  local=yes
//! sh:409                fi
//! sh:410
//! sh:411                comparguments -W line opt_args $opt_args_use_NUL_separators
//! sh:412
//! sh:413                if [[ "$action" = \ # ]]; then
//! sh:414
//! sh:415                  # An empty action means that we should just display a message.
//! sh:416
//! sh:417  	        _message -e "$subc" "$descr"
//! sh:418  	        mesg=yes
//! sh:419  	        tried=yes
//! sh:420                  alwopt=${alwopt:-yes}
//! sh:421                elif [[ "$action" = \(\(*\)\) ]]; then
//! sh:422
//! sh:423                  # ((...)) contains literal strings with descriptions.
//! sh:424
//! sh:425                  eval ws\=\( "${action[3,-3]}" \)
//! sh:426
//! sh:427                  _describe -t "$subc" "$descr" ws -M "$matcher" "$subopts[@]" ||
//! sh:428                      alwopt=${alwopt:-yes}
//! sh:429  	        tried=yes
//! sh:430
//! sh:431                elif [[ "$action" = \(*\) ]]; then
//! sh:432
//! sh:433                  # Anything inside `(...)' is added directly.
//! sh:434
//! sh:435                  eval ws\=\( "${action[2,-2]}" \)
//! sh:436
//! sh:437                  _all_labels "$subc" expl "$descr" compadd "$subopts[@]" -a - ws ||
//! sh:438                      alwopt=${alwopt:-yes}
//! sh:439  	        tried=yes
//! sh:440                elif [[ "$action" = \{*\} ]]; then
//! sh:441
//! sh:442                  # A string in braces is evaluated.
//! sh:443
//! sh:444                  while _next_label "$subc" expl "$descr"; do
//! sh:445                    eval "$action[2,-2]" && ret=0
//! sh:446                  done
//! sh:447                  (( ret )) && alwopt=${alwopt:-yes}
//! sh:448  	        tried=yes
//! sh:449                elif [[ "$action" = \ * ]]; then
//! sh:450
//! sh:451                  # If the action starts with a space, we just call it.
//! sh:452
//! sh:453  	        eval "action=( $action )"
//! sh:454                  while _next_label "$subc" expl "$descr"; do
//! sh:455                    "$action[@]" && ret=0
//! sh:456                  done
//! sh:457                  (( ret )) && alwopt=${alwopt:-yes}
//! sh:458  	        tried=yes
//! sh:459                else
//! sh:460
//! sh:461                  # Otherwise we call it with the description-arguments.
//! sh:462
//! sh:463  	        eval "action=( $action )"
//! sh:464                  while _next_label "$subc" expl "$descr"; do
//! sh:465                    "$action[1]" "$subopts[@]" "$expl[@]" "${(@)action[2,-1]}" && ret=0
//! sh:466  	        done
//! sh:467                  (( ret )) && alwopt=${alwopt:-yes}
//! sh:468  	        tried=yes
//! sh:469                fi
//! sh:470              fi
//! sh:471            fi
//! sh:472          done
//! sh:473        fi
//! sh:474        if _requested options &&
//! sh:475           [[ -z "$hasopts" &&
//! sh:476              -z "$matched" &&
//! sh:477              ( -z "$aret" || "$PREFIX" = "$origpre" ) ]] &&
//! sh:478            { ! zstyle -T ":completion:${oldcontext%:*}:options" prefix-needed ||
//! sh:479              [[ "$origpre" = [-+]* || -z "$aret$mesg$tried" ]] } ; then
//! sh:480  	local prevpre="$PREFIX" previpre="$IPREFIX" prevcontext="$curcontext"
//! sh:481
//! sh:482          curcontext="${oldcontext%:*}:options"
//! sh:483
//! sh:484  	hasopts=yes
//! sh:485
//! sh:486  	PREFIX="$origpre"
//! sh:487  	IPREFIX="$origipre"
//! sh:488
//! sh:489          if [[ -z "$alwopt" || -z "$tried" || "$alwopt" = arg ]] &&
//! sh:490             comparguments -s single; then
//! sh:491
//! sh:492            if [[ "$single" = direct ]]; then
//! sh:493              _all_labels options expl option \
//! sh:494  	        compadd -QS '' - "${PREFIX}${SUFFIX}"
//! sh:495            elif [[ -z "$optarg" && "$single" = next ]]; then
//! sh:496              _all_labels options expl option \
//! sh:497  	        compadd -Q - "${PREFIX}${SUFFIX}"
//! sh:498            elif [[ "$single" = equal ]]; then
//! sh:499              _all_labels options expl option \
//! sh:500  	        compadd -QqS= - "${PREFIX}${SUFFIX}"
//! sh:501            else
//! sh:502
//! sh:503  	    tmp1=( "$next[@]" "$direct[@]" "$odirect[@]" "$equal[@]" )
//! sh:504
//! sh:505              [[ "$PREFIX" = [-+]* ]] && tmp1=( "${(@M)tmp1:#${PREFIX[1]}*}" )
//! sh:506
//! sh:507              [[ "$single" = next ]] &&
//! sh:508                  tmp1=( "${(@)tmp1:#[-+]${PREFIX[-1]}((#e)|:*)}" )
//! sh:509
//! sh:510  	    [[ "$PREFIX" != --* ]] && tmp1=( "${(@)tmp1:#--*}" )
//! sh:511  	    tmp3=( "${(M@)tmp1:#[-+]?[^:]*}" )
//! sh:512  	    tmp1=( "${(M@)tmp1:#[-+]?(|:*)}" )
//! sh:513  	    tmp2=( "${PREFIX}${(@M)^${(@)${(@)tmp1%%:*}#[-+]}:#?}" )
//! sh:514
//! sh:515              _describe -O option \
//! sh:516                  tmp1 tmp2 -S '' -- \
//! sh:517                  tmp3
//! sh:518
//! sh:519              [[ -n "$optarg" && "$single" = next && nm -eq $compstate[nmatches] ]] &&
//! sh:520                  _all_labels options expl option \
//! sh:521  	            compadd -Q - "${PREFIX}${SUFFIX}"
//! sh:522
//! sh:523            fi
//! sh:524            single=yes
//! sh:525          else
//! sh:526            next+=( "$odirect[@]" )
//! sh:527            _describe -O option \
//! sh:528                next -M "$matcher" -- \
//! sh:529                direct -S '' -M "$matcher" -- \
//! sh:530                equal -qS= -M "$matcher"
//! sh:531          fi
//! sh:532  	PREFIX="$prevpre"
//! sh:533  	IPREFIX="$previpre"
//! sh:534          curcontext="$prevcontext"
//! sh:535        fi
//! sh:536        [[ -n "$tried" && "${${alwopt:+$origpre}:-$PREFIX}" != [-+]* ]] && break
//! sh:537      done
//! sh:538      if [[ -n "$opts" && -z "$aret" &&
//! sh:539            -z "$matched" &&
//! sh:540            ( -z "$tried" || -n "$alwopt" ) &&
//! sh:541            nm -eq compstate[nmatches] ]]; then
//! sh:542
//! sh:543        PREFIX="$origpre"
//! sh:544        IPREFIX="$origipre"
//! sh:545
//! sh:546        prefix="${PREFIX#*\=}"
//! sh:547        suffix="$SUFFIX"
//! sh:548        PREFIX="${PREFIX%%\=*}"
//! sh:549        SUFFIX=''
//! sh:550
//! sh:551        compadd -M "$matcher" -D equal - "${(@)equal%%:*}"
//! sh:552
//! sh:553        if [[ $#equal -eq 1 ]]; then
//! sh:554          PREFIX="$prefix"
//! sh:555  	SUFFIX="$suffix"
//! sh:556  	IPREFIX="${IPREFIX}${equal[1]%%:*}="
//! sh:557  	matched=yes
//! sh:558
//! sh:559  	comparguments -L "${equal[1]%%:*}" descrs actions subcs
//! sh:560
//! sh:561  	_tags "$subcs[@]"
//! sh:562
//! sh:563  	continue
//! sh:564        fi
//! sh:565      fi
//! sh:566      break
//! sh:567    done
//! sh:568
//! sh:569    [[ -z "$aret" || -z "$usecc" ]] && curcontext="$oldcontext"
//! sh:570
//! sh:571    if [[ -n "$aret" ]]; then
//! sh:572      [[ -n $rawret ]] && return 300
//! sh:573
//! sh:574  ### Returning non-zero would allow the calling function to add its own
//! sh:575  ### completions if we generated only options and have to use a ->state
//! sh:576  ### action.  But if that then doesn't generate matches, the calling
//! sh:577  ### function's return value would be wrong unless it compares
//! sh:578  ### $compstate[nmatches] to its previous value.  Ugly.
//! sh:579  ###
//! sh:580  ###    return 1
//! sh:581    else
//! sh:582      [[ -n "$noargs" && nm -eq "$compstate[nmatches]" ]] && _message "$noargs"
//! sh:583    fi
//! sh:584    # Set the return value.
//! sh:585
//! sh:586    [[ nm -ne "$compstate[nmatches]" ]]
//! sh:587  else
//! sh:588    return 1
//! sh:589  fi
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
