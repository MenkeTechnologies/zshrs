//! Port of `_email_addresses` from `Completion/Unix/Type/_email_addresses`.
//!
//! Full upstream body (187 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2  # options:
//! sh:  3  #
//! sh:  4  # -n plugin - can complete nicknames from specified plugin
//! sh:  5  # -s sep    - complete a list of addresses separated by specified character
//! sh:  6  # -c        - e-mail address must be of form user@host (no comments or aliases)
//! sh:  7  #
//! sh:  8  # TODO: with -n, have the named plugin complete not only aliases but also addresses?
//! sh:  9  #
//! sh: 10  # Plugins are written as separate functions with names starting `_email-'.
//! sh: 11  # They should either do their own completion or return the addresses in the
//! sh: 12  # reply array in the form 'alias:address' and return 300. The -c option is
//! sh: 13  # passed on to plugins (and -n could be if needed ever). New plugins will be
//! sh: 14  # picked up and run automatically.
//! sh: 15
//! sh: 16  # plugins
//! sh: 17  (( $+functions[_email-mail] )) ||
//! sh: 18  _email-mail() {
//! sh: 19    local rc rcfiles i
//! sh: 20
//! sh: 21    rcfiles=( $files[$plugin] )
//! sh: 22    for ((i=1;i<=$#rcfiles;i++)); do
//! sh: 23      rcfiles+=( ${~${(M)${(f)"$(<$rcfiles[i])"}:#source*}##source[[:blank:]]##}(N) )
//! sh: 24    done
//! sh: 25    reply=()
//! sh: 26    for rc in $rcfiles; do
//! sh: 27      reply+=( ${${${(M)${(f)"$(<$rc)"}:#alias*}##alias[[:blank:]]##}/[[:blank:]]##/:} )
//! sh: 28    done
//! sh: 29    return 300
//! sh: 30  }
//! sh: 31  (( $+functions[_email-mutt] )) || _email-mutt() { _email-mail }
//! sh: 32  (( $+functions[_email-mush] )) || _email-mush() { _email-mail }
//! sh: 33
//! sh: 34  (( $+functions[_email-MH] )) ||
//! sh: 35  _email-MH() {
//! sh: 36    reply=( ${${(f)"$(_call_program aliases ali 2>/dev/null)"}/: /:} )
//! sh: 37    return 300
//! sh: 38  }
//! sh: 39
//! sh: 40  (( $+functions[_email-pine] )) ||
//! sh: 41  _email-pine() {
//! sh: 42    reply=( ${${${${${(f)"$(<~/.addressbook)"}:#*DELETED*}:#\ *}/	[^	]#	/:}%%	*} )
//! sh: 43    return 300
//! sh: 44  }
//! sh: 45
//! sh: 46  (( $+functions[_email-ldap] )) ||
//! sh: 47  _email-ldap() {
//! sh: 48    local -a expl ali res filter
//! sh: 49    local -A opts
//! sh: 50    local dn cn mail
//! sh: 51
//! sh: 52    zparseopts -D -E -A opts c
//! sh: 53
//! sh: 54    zstyle -a ":completion:${curcontext}:$curtag" filter filter
//! sh: 55    (( $#filter )) || return
//! sh: 56
//! sh: 57    filter=( "("${filter}"=${PREFIX}*${SUFFIX})" )
//! sh: 58    (( $#filter > 1 )) && filter="(|"${(j..)filter}")"
//! sh: 59    res=( ${(f)"$(_call_program $curtag ldapsearch -LLL \$filter cn mail 2>/dev/null)"} )
//! sh: 60    (( $#res > 1 )) || return
//! sh: 61
//! sh: 62    for dn cn mail in "${res[@]}"; do
//! sh: 63      if (( $+opts[-c] )); then
//! sh: 64        ali+=( "${mail#*: }" )
//! sh: 65      else
//! sh: 66        cn="${cn#*: }"
//! sh: 67        [[ $cn = *$~__specials* ]] && cn="\"$cn\""
//! sh: 68        ali+=( "$cn <${mail#*: }>" )
//! sh: 69      fi
//! sh: 70    done
//! sh: 71    compstate[insert]=menu
//! sh: 72    _wanted email-ldap expl 'matching name' \
//! sh: 73        compadd -U -i "$IPREFIX" -I "$ISUFFIX" "$@" -a - ali
//! sh: 74  }
//! sh: 75
//! sh: 76  (( $+functions[_email-local] )) ||
//! sh: 77  _email-local() {
//! sh: 78    local suf opts
//! sh: 79    zparseopts -D -E -A opts c S:=suf
//! sh: 80
//! sh: 81    if compset -P '*@'; then
//! sh: 82      _hosts "$@" "$suf[@]"
//! sh: 83    else
//! sh: 84      suf=()
//! sh: 85      compset -S '@*' || suf=( -qS @ )
//! sh: 86      _users "$suf[@]" "$@"
//! sh: 87    fi
//! sh: 88  }
//! sh: 89
//! sh: 90  _email_addresses() {
//! sh: 91    local -a plugins reply list args
//! sh: 92    local -A opts files
//! sh: 93    local plugin rcfile muttrc expl sep ret fret
//! sh: 94
//! sh: 95    local __specialx='][()<>@,;:\\".'
//! sh: 96    local __spacex=" 	"				# Space, tab
//! sh: 97    local __specials="[$__specialx]"
//! sh: 98    local __atom="[^$__specialx$__spacex]##"
//! sh: 99    local __space="[$__spacex]#"				# Really, space or comment
//! sh:100    local __qtext='[^"\\]'
//! sh:101    local __qpair='\\?'
//! sh:102    local __beginq='"'
//! sh:103    local __endq='(|[^\\])"'
//! sh:104    local __dot="$__space.$__space"
//! sh:105
//! sh:106    local __domainref="$__atom"
//! sh:107    local __domainlit='\[([^]]|'"$__qpair"')#(|[^\\])\]'
//! sh:108    local __quotedstring="$__beginq($__qtext|$__qpair)#$__endq"
//! sh:109    local __word="($__atom|$__quotedstring)"
//! sh:110    local __phrase="($__space$__word$__space)#"		# Strictly, should use `##'
//! sh:111    local __localpart="$__word($__dot$__word)#"
//! sh:112
//! sh:113    local __subdomain="($__domainref|$__domainlit)"
//! sh:114    local __domain="$__subdomain($__dot$__subdomain)#"
//! sh:115    local __addrspec="$__localpart$__space@$__space$__domain"
//! sh:116
//! sh:117    local __addresses="($__qtext|$__quotedstring)##"
//! sh:118
//! sh:119    zparseopts -D -E -A opts n: s: c
//! sh:120    set -- "$@" -M 'r:|[.@]=* r:|=* m:{a-zA-Z}={A-Za-z}'
//! sh:121
//! sh:122    if [[ -n $opts[-s] ]]; then
//! sh:123      # remove up to the last unquoted separator
//! sh:124      if [[ ${(Q)PREFIX} = (#b)($~__addresses$opts[-s])* ]]; then
//! sh:125        IFS="$opts[-s]" eval 'compset -P $(( ${#${=${:-x${match[1]}x}}} - 1 )) "*${opts[-s]}"'
//! sh:126      fi
//! sh:127
//! sh:128      # for the suffix, I'm too lazy to work out how to preserve quoted separators
//! sh:129      compset -S "$opts[-s]*" || set -- -q -S "$opts[-s]" "$@"
//! sh:130    fi
//! sh:131
//! sh:132    # get list of all plugins except any with missing config files
//! sh:133    if ! zstyle -s ":completion:${curcontext}:email-addresses" muttrc muttrc; then
//! sh:134      [[ -e ~/mutt/muttrc ]] && muttrc="~/mutt/muttrc" || muttrc="~/.muttrc"
//! sh:135    fi
//! sh:136    files=( MH ${MH:-~/.mh_profile} mutt $~muttrc mush ~/.mushrc mail ${MAILRC:-~/.mailrc} pine ~/.addressbook )
//! sh:137    plugins=(
//! sh:138      ${${(k)functions[(I)_email-*]#*-}:#(${(kj.|.)~files})}
//! sh:139      $files(Ne:'REPLY=( ${(k)files[(r)$REPLY]} ):')
//! sh:140    )
//! sh:141
//! sh:142    ret=1
//! sh:143    _tags email-$plugins
//! sh:144    while _tags; do
//! sh:145      for plugin in $plugins; do
//! sh:146        if _requested email-$plugin; then
//! sh:147  	while _next_label email-$plugin expl 'email address'; do
//! sh:148
//! sh:149            args=()
//! sh:150  	  if (( $+opts[-c] )) || zstyle -t \
//! sh:151  	      ":completion:${curcontext}:$curtag" strip-comments
//! sh:152  	  then
//! sh:153  	    args=( '-c' )
//! sh:154  	  fi
//! sh:155
//! sh:156  	  if ! _call_function fret _email-$plugin "$@" $args; then
//! sh:157  	    _message "$plugin: plugin not found"
//! sh:158  	    continue
//! sh:159  	  fi
//! sh:160  	  ret=$(( ret && fret ))
//! sh:161
//! sh:162  	  if (( fret == 300 )); then
//! sh:163  	    if (( ! $+opts[-c] )) && [[ $opts[-n] = $plugin ]]; then
//! sh:164  	      zstyle -s ":completion:${curcontext}:$curtag" list-separator sep || sep=--
//! sh:165  	      zformat -a list " $sep " "${reply[@]}"
//! sh:166  	      _wanted mail-aliases expl 'alias' compadd "$@" \
//! sh:167  		  -d list - ${reply%%:*} && ret=0
//! sh:168  	    else
//! sh:169  	      if (( $#args )); then
//! sh:170  		reply=( ${(SM)${reply#*:}##$~__addrspec} )
//! sh:171  	      else
//! sh:172  		# remove lines not containing `@' as they probably aren't addresses
//! sh:173  		reply=( "${(@)${(M@)reply:#*@*}#*:}" )
//! sh:174  	      fi
//! sh:175  	      compadd -a "$@" "$expl[@]" reply && ret=0
//! sh:176  	    fi
//! sh:177  	  fi
//! sh:178  	done
//! sh:179        fi
//! sh:180      done
//! sh:181      (( ret )) || return 0
//! sh:182    done
//! sh:183
//! sh:184    return 1
//! sh:185  }
//! sh:186
//! sh:187  _email_addresses "$@"
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
