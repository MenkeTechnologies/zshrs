//! Port of `_path_files` from `Completion/Unix/Type/_path_files`.
//!
//! Full upstream body (895 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  local -a match mbegin mend
//! sh:  4
//! sh:  5  local splitchars
//! sh:  6  if zstyle -s ":completion:${curcontext}:" file-split-chars splitchars; then
//! sh:  7    compset -P "*[${(q)splitchars}]"
//! sh:  8  fi
//! sh:  9
//! sh: 10  # Look for glob qualifiers.  Do this first:  if we're really
//! sh: 11  # in a glob qualifier, we don't actually want to expand
//! sh: 12  # the earlier part of the path.  We can't expand inside
//! sh: 13  # parentheses otherwise, so as we test that successfully
//! sh: 14  # we should be able to commit to glob qualifiers here.
//! sh: 15  #
//! sh: 16  # Extra nastiness to be careful about a quoted parenthesis.
//! sh: 17  # The initial tests look for parentheses with zero or an
//! sh: 18  # even number of backslashes in front.  We also require that
//! sh: 19  # there was at least one character before the parenthesis for
//! sh: 20  # a bare glob qualifier.
//! sh: 21  # The later test looks for an outstanding quote.
//! sh: 22  if _have_glob_qual $PREFIX; then
//! sh: 23    local ret=1
//! sh: 24    compset -p ${#match[1]}
//! sh: 25    compset -S '[^\)\|\~]#(|\))'
//! sh: 26    if [[ $_comp_caller_options[extendedglob] == on ]] && compset -P '\#'; then
//! sh: 27      _globflags && ret=0
//! sh: 28    else
//! sh: 29      if [[ $_comp_caller_options[extendedglob] == on ]]; then
//! sh: 30        local -a flags
//! sh: 31        flags=(
//! sh: 32        '#:introduce glob flag'
//! sh: 33        )
//! sh: 34        _describe -t globflags "glob flag" flags -Q -S '' && ret=0
//! sh: 35      fi
//! sh: 36      _globquals && ret=0
//! sh: 37    fi
//! sh: 38    return ret
//! sh: 39  fi
//! sh: 40
//! sh: 41  # Utility function for in-path completion. This allows `/u/l/b<TAB>'
//! sh: 42  # to complete to `/usr/local/bin'.
//! sh: 43
//! sh: 44  local linepath realpath donepath prepath testpath exppath skips skipped
//! sh: 45  local tmp1 tmp2 tmp3 tmp4 i orig eorig pre suf tpre tsuf opre osuf cpre
//! sh: 46  local pats haspats ignore pfx pfxsfx sopt gopt opt sdirs ignpar cfopt listsfx
//! sh: 47  local nm=$compstate[nmatches] menu matcher mopts sort mid accex fake
//! sh: 48  local listfiles listopts tmpdisp origtmp1 Uopt
//! sh: 49  local accept_exact_dirs path_completion
//! sh: 50  integer npathcheck
//! sh: 51  local -a Mopts
//! sh: 52
//! sh: 53  typeset -U prepaths exppaths
//! sh: 54
//! sh: 55  exppaths=()
//! sh: 56
//! sh: 57  # Get the options.
//! sh: 58
//! sh: 59  zparseopts -a mopts \
//! sh: 60      'P:=pfx' 'S:=pfxsfx' 'q=pfxsfx' 'r:=pfxsfx' 'R:=pfxsfx' \
//! sh: 61      'W:=prepaths' 'F:=ignore' 'M+:=matcher' \
//! sh: 62      J+: V+: x+: X+: 1 2 o+: n 'f=tmp1' '/=tmp1' 'g+:-=tmp1'
//! sh: 63
//! sh: 64  sopt="-${(@j::M)${(@)tmp1#-}#?}"
//! sh: 65  (( $tmp1[(I)-[/g]*] )) && haspats=yes
//! sh: 66  (( $tmp1[(I)-g*] )) && gopt=yes
//! sh: 67  if (( $tmp1[(I)-/] )); then
//! sh: 68    pats="${(@)${(@M)tmp1:#-g*}#-g}"
//! sh: 69    pats=( '*(-/)' ${${(z):-x $pats}[2,-1]} )
//! sh: 70  else
//! sh: 71    pats="${(@)${(@M)tmp1:#-g*}#-g}"
//! sh: 72    pats=( ${${(z):-x $pats}[2,-1]} )
//! sh: 73  fi
//! sh: 74  pats=( "${(@)pats:# #}" )
//! sh: 75
//! sh: 76  if (( $#pfx )); then
//! sh: 77    compset -P "${(b)pfx[2]}" || pfxsfx=( "$pfx[@]" "$pfxsfx[@]" )
//! sh: 78  fi
//! sh: 79
//! sh: 80  if (( $#prepaths )); then
//! sh: 81    tmp1="${prepaths[2]}"
//! sh: 82    if [[ "$tmp1[1]" = '(' ]]; then
//! sh: 83      prepaths=( ${^=tmp1[2,-2]%/}/ )
//! sh: 84    elif [[ "$tmp1[1]" = '/' ]]; then
//! sh: 85      prepaths=( "${tmp1%/}/" )
//! sh: 86    else
//! sh: 87      prepaths=( ${(P)^tmp1%/}/ )
//! sh: 88      (( ! $#prepaths )) && prepaths=( ${tmp1%/}/ )
//! sh: 89    fi
//! sh: 90    (( ! $#prepaths )) && prepaths=( '' )
//! sh: 91  else
//! sh: 92    prepaths=( '' )
//! sh: 93  fi
//! sh: 94
//! sh: 95  if (( $#ignore )); then
//! sh: 96    if [[ "${ignore[2]}" = \(* ]]; then
//! sh: 97      ignore=( ${=ignore[2][2,-2]} )
//! sh: 98    else
//! sh: 99      ignore=( ${(P)ignore[2]} )
//! sh:100    fi
//! sh:101  fi
//! sh:102
//! sh:103  # If we were given no file selection option, we behave as if we were given
//! sh:104  # a `-f'.
//! sh:105
//! sh:106  if [[ "$sopt" = -(f|) ]]; then
//! sh:107    if [[ -z "$gopt" ]]; then
//! sh:108      sopt='-f'
//! sh:109      pats=('*')
//! sh:110    else
//! sh:111      unset sopt
//! sh:112    fi
//! sh:113  fi
//! sh:114
//! sh:115  if (( ! $mopts[(I)-[JVX]] )); then
//! sh:116    local expl
//! sh:117
//! sh:118    if [[ -z "$gopt" && "$sopt" = -/ ]]; then
//! sh:119      _description directories expl directory
//! sh:120    else
//! sh:121      _description files expl file
//! sh:122    fi
//! sh:123    tmp1=$expl[(I)-M*]
//! sh:124    if (( tmp1 )); then
//! sh:125      if (( $#matcher )); then
//! sh:126        matcher[2]="$matcher[2] $expl[1+tmp1]"
//! sh:127      else
//! sh:128        matcher=(-M "$expl[1+tmp1]")
//! sh:129      fi
//! sh:130    fi
//! sh:131    mopts=( "$mopts[@]" "$expl[@]" )
//! sh:132  fi
//! sh:133
//! sh:134  # If given no `-F' option, we may want to use $fignore, turned into patterns.
//! sh:135
//! sh:136  [[ -z "$_comp_no_ignore" && $#ignore -eq 0 &&
//! sh:137     ( -z $gopt || "$pats" = \ #\*\ # ) && -n $FIGNORE ]] &&
//! sh:138      ignore=( "?*${^fignore[@]}" )
//! sh:139
//! sh:140  if (( $#ignore )); then
//! sh:141    _comp_ignore=( "$_comp_ignore[@]" "$ignore[@]" )
//! sh:142    (( $mopts[(I)-F] )) || mopts=( "$mopts[@]" -F _comp_ignore )
//! sh:143  fi
//! sh:144
//! sh:145  if [[ $#matcher -eq 0 && -o nocaseglob ]]; then
//! sh:146    # If globbing is case insensitive and there's no matcher,
//! sh:147    # do case-insensitive matching.
//! sh:148    matcher=( -M 'm:{a-zA-Z}={A-Za-z}' )
//! sh:149  fi
//! sh:150
//! sh:151  if (( $#matcher )); then
//! sh:152    # Add the current matcher to the options to compadd.
//! sh:153    mopts=( "$mopts[@]" "$matcher[@]" )
//! sh:154  fi
//! sh:155
//! sh:156  if zstyle -s ":completion:${curcontext}:" file-sort tmp1; then
//! sh:157    case "$tmp1" in
//! sh:158    *size*)             sort=oL;;
//! sh:159    *links*)            sort=ol;;
//! sh:160    *(time|date|modi)*) sort=om;;
//! sh:161    *access*)           sort=oa;;
//! sh:162    *(inode|change)*)   sort=oc;;
//! sh:163    *)                  sort=on;;
//! sh:164    esac
//! sh:165    [[ "$tmp1" = *rev* ]] && sort[1]=O
//! sh:166    [[ "$tmp1" = *follow* ]] && sort="-${sort}-"
//! sh:167
//! sh:168    if [[ "$sort" = on ]]; then
//! sh:169      sort=
//! sh:170    else
//! sh:171      mopts=( -o nosort "${mopts[@]}" )
//! sh:172
//! sh:173      tmp2=()
//! sh:174      for tmp1 in "$pats[@]"; do
//! sh:175        if _have_glob_qual "$tmp1" complete; then
//! sh:176  	# unbalanced parenthesis is correct: match[1] contains the start,
//! sh:177  	# match[5] doesn't contain the end.
//! sh:178  	tmp2+=( "${match[1]}#q${sort})(${match[5]})" )
//! sh:179        else
//! sh:180          tmp2+=( "${tmp1}(${sort})" )
//! sh:181        fi
//! sh:182      done
//! sh:183      pats=( "$tmp2[@]" )
//! sh:184    fi
//! sh:185  fi
//! sh:186
//! sh:187  # Check if we have to skip over sequences of slashes. The value of $skips
//! sh:188  # is used below to match the pathname components we always have to accept
//! sh:189  # immediately.
//! sh:190
//! sh:191  if zstyle -t ":completion:${curcontext}:paths" squeeze-slashes; then
//! sh:192    skips='((.|..|)/)##'
//! sh:193  else
//! sh:194    skips='((.|..)/)##'
//! sh:195  fi
//! sh:196
//! sh:197  zstyle -s ":completion:${curcontext}:paths" special-dirs sdirs
//! sh:198  zstyle -t ":completion:${curcontext}:paths" list-suffixes &&
//! sh:199      listsfx=yes
//! sh:200
//! sh:201  [[ "$pats" = ((|*[[:blank:]])\*(|[[:blank:]]*|\([^[:blank:]]##\))|*\([^[:blank:]]#/[^[:blank:]]#\)*) ]] &&
//! sh:202      sopt=$sopt/
//! sh:203
//! sh:204  zstyle -a ":completion:${curcontext}:paths" accept-exact accex
//! sh:205  zstyle -a ":completion:${curcontext}:" fake-files fake
//! sh:206
//! sh:207  zstyle -s ":completion:${curcontext}:" ignore-parents ignpar
//! sh:208
//! sh:209  zstyle -t ":completion:${curcontext}:paths" accept-exact-dirs &&
//! sh:210    accept_exact_dirs=1
//! sh:211  zstyle -T ":completion:${curcontext}:paths" path-completion &&
//! sh:212    path_completion=1
//! sh:213
//! sh:214  if [[ -n "$compstate[pattern_match]" ]]; then
//! sh:215    if { [[ -z "$SUFFIX" ]] && _have_glob_qual "$PREFIX" complete; } ||
//! sh:216      _have_glob_qual "$SUFFIX" complete; then
//! sh:217      # Copy all glob qualifiers from the line to
//! sh:218      # the patterns used when generating matches
//! sh:219      tmp3=${match[5]}
//! sh:220      if [[ -n "$SUFFIX" ]]; then
//! sh:221        SUFFIX=${match[2]}
//! sh:222      else
//! sh:223        PREFIX=${match[2]}
//! sh:224      fi
//! sh:225      tmp2=()
//! sh:226      for tmp1 in "$pats[@]"; do
//! sh:227        if _have_glob_qual "$tmp1" complete; then
//! sh:228  	# unbalanced parenthesis is correct: match[1] contains the start,
//! sh:229  	# match[5] doesn't contain the end.
//! sh:230  	tmp2+=( "${match[1]}${tmp3}${match[5]})")
//! sh:231        else
//! sh:232  	tmp2+=( "${tmp1}(${tmp3})" )
//! sh:233        fi
//! sh:234      done
//! sh:235      pats=( "$tmp2[@]" )
//! sh:236    fi
//! sh:237  fi
//! sh:238
//! sh:239  # We get the prefix and the suffix from the line and save the whole
//! sh:240  # original string. Then we see if we will do menu completion.
//! sh:241
//! sh:242  pre="$PREFIX"
//! sh:243  suf="$SUFFIX"
//! sh:244  opre="$PREFIX"
//! sh:245  osuf="$SUFFIX"
//! sh:246  orig="${PREFIX}${SUFFIX}"
//! sh:247  eorig="$orig"
//! sh:248
//! sh:249  [[ $compstate[insert] = (*menu|[0-9]*) || -n "$_comp_correct" ||
//! sh:250     ( -n "$compstate[pattern_match]" &&
//! sh:251       "${orig#\~}" != (|*[^\\])[][*?#~^\|\<\>]* ) ]] && menu=yes
//! sh:252  if [[ -n "$_comp_correct" ]]; then
//! sh:253      cfopt=-
//! sh:254      Uopt=-U
//! sh:255  else
//! sh:256      Mopts=(-M "r:|/=* r:|=*")
//! sh:257  fi
//! sh:258
//! sh:259  # Now let's have a closer look at the string to complete.
//! sh:260
//! sh:261  if [[ "$pre" = [^][*?#^\|\<\>\\]#(\`[^\`]#\`|\$)*/* && "$compstate[quote]" != \' ]]; then
//! sh:262
//! sh:263    # If there is a parameter expansion in the word from the line, we try
//! sh:264    # to complete the beast by expanding the prefix and completing anything
//! sh:265    # after the first slash after the parameter expansion.
//! sh:266    # This fails for things like `f/$foo/b/<TAB>' where the first `f' is
//! sh:267    # meant as a partial path.
//! sh:268
//! sh:269    linepath="${(M)pre##*\$[^/]##/}"
//! sh:270    function {
//! sh:271      # do not treat an unset parameter expansion as the empty string
//! sh:272      setopt localoptions nounset
//! sh:273      eval 'realpath=${(e)~linepath}' 2>/dev/null
//! sh:274    }
//! sh:275    [[ -z "$realpath" || "$realpath" = "$linepath" ]] && return 1
//! sh:276    pre="${pre#${linepath}}"
//! sh:277    i='[^/]'
//! sh:278    i="${#linepath//$i}"
//! sh:279    orig="${orig[1,(in:i:)/][1,-2]}"
//! sh:280    donepath=
//! sh:281    prepaths=( '' )
//! sh:282  elif [[ "$pre[1]" = \~ && "$compstate[quote]" = (|\`) ]]; then
//! sh:283
//! sh:284    # It begins with `~', so remember anything before the first slash to be able
//! sh:285    # to report it to the completion code. Also get an expanded version of it
//! sh:286    # (in `realpath'), so that we can generate the matches. Then remove that
//! sh:287    # prefix from the string to complete, set `donepath' to build the correct
//! sh:288    # paths and make sure that the loop below is run only once with an empty
//! sh:289    # prefix path by setting `prepaths'.
//! sh:290
//! sh:291    linepath="${pre[2,-1]%%/*}"
//! sh:292    if [[ -z "$linepath" ]]; then
//! sh:293      realpath="${HOME%/}/"
//! sh:294    elif [[ "$linepath" = ([-+]|)[0-9]## ]]; then
//! sh:295      if [[ "$linepath" != [-+]* ]]; then
//! sh:296        tmp1="$linepath"
//! sh:297      else
//! sh:298        if [[ "$linepath" = -* ]]; then
//! sh:299          tmp1=$(( $#dirstack $linepath ))
//! sh:300        else
//! sh:301          tmp1=$linepath[2,-1]
//! sh:302        fi
//! sh:303        [[ -o pushdminus ]] && tmp1=$(( $#dirstack - $tmp1 ))
//! sh:304      fi
//! sh:305      if (( ! tmp1 )); then
//! sh:306        realpath=$PWD/
//! sh:307      elif [[ tmp1 -le $#dirstack ]]; then
//! sh:308        realpath=$dirstack[tmp1]/
//! sh:309      else
//! sh:310        _message 'not enough directory stack entries'
//! sh:311        return 1
//! sh:312      fi
//! sh:313    elif [[ "$linepath" = [-+] ]]; then
//! sh:314      realpath=${~:-\~$linepath}/
//! sh:315    else
//! sh:316      eval "realpath=~${linepath}/" 2>/dev/null
//! sh:317      if [[ -z "$realpath" ]]; then
//! sh:318        _message "unknown user \`$linepath'"
//! sh:319        return 1
//! sh:320      fi
//! sh:321    fi
//! sh:322    linepath="~${linepath}/"
//! sh:323    [[ "$realpath" = "$linepath" ]] && return 1
//! sh:324    pre="${pre#*/}"
//! sh:325    orig="${orig#*/}"
//! sh:326    donepath=
//! sh:327    prepaths=( '' )
//! sh:328  else
//! sh:329    # If the string does not start with a `~' we don't remove a prefix from the
//! sh:330    # string.
//! sh:331
//! sh:332    linepath=
//! sh:333    realpath=
//! sh:334
//! sh:335    if zstyle -s ":completion:${curcontext}:" preserve-prefix tmp1 &&
//! sh:336       [[ -n "$tmp1" && "$pre" = (#b)(${~tmp1})* ]]; then
//! sh:337
//! sh:338      pre="$pre[${#match[1]}+1,-1]"
//! sh:339      orig="$orig[${#match[1]}+1,-1]"
//! sh:340      donepath="$match[1]"
//! sh:341      prepaths=( '' )
//! sh:342
//! sh:343    elif [[ "$pre[1]" = / ]]; then
//! sh:344      # If it is a absolute path name, we remove the first slash and put it in
//! sh:345      # `donepath' meaning that we treat it as the path that was already handled.
//! sh:346      # Also, we don't use the paths from `-W'.
//! sh:347
//! sh:348      pre="$pre[2,-1]"
//! sh:349      orig="$orig[2,-1]"
//! sh:350      donepath='/'
//! sh:351      prepaths=( '' )
//! sh:352    else
//! sh:353      # The common case, we just use the string as it is, unless it begins with
//! sh:354      # `./' or `../' in which case we don't use the paths from `-W'.
//! sh:355
//! sh:356      [[ "$pre" = (.|..)/* ]] && prepaths=( '' )
//! sh:357      donepath=
//! sh:358    fi
//! sh:359  fi
//! sh:360
//! sh:361  # Now we generate the matches. First we loop over all prefix paths given
//! sh:362  # with the `-W' option.
//! sh:363
//! sh:364  for prepath in "$prepaths[@]"; do
//! sh:365
//! sh:366    # Get local copies of the prefix, suffix, and the prefix path to use
//! sh:367    # in the following loop, which walks through the pathname components
//! sh:368    # in the string from the line.
//! sh:369
//! sh:370    skipped=
//! sh:371    cpre=
//! sh:372
//! sh:373    if [[ ( -n $accept_exact_dirs || -z $path_completion ) && \
//! sh:374          ${pre} = (#b)(*)/([^/]#) ]]; then
//! sh:375      # We've been told either that we can accept an exact directory prefix
//! sh:376      # immediately, or that path expansion is inhibited.  Try the longest
//! sh:377      # path prefix first: in the first case, this saves stats in the simple
//! sh:378      # case and may get around automount behaviour if early components don't
//! sh:379      # yet exist, and in the second case this is the prefix we want to keep.
//! sh:380      #
//! sh:381      # Explanation of substitution: For tmp1 and tpre, which are used further
//! sh:382      # on, we need to remove quotes from everything that's not a pattern
//! sh:383      # character, because the code that does the file generation only
//! sh:384      # strips quotes from pattern characters (you know better than
//! sh:385      # to ask why).
//! sh:386      tmp1=${match[1]}
//! sh:387      tpre=${match[2]}
//! sh:388      tmp2=$tmp1
//! sh:389      tmp1=${tmp1//(#b)\\(?)/$match[1]}
//! sh:390      tpre=${tpre//(#b)\\([^\\\]\[\^\~\(\)\#\*\?])/$match[1]}
//! sh:391      # Theory: donepath needs the quoting of special characters
//! sh:392      # still in it.  However, we need it without at this point.
//! sh:393      # (I think.)  Note this is different from the above where we're
//! sh:394      # doing something a bit different.
//! sh:395      tmp3=${donepath//(#b)\\(?)/$match[1]}
//! sh:396      while true; do
//! sh:397        if [[ -z $path_completion || -d $prepath$realpath$tmp3$tmp2 ]]; then
//! sh:398  	tmp3=$tmp3$tmp1/
//! sh:399  	# Now put donepath back the way it should be.  (I think.)
//! sh:400  	donepath=${tmp3//(#b)([\\\]\[\^\~\(\)\#\*\?])/\\$match[1]}
//! sh:401  	pre=$tpre
//! sh:402  	break
//! sh:403        elif [[ $tmp1 = (#b)(*)/([^/]#) ]]; then
//! sh:404  	tmp1=$match[1]
//! sh:405  	tpre=$match[2]/$tpre
//! sh:406        else
//! sh:407  	break
//! sh:408        fi
//! sh:409      done
//! sh:410    fi
//! sh:411
//! sh:412    tpre="$pre"
//! sh:413    tsuf="$suf"
//! sh:414    # Now we strip quoting from pattern characters, too, because
//! sh:415    # testpath is used as a literal string.  I suppose we could
//! sh:416    # alternatively use ${~testpath} later.
//! sh:417    #
//! sh:418    # I'm not sure if donepath itself should be entirely unquoted at
//! sh:419    # some point but probably not here, since we need the quoted pattern
//! sh:420    # characters in tmp1 below (I think).
//! sh:421    testpath="${donepath//(#b)\\([\\\]\[\^\~\(\)\#\*\?])/$match[1]}"
//! sh:422
//! sh:423    tmp2="${(M)tpre##${~skips}}"
//! sh:424    tpre="${tpre#$tmp2}"
//! sh:425
//! sh:426    tmp1=( "$prepath$realpath$donepath$tmp2" )
//! sh:427
//! sh:428    # count of attempts for pws non-canonical hack
//! sh:429    (( npathcheck = 0 ))
//! sh:430    while true; do
//! sh:431
//! sh:432      origtmp1=("${tmp1[@]}")
//! sh:433      # Get the prefix and suffix for matching.
//! sh:434
//! sh:435      if [[ "$tpre" = */* ]]; then
//! sh:436        PREFIX="${tpre%%/*}"
//! sh:437        SUFFIX=
//! sh:438      else
//! sh:439        PREFIX="${tpre}"
//! sh:440        SUFFIX="${tsuf%%/*}"
//! sh:441      fi
//! sh:442
//! sh:443      # Force auto-mounting. There might be a better way...
//! sh:444      # Commented out in the hope that `pws non-canonical hack'
//! sh:445      # down below does this for us.  Can be uncommented if it
//! sh:446      # doesn't.
//! sh:447
//! sh:448      # : ${^tmp1}/${PREFIX}${SUFFIX}/.(/)
//! sh:449
//! sh:450      # Get the matching files by globbing.
//! sh:451
//! sh:452      tmp2=( "$tmp1[@]" )
//! sh:453
//! sh:454      if [[ "$tpre$tsuf" = (#b)*/(*) ]]; then
//! sh:455
//! sh:456        # We are going to be looping over the leading path segments.
//! sh:457        # This means we should not apply special-dirs handling unless
//! sh:458        # the path tail is a fake directory that needs to be simulated,
//! sh:459        # and we should not apply pattern matching until we are looking
//! sh:460        # for files rather than for intermediate directories.
//! sh:461
//! sh:462        if [[ -n "$fake${match[1]}" ]]; then
//! sh:463          compfiles -P$cfopt tmp1 accex "$skipped" "$_matcher $matcher[2]" "$sdirs" fake
//! sh:464        else
//! sh:465          compfiles -P$cfopt tmp1 accex "$skipped" "$_matcher $matcher[2]" '' fake
//! sh:466        fi
//! sh:467      elif [[ "$sopt" = *[/f]* ]]; then
//! sh:468        compfiles -p$cfopt tmp1 accex "$skipped" "$_matcher $matcher[2]" "$sdirs" fake "$pats[@]"
//! sh:469      else
//! sh:470        compfiles -p$cfopt tmp1 accex "$skipped" "$_matcher $matcher[2]" '' fake "$pats[@]"
//! sh:471      fi
//! sh:472      tmp1=( $~tmp1 ) 2> /dev/null
//! sh:473
//! sh:474      if [[ -n "$PREFIX$SUFFIX" ]]; then
//! sh:475        # See which of them match what's on the line.
//! sh:476
//! sh:477        # pws non-canonical hack which seems to work so far...
//! sh:478        # if we didn't match by globbing, check that there is
//! sh:479        # something to match by explicit name.  This is for
//! sh:480        # `clever' filing systems where names pop into existence
//! sh:481        # when referenced.
//! sh:482        #
//! sh:483        # As suggested by Bart, to make sure the "compfiles" checks
//! sh:484        # still work we repeat the tests above if we successfully
//! sh:485        # find something that might need adding, but we make sure
//! sh:486        # we only do this once for completion of each path segment.
//! sh:487        if (( ! $#tmp1 && npathcheck == 0 )); then
//! sh:488  	(( npathcheck = 1 ))
//! sh:489  	for tmp3 in "$tmp2[@]"; do
//! sh:490  	  if [[ -n $tmp3 && $tmp3 != */ ]]; then
//! sh:491  	    tmp3+=/
//! sh:492  	  fi
//! sh:493  	  if [[ -e "$tmp3${(Q)PREFIX}${(Q)SUFFIX}" ]] then
//! sh:494  	    (( npathcheck = 2 ))
//! sh:495  	  fi
//! sh:496  	done
//! sh:497  	if (( npathcheck == 2 )); then
//! sh:498  	  # repeat loop with same arguments
//! sh:499  	  tmp1=("$origtmp1[@]")
//! sh:500  	  continue
//! sh:501  	fi
//! sh:502        fi
//! sh:503
//! sh:504        if (( ! $#tmp1 )); then
//! sh:505          tmp2=( ${^${tmp2:#/}}/$PREFIX$SUFFIX )
//! sh:506        elif [[ "$tmp1[1]" = */* ]]; then
//! sh:507          if [[ -n "$_comp_correct" ]]; then
//! sh:508            tmp2=( "$tmp1[@]" )
//! sh:509            builtin compadd -D tmp1 "$matcher[@]" - "${(@)tmp1:t}"
//! sh:510
//! sh:511            if [[ $#tmp1 -eq 0 ]]; then
//! sh:512              tmp1=( "$tmp2[@]" )
//! sh:513  	    compadd -D tmp1 "$matcher[@]" - "${(@)tmp2:t}"
//! sh:514            fi
//! sh:515          else
//! sh:516            tmp2=( "$tmp1[@]" )
//! sh:517            compadd -D tmp1 "$matcher[@]" - "${(@)tmp1:t}"
//! sh:518          fi
//! sh:519        else
//! sh:520          tmp2=( '' )
//! sh:521          compadd -D tmp1 "$matcher[@]" -a tmp1
//! sh:522        fi
//! sh:523
//! sh:524        # If no file matches, save the expanded path and continue with
//! sh:525        # the outer loop.
//! sh:526
//! sh:527        if (( ! $#tmp1 )); then
//! sh:528   	if [[ "$tmp2[1]" = */* ]]; then
//! sh:529  	  tmp2=( "${(@)tmp2#${prepath}${realpath}}" )
//! sh:530  	  if [[ "$tmp2[1]" = */* ]]; then
//! sh:531  	    tmp2=( "${(@)tmp2:h}" )
//! sh:532  	    compquote tmp2
//! sh:533  	    if [[ "$tmp2" = */ ]]; then
//! sh:534  	      exppaths=( "$exppaths[@]" ${^tmp2}${tpre}${tsuf} )
//! sh:535  	    else
//! sh:536  	      exppaths=( "$exppaths[@]" ${^tmp2}/${tpre}${tsuf} )
//! sh:537  	    fi
//! sh:538            elif [[ ${tpre}${tsuf} = */* ]]; then
//! sh:539  	    exppaths=( "$exppaths[@]" ${tpre}${tsuf} )
//! sh:540
//! sh:541  	    ### this once was in an `else' (not `elif')
//! sh:542  	  fi
//! sh:543          fi
//! sh:544          continue 2
//! sh:545        fi
//! sh:546      elif (( ! $#tmp1 )); then
//! sh:547        # A little extra hack: if we were completing `foo/<TAB>' and `foo'
//! sh:548        # contains no files, this will normally produce no matches and other
//! sh:549        # completers might think that's it's their time now. But if the next
//! sh:550        # completer is _correct or something like that, this will result in
//! sh:551        # an attempt to correct a valid directory name. So we just add the
//! sh:552        # original string in such a case so that the command line doesn't
//! sh:553        # change but other completers still think there are matches.
//! sh:554        # We do this only if we weren't given a `-g' or `-/' option because
//! sh:555        # otherwise this would keep `_files' from completing all filenames
//! sh:556        # if none of the patterns match.
//! sh:557
//! sh:558        if [[ -z "$tpre$tsuf" && -n "$pre$suf" ]]; then
//! sh:559  	pfxsfx=(-S '' "$pfxsfx[@]")
//! sh:560  	### Don't remember what the break was good for. We explicitly
//! sh:561  	### execute this only when there are no matches in the directory,
//! sh:562  	### so why continue?
//! sh:563  	###
//! sh:564          ### tmp1=( "$tmp2[@]" )
//! sh:565  	### break
//! sh:566        elif [[ -n "$haspats" && -z "$tpre$tsuf$suf" && "$pre" = */ ]]; then
//! sh:567  	PREFIX="${opre}"
//! sh:568  	SUFFIX="${osuf}"
//! sh:569          compadd -nQS '' - "$linepath$donepath$orig"
//! sh:570          tmp4=-
//! sh:571        fi
//! sh:572        continue 2
//! sh:573      fi
//! sh:574
//! sh:575      if [[ -n "$ignpar" && -z "$_comp_no_ignore" &&
//! sh:576            "$tpre$tsuf" != */* && $#tmp1 -ne 0 &&
//! sh:577            ( "$ignpar" != *dir* || "$pats" = '*(-/)' ) &&
//! sh:578            ( "$ignpar" != *..* || "$tmp1[1]" = *../* ) ]]; then
//! sh:579
//! sh:580        compfiles -i tmp1 ignore "$ignpar" "$prepath$realpath$donepath"
//! sh:581        _comp_ignore+=( ${(@)ignore#$prepath$realpath$donepath} )
//! sh:582
//! sh:583        (( $#_comp_ignore && ! $mopts[(I)-F] )) &&
//! sh:584            mopts=( "$mopts[@]" -F _comp_ignore )
//! sh:585      fi
//! sh:586
//! sh:587      # Step over to the next component, if any.
//! sh:588
//! sh:589      if [[ "$tpre" = */* ]]; then
//! sh:590        tpre="${tpre#*/}"
//! sh:591      elif [[ "$tsuf" = */* ]]; then
//! sh:592        tpre="${tsuf#*/}"
//! sh:593        tsuf=
//! sh:594      else
//! sh:595        break
//! sh:596      fi
//! sh:597
//! sh:598      # There are more components, so skip over the next components and make a
//! sh:599      # slash be added.
//! sh:600
//! sh:601      #tmp1=( ${tmp1//(#b)([][()|*?^#~<>\\=])/\\${match[1]}} )
//! sh:602      tmp2="${(M)tpre##${~skips}}"
//! sh:603      if [[ -n "$tmp2" ]]; then
//! sh:604        skipped="/$tmp2"
//! sh:605        tpre="${tpre#$tmp2}"
//! sh:606      else
//! sh:607        skipped=/
//! sh:608      fi
//! sh:609      (( npathcheck = 0 ))
//! sh:610    done
//! sh:611
//! sh:612    # The next loop searches the first ambiguous component.
//! sh:613
//! sh:614    tmp3="$pre$suf"
//! sh:615    tpre="$pre"
//! sh:616    tsuf="$suf"
//! sh:617    if [[ -n "${prepath}${realpath}${testpath}" ]]
//! sh:618    then
//! sh:619      if [[ -o nocaseglob ]]
//! sh:620      then
//! sh:621        tmp1=( "${(@)tmp1#(#i)${prepath}${realpath}${testpath}}" )
//! sh:622      else
//! sh:623        tmp1=( "${(@)tmp1#${prepath}${realpath}${testpath}}" )
//! sh:624      fi
//! sh:625    fi
//! sh:626
//! sh:627    while true; do
//! sh:628
//! sh:629      # First we check if some of the files match the original string
//! sh:630      # for this component. If there are some we remove all other
//! sh:631      # names. This avoids having `foo' complete to `foo' and `foobar'.
//! sh:632      # The return value is non-zero if the component is ambiguous.
//! sh:633
//! sh:634      compfiles -r tmp1 "${(Q)tmp3}"
//! sh:635      tmp4=$?
//! sh:636
//! sh:637      if [[ "$tpre" = */* ]]; then
//! sh:638        tmp2="${cpre}${tpre%%/*}"
//! sh:639        PREFIX="${linepath}${donepath}${tmp2}"
//! sh:640        SUFFIX="/${tpre#*/}${tsuf#*/}"
//! sh:641      else
//! sh:642        tmp2="${cpre}${tpre}"
//! sh:643        PREFIX="${linepath}${donepath}${tmp2}"
//! sh:644        SUFFIX="${tsuf}"
//! sh:645      fi
//! sh:646
//! sh:647      # This once tested `|| [[ -n "$compstate[pattern_match]" &&
//! sh:648      # "$tmp2" = (|*[^\\])[][*?#~^\|\<\>]* ]]' but it should now be smart
//! sh:649      # enough to handle multiple components with patterns.
//! sh:650
//! sh:651      if (( tmp4 )); then
//! sh:652        # The component we're checking is ambiguous.
//! sh:653        # For menu completion we now add the possible completions
//! sh:654        # for this component with the unambiguous prefix we have built
//! sh:655        # and the rest of the string from the line as the suffix.
//! sh:656        # For normal completion we add the rests of the filenames
//! sh:657        # collected as the suffixes to make the completion code expand
//! sh:658        # it as far as possible.
//! sh:659
//! sh:660        tmp2="$testpath"
//! sh:661        if [[ -n "$linepath" ]]; then
//! sh:662          compquote -p tmp2 tmp1
//! sh:663        elif [[ -n "$tmp2" ]]; then
//! sh:664          compquote -p tmp1
//! sh:665          compquote tmp2
//! sh:666        else
//! sh:667          compquote tmp1 tmp2
//! sh:668        fi
//! sh:669
//! sh:670        if [[ -z "$_comp_correct" &&
//! sh:671              "$compstate[pattern_match]" = \*  && -n "$listsfx" &&
//! sh:672              "$tmp2" = (|*[^\\])[][*?#~^\|\<\>]* ]]; then
//! sh:673          PREFIX="$opre"
//! sh:674          SUFFIX="$osuf"
//! sh:675        fi
//! sh:676
//! sh:677        # This once tested `-n $menu ||' but our menu-completion expert says
//! sh:678        # that's not what we want.
//! sh:679
//! sh:680        if [[ -z "$compstate[insert]" ]] ||
//! sh:681           { ! zstyle -t ":completion:${curcontext}:paths" expand suffix &&
//! sh:682             [[ -z "$listsfx" &&
//! sh:683                ( -n "$_comp_correct" ||
//! sh:684                  -z "$compstate[pattern_match]" || "$SUFFIX" != */* ||
//! sh:685                  "${SUFFIX#*/}" = (|*[^\\])[][*?#~^\|\<\>]* ) ]] }; then
//! sh:686  	# We have not been told to insert the match, so we are
//! sh:687  	# listing, or something.
//! sh:688          (( tmp4 )) && zstyle -t ":completion:${curcontext}:paths" ambiguous &&
//! sh:689              compstate[to_end]=
//! sh:690          if [[ "$tmp3" = */* ]]; then
//! sh:691  	  if [[ -z "$listsfx" || "$tmp3" != */?* ]]; then
//! sh:692  	    # I think this means we are expanding some directory
//! sh:693  	    # back up the path.
//! sh:694  	    tmp1=("${(@)tmp1%%/*}")
//! sh:695  	    _list_files tmp1 "$prepath$realpath$testpath"
//! sh:696  	    compadd $Uopt -Qf "$mopts[@]" \
//! sh:697                      -p "${Uopt:+$IPREFIX}$linepath$tmp2" \
//! sh:698  	            -s "/${tmp3#*/}${Uopt:+$ISUFFIX}" \
//! sh:699  	            -W "$prepath$realpath$testpath" \
//! sh:700  		    "$pfxsfx[@]" $Mopts \
//! sh:701  		    $listopts \
//! sh:702  	            -a tmp1
//! sh:703            else
//! sh:704  	    # Same with a non-empty suffix
//! sh:705  	    tmp1=("${(@)^tmp1%%/*}/${tmp3#*/}")
//! sh:706  	    _list_files tmp1 "$prepath$realpath$testpath"
//! sh:707  	    compadd $Uopt -Qf "$mopts[@]" \
//! sh:708                      -p "${Uopt:+$IPREFIX}$linepath$tmp2" \
//! sh:709  	            -s "${Uopt:+$ISUFFIX}" \
//! sh:710  	            -W "$prepath$realpath$testpath" \
//! sh:711  		    "$pfxsfx[@]" $Mopts \
//! sh:712  	            $listopts \
//! sh:713  		    -a tmp1
//! sh:714            fi
//! sh:715  	else
//! sh:716  	  _list_files tmp1 "$prepath$realpath$testpath"
//! sh:717  	  compadd $Uopt -Qf "$mopts[@]" -p "${Uopt:+$IPREFIX}$linepath$tmp2" \
//! sh:718  	          -s "${Uopt:+$ISUFFIX}" \
//! sh:719  	          -W "$prepath$realpath$testpath" \
//! sh:720  		   "$pfxsfx[@]" $Mopts \
//! sh:721  	           $listopts \
//! sh:722  		   -a tmp1
//! sh:723  	fi
//! sh:724        else
//! sh:725  	# We are inserting the match into the command line.
//! sh:726          if [[ "$tmp3" = */* ]]; then
//! sh:727  	  tmp4=( $Uopt -Qf "$mopts[@]" -p "${Uopt:+$IPREFIX}$linepath$tmp2"
//! sh:728  	         -W "$prepath$realpath$testpath"
//! sh:729  	         "$pfxsfx[@]" $Mopts )
//! sh:730  	  if [[ -z "$listsfx" ]]; then
//! sh:731              for i in "$tmp1[@]"; do
//! sh:732  	      tmpdisp=("$i")
//! sh:733  	      _list_files tmpdisp "$prepath$realpath$testpath"
//! sh:734  	      compadd "$tmp4[@]" -s "${Uopt:+$ISUFFIX}" $listopts - "$tmpdisp"
//! sh:735  	    done
//! sh:736            else
//! sh:737              [[ -n "$compstate[pattern_match]" ]] && SUFFIX="${SUFFIX:gs./.*/}*"
//! sh:738
//! sh:739              for i in "$tmp1[@]"; do
//! sh:740  	      _list_files i "$prepath$realpath$testpath"
//! sh:741  	      compadd "$tmp4[@]" $listopts - "$i"
//! sh:742  	    done
//! sh:743            fi
//! sh:744          else
//! sh:745  	  _list_files tmp1 "$prepath$realpath$testpath"
//! sh:746  	  compadd $Uopt -Qf "$mopts[@]" -p "${Uopt:+$IPREFIX}$linepath$tmp2" \
//! sh:747  	          -s "${Uopt:+$ISUFFIX}" \
//! sh:748                    -W "$prepath$realpath$testpath" \
//! sh:749  		  "$pfxsfx[@]" $Mopts \
//! sh:750                    $listopts \
//! sh:751  		  -a tmp1
//! sh:752          fi
//! sh:753        fi
//! sh:754        tmp4=-
//! sh:755        # Found an ambiguity, stop the loop over components.
//! sh:756        break
//! sh:757      fi
//! sh:758
//! sh:759      # If we have checked all components, we stop now and add the
//! sh:760      # strings collected after the loop.
//! sh:761
//! sh:762      if [[ "$tmp3" != */* ]]; then
//! sh:763        tmp4=
//! sh:764        break
//! sh:765      fi
//! sh:766
//! sh:767      # Otherwise we add the unambiguous component to `testpath' and
//! sh:768      # take it from the filenames.
//! sh:769
//! sh:770      testpath="${testpath}${tmp1[1]%%/*}/"
//! sh:771
//! sh:772      tmp3="${tmp3#*/}"
//! sh:773
//! sh:774      if [[ "$tpre" = */* ]]; then
//! sh:775        if [[ -z "$_comp_correct" && -n "$compstate[pattern_match]" &&
//! sh:776              "$tmp2" = (|*[^\\])[][*?#~^\|\<\>]* ]]; then
//! sh:777          cpre="${cpre}${tmp1[1]%%/*}/"
//! sh:778        else
//! sh:779          cpre="${cpre}${tpre%%/*}/"
//! sh:780        fi
//! sh:781        tpre="${tpre#*/}"
//! sh:782      elif [[ "$tsuf" = */* ]]; then
//! sh:783        [[ "$tsuf" != /* ]] && mid="$testpath"
//! sh:784        if [[ -z "$_comp_correct" && -n "$compstate[pattern_match]" &&
//! sh:785              "$tmp2" = (|*[^\\])[][*?#~^\|\<\>]* ]]; then
//! sh:786          cpre="${cpre}${tmp1[1]%%/*}/"
//! sh:787        else
//! sh:788          cpre="${cpre}${tpre}/"
//! sh:789        fi
//! sh:790        tpre="${tsuf#*/}"
//! sh:791        tsuf=
//! sh:792      else
//! sh:793        tpre=
//! sh:794        tsuf=
//! sh:795      fi
//! sh:796
//! sh:797      tmp1=( "${(@)tmp1#*/}" )
//! sh:798    done
//! sh:799
//! sh:800    if [[ -z "$tmp4" ]]; then
//! sh:801      # I think this means it's finally time to add the matches,
//! sh:802      # now we've collected contributions from all components.
//! sh:803      if [[ "$mid" = */ ]]; then
//! sh:804        # This seems to mean we're completing in the middle of the
//! sh:805        # command line argument, i.e. not in the last component.
//! sh:806        # There are two cases, depending on whether this part of
//! sh:807        # the path itself has multiple directories or not.
//! sh:808        PREFIX="${opre}"
//! sh:809        SUFFIX="${osuf}"
//! sh:810
//! sh:811        tmp4="${testpath#${mid}}"
//! sh:812        if [[ $mid = */*/* ]]; then
//! sh:813  	# Multiple levels of directory involved.
//! sh:814  	tmp3="${mid%/*/}"
//! sh:815  	tmp2="${${mid%/}##*/}"
//! sh:816  	if [[ -n "$linepath" ]]; then
//! sh:817            compquote -p tmp3
//! sh:818  	else
//! sh:819            compquote tmp3
//! sh:820  	fi
//! sh:821  	compquote tmp4 tmp2 tmp1
//! sh:822  	for i in "$tmp1[@]"; do
//! sh:823  	  _list_files tmp2 "$prepath$realpath${mid%/*/}"
//! sh:824            compadd $Uopt -Qf "$mopts[@]" -p "${Uopt:+$IPREFIX}$linepath$tmp3/" \
//! sh:825  	    -s "/$tmp4$i${Uopt:+$ISUFFIX}" \
//! sh:826              -W "$prepath$realpath${mid%/*/}/" \
//! sh:827  	    "$pfxsfx[@]" $Mopts $listopts - "$tmp2"
//! sh:828  	done
//! sh:829        else
//! sh:830  	# Simpler case with fewer directories: avoid double counting.
//! sh:831  	tmp2="${${mid%/}##*/}"
//! sh:832  	compquote tmp4 tmp2 tmp1
//! sh:833  	for i in "$tmp1[@]"; do
//! sh:834  	  _list_files tmp2 "$prepath$realpath${mid%/*/}"
//! sh:835            compadd $Uopt -Qf "$mopts[@]" -p "${Uopt:+$IPREFIX}$linepath" \
//! sh:836  	    -s "/$tmp4$i${Uopt:+$ISUFFIX}" \
//! sh:837              -W "$prepath$realpath" \
//! sh:838  	    "$pfxsfx[@]" $Mopts $listopts - "$tmp2"
//! sh:839  	done
//! sh:840        fi
//! sh:841      else
//! sh:842        # This would seem to be where we're completing the last
//! sh:843        # component of the path -- the normal one, in other words.
//! sh:844        if [[ "$osuf" = */* ]]; then
//! sh:845          PREFIX="${opre}${osuf}"
//! sh:846          SUFFIX=
//! sh:847        else
//! sh:848          PREFIX="${opre}"
//! sh:849          SUFFIX="${osuf}"
//! sh:850        fi
//! sh:851        tmp4="$testpath"
//! sh:852        if [[ -n "$linepath" ]]; then
//! sh:853          compquote -p tmp4 tmp1
//! sh:854        elif [[ -n "$tmp4" ]]; then
//! sh:855          compquote -p tmp1
//! sh:856          compquote tmp4
//! sh:857        else
//! sh:858          compquote tmp4 tmp1
//! sh:859        fi
//! sh:860        if [[ -z "$_comp_correct" && -n "$compstate[pattern_match]" &&
//! sh:861              "${PREFIX#\~}$SUFFIX" = (|*[^\\])[][*?#~^\|\<\>]* ]]; then
//! sh:862  	# Pattern match, we need to be clever with matchers.
//! sh:863  	tmp1=("$linepath$tmp4${(@)^tmp1}")
//! sh:864  	_list_files tmp1 "$prepath$realpath"
//! sh:865          compadd -Qf -W "$prepath$realpath" "$pfxsfx[@]" "$mopts[@]" \
//! sh:866                  -M "r:|/=* r:|=*" $listopts -a tmp1
//! sh:867        else
//! sh:868  	# Not a pattern match
//! sh:869  	_list_files tmp1 "$prepath$realpath$testpath"
//! sh:870          compadd $Uopt -Qf -p "${Uopt:+$IPREFIX}$linepath$tmp4" \
//! sh:871  	        -s "${Uopt:+$ISUFFIX}" \
//! sh:872  	        -W "$prepath$realpath$testpath" \
//! sh:873  	        "$pfxsfx[@]" "$mopts[@]" $Mopts $listopts -a tmp1
//! sh:874        fi
//! sh:875      fi
//! sh:876    fi
//! sh:877  done
//! sh:878
//! sh:879  # If we are configured to expand paths as far as possible and we collected
//! sh:880  # expanded paths that are different from the string on the line, we add
//! sh:881  # them as possible matches. Do that only if we are currently trying the
//! sh:882  # last entry in the matcher-list style, otherwise other match specs might
//! sh:883  # make the suffix that didn't match this time match in one of the following
//! sh:884  # attempts.
//! sh:885
//! sh:886  if [[ _matcher_num -eq ${#_matchers} ]] &&
//! sh:887     zstyle -t ":completion:${curcontext}:paths" expand prefix &&
//! sh:888     [[ nm -eq compstate[nmatches] && $#exppaths -ne 0 &&
//! sh:889        "$linepath$exppaths" != "$eorig" ]]; then
//! sh:890    PREFIX="${opre}"
//! sh:891    SUFFIX="${osuf}"
//! sh:892    compadd -Q "$mopts[@]" -S '' -M "r:|/=* r:|=*" -p "$linepath" -a exppaths
//! sh:893  fi
//! sh:894
//! sh:895  [[ nm -ne compstate[nmatches] ]]
//! ```
//!
//! Simplified Rust port: exposes `PathFilesOpts` for caller-side
//! flag construction (`-W` → search_dirs, `-g` → glob, `-/` →
//! dirs_only, `-S` → suffix, `-P` → prefix, etc.) and walks the
//! directory tree with prefix-match + glob filter + extension/
//! permission classification (`/` for dir, `@` for symlink, `*`
//! for executable, NOSPACE on dir entries). Drops the cdpath
//! integration + glob-qualifier parsing (deferred to caller).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
