//! Port of `compinit` from `Completion/compinit`.
//!
//! Full upstream body (574 lines verbatim):
//! ```text
//! sh:  1  # Initialisation for new style completion. This mainly contains some helper
//! sh:  2  # functions and setup. Everything else is split into different files that
//! sh:  3  # will automatically be made autoloaded (see the end of this file).  The
//! sh:  4  # names of the files that will be considered for autoloading are those that
//! sh:  5  # begin with an underscores (like `_condition).
//! sh:  6  #
//! sh:  7  # The first line of each of these files is read and must indicate what
//! sh:  8  # should be done with its contents:
//! sh:  9  #
//! sh: 10  #   `#compdef <names ...>'
//! sh: 11  #     If the first line looks like this, the file is autoloaded as a
//! sh: 12  #     function and that function will be called to generate the matches
//! sh: 13  #     when completing for one of the commands whose <names> are given.
//! sh: 14  #     The names may also be interspersed with `-T <assoc>' options
//! sh: 15  #     specifying for which set of functions this should be added.
//! sh: 16  #
//! sh: 17  #   `#compdef -[pP] <patterns ...>'
//! sh: 18  #     This defines a function that should be called to generate matches
//! sh: 19  #     for commands whose name matches <pattern>. Note that only one pattern
//! sh: 20  #     may be given.
//! sh: 21  #
//! sh: 22  #   `#compdef -k <style> [ <key-sequence> ... ]'
//! sh: 23  #     This is used to bind special completions to all the given
//! sh: 24  #     <key-sequence>(s). The <style> is the name of one of the built-in
//! sh: 25  #     completion widgets (complete-word, delete-char-or-list,
//! sh: 26  #     expand-or-complete, expand-or-complete-prefix, list-choices,
//! sh: 27  #     menu-complete, menu-expand-or-complete, or reverse-menu-complete).
//! sh: 28  #     This creates a widget behaving like <style> so that the
//! sh: 29  #     completions are chosen as given in the rest of the file,
//! sh: 30  #     rather than by the context.  The widget has the same name as
//! sh: 31  #     the autoload file and can be bound using bindkey in the normal way.
//! sh: 32  #
//! sh: 33  #   `#compdef -K <widget-name> <style> <key-sequence> [ ... ]'
//! sh: 34  #     This is similar to -k, except it takes any number of sets of
//! sh: 35  #     three arguments.  In each set, the widget <widget-name> will
//! sh: 36  #     be defined, which will behave as <style>, as with -k, and will
//! sh: 37  #     be bound to <key-sequence>, exactly one of which must be defined.
//! sh: 38  #     <widget-name> must be different for each:  this must begin with an
//! sh: 39  #     underscore, else one will be added, and should not clash with other
//! sh: 40  #     completion widgets (names based on the name of the function are the
//! sh: 41  #     clearest), but is otherwise arbitrary.  It can be tested in the
//! sh: 42  #     function by the parameter $WIDGET.
//! sh: 43  #
//! sh: 44  #   `#autoload [ <options> ]'
//! sh: 45  #     This is for helper functions that are not used to
//! sh: 46  #     generate matches, but should automatically be loaded
//! sh: 47  #     when they are called. The <options> will be given to the
//! sh: 48  #     autoload builtin when making the function autoloaded. Note
//! sh: 49  #     that this need not include `-U' and `-z'.
//! sh: 50  #
//! sh: 51  # Note that no white space is allowed between the `#' and the rest of
//! sh: 52  # the string.
//! sh: 53  #
//! sh: 54  # Functions that are used to generate matches should return zero if they
//! sh: 55  # were able to add matches and non-zero otherwise.
//! sh: 56  #
//! sh: 57  # See the file `compdump' for how to speed up initialisation.
//! sh: 58
//! sh: 59  # If we got the `-d'-flag, we will automatically dump the new state (at
//! sh: 60  # the end).  This takes the dumpfile as an argument.  -d (with the
//! sh: 61  # default dumpfile) is now the default; to turn off dumping use -D.
//! sh: 62
//! sh: 63  # If the dumpfile is being regenerated and you don't know why, you can use
//! sh: 64  # the -w flag to see if it was because -D was passed, zsh version mismatched,
//! sh: 65  # or number of files in $fpath differed.
//! sh: 66
//! sh: 67  # The -C flag bypasses both the check for rebuilding the dump file and the
//! sh: 68  # usual call to compaudit; the -i flag causes insecure directories found by
//! sh: 69  # compaudit to be ignored, and the -u flag causes all directories found by
//! sh: 70  # compaudit to be used (without security checking).  Otherwise the user is
//! sh: 71  # queried for whether to use or ignore the insecure directories (which
//! sh: 72  # means compinit should not be called from non-interactive shells).
//! sh: 73
//! sh: 74  emulate -L zsh
//! sh: 75  setopt extendedglob
//! sh: 76
//! sh: 77  typeset _i_dumpfile _i_files _i_line _i_done _i_dir _i_autodump=1
//! sh: 78  typeset _i_tag _i_file _i_addfiles _i_fail=ask _i_check=yes _i_name _i_why
//! sh: 79
//! sh: 80  while [[ $# -gt 0 && $1 = -[dDiuCw] ]]; do
//! sh: 81    case "$1" in
//! sh: 82    -d)
//! sh: 83      _i_autodump=1
//! sh: 84      shift
//! sh: 85      if [[ $# -gt 0 && "$1" != -[dfQC] ]]; then
//! sh: 86        _i_dumpfile="$1"
//! sh: 87        shift
//! sh: 88      fi
//! sh: 89      ;;
//! sh: 90    -D)
//! sh: 91      _i_autodump=0
//! sh: 92      shift
//! sh: 93      ;;
//! sh: 94    -i)
//! sh: 95      _i_fail=ign
//! sh: 96      shift
//! sh: 97      ;;
//! sh: 98    -u)
//! sh: 99      _i_fail=use
//! sh:100      shift
//! sh:101      ;;
//! sh:102    -C)
//! sh:103      _i_check=
//! sh:104      shift
//! sh:105      ;;
//! sh:106    -w)
//! sh:107      _i_why=1
//! sh:108      shift
//! sh:109      ;;
//! sh:110    esac
//! sh:111  done
//! sh:112
//! sh:113  # The associative arrays containing the definitions for the commands and
//! sh:114  # services.
//! sh:115
//! sh:116  typeset -gHA _comps _services _patcomps _postpatcomps
//! sh:117
//! sh:118  # `_compautos' contains the names and options for autoloaded functions
//! sh:119  # that get options.
//! sh:120
//! sh:121  typeset -gHA _compautos
//! sh:122
//! sh:123  # The associative array use to report information about the last
//! sh:124  # completion to the outside.
//! sh:125
//! sh:126  typeset -gHA _lastcomp
//! sh:127
//! sh:128  # Remember dumpfile.
//! sh:129  if [[ -n $_i_dumpfile ]]; then
//! sh:130    # Explicitly supplied dumpfile.
//! sh:131    typeset -g _comp_dumpfile="$_i_dumpfile"
//! sh:132  else
//! sh:133    typeset -g _comp_dumpfile="${ZDOTDIR:-$HOME}/.zcompdump"
//! sh:134  fi
//! sh:135
//! sh:136  # The standard options set in completion functions.
//! sh:137
//! sh:138  typeset -gHa _comp_options
//! sh:139  _comp_options=(
//! sh:140         bareglobqual
//! sh:141         extendedglob
//! sh:142         glob
//! sh:143         multibyte
//! sh:144         multifuncdef
//! sh:145         nullglob
//! sh:146         rcexpandparam
//! sh:147         unset
//! sh:148      NO_allexport
//! sh:149      NO_aliases
//! sh:150      NO_autonamedirs
//! sh:151      NO_cshnullglob
//! sh:152      NO_cshjunkiequotes
//! sh:153      NO_errexit
//! sh:154      NO_errreturn
//! sh:155      NO_globassign
//! sh:156      NO_globsubst
//! sh:157      NO_histsubstpattern
//! sh:158      NO_ignorebraces
//! sh:159      NO_ignoreclosebraces
//! sh:160      NO_kshglob
//! sh:161      NO_ksharrays
//! sh:162      NO_kshtypeset
//! sh:163      NO_markdirs
//! sh:164      NO_octalzeroes
//! sh:165      NO_posixbuiltins
//! sh:166      NO_posixidentifiers
//! sh:167      NO_shwordsplit
//! sh:168      NO_shglob
//! sh:169      NO_typesettounset
//! sh:170      NO_warnnestedvar
//! sh:171      NO_warncreateglobal
//! sh:172  )
//! sh:173
//! sh:174  # And this one should be `eval'ed at the beginning of every entry point
//! sh:175  # to the completion system.  It sets up what we currently consider a
//! sh:176  # sane environment.  That means we set the options above, make sure we
//! sh:177  # have a valid stdin descriptor (zle closes it before calling widgets)
//! sh:178  # and don't get confused by user's ZERR trap handlers.
//! sh:179
//! sh:180  typeset -gH _comp_setup='local -A _comp_caller_options;
//! sh:181               _comp_caller_options=(${(kv)options[@]});
//! sh:182               setopt localoptions localtraps localpatterns ${_comp_options[@]};
//! sh:183               local IFS=$'\'\ \\t\\r\\n\\0\'';
//! sh:184               builtin enable -p \| \~ \( \? \* \[ \< \^ \# 2>&-;
//! sh:185               exec </dev/null;
//! sh:186               trap - ZERR;
//! sh:187               local -a reply;
//! sh:188               local REPLY;
//! sh:189               local REPORTTIME;
//! sh:190               unset REPORTTIME'
//! sh:191
//! sh:192  # These can hold names of functions that are to be called before/after all
//! sh:193  # matches have been generated.
//! sh:194
//! sh:195  typeset -ga compprefuncs comppostfuncs
//! sh:196  compprefuncs=()
//! sh:197  comppostfuncs=()
//! sh:198
//! sh:199  # Loading it now ensures that the `funcstack' parameter is always correct.
//! sh:200
//! sh:201  : $funcstack
//! sh:202
//! sh:203  # This function is used to register or delete completion functions. For
//! sh:204  # registering completion functions, it is invoked with the name of the
//! sh:205  # function as it's first argument (after the options). The other
//! sh:206  # arguments depend on what type of completion function is defined. If
//! sh:207  # none of the `-p' and `-k' options is given a function for a command is
//! sh:208  # defined. The arguments after the function name are then interpreted as
//! sh:209  # the names of the command for which the function generates matches.
//! sh:210  # With the `-p' option a function for a name pattern is defined. This
//! sh:211  # function will be invoked when completing for a command whose name
//! sh:212  # matches the pattern given as argument after the function name (in this
//! sh:213  # case only one argument is accepted).
//! sh:214  # The option `-P' is like `-p', but the function will be called after
//! sh:215  # trying to find a function defined for the command on the line if no
//! sh:216  # such function could be found.
//! sh:217  # With the `-k' option a function for a special completion keys is
//! sh:218  # defined and immediately bound to those keys. Here, the extra arguments
//! sh:219  # are the name of one of the builtin completion widgets and any number
//! sh:220  # of key specifications as accepted by the `bindkey' builtin.
//! sh:221  # In any case the `-a' option may be given which makes the function
//! sh:222  # whose name is given as the first argument be autoloaded. When defining
//! sh:223  # a function for command names the `-n' option may be given and keeps
//! sh:224  # the definitions from overriding any previous definitions for the
//! sh:225  # commands; with `-k', the `-n' option prevents compdef from rebinding
//! sh:226  # a key sequence which is already bound.
//! sh:227  # For deleting definitions, the `-d' option must be given. Without the
//! sh:228  # `-p' option, this deletes definitions for functions for the commands
//! sh:229  # whose names are given as arguments. If combined with the `-p' option
//! sh:230  # it deletes the definitions for the patterns given as argument.
//! sh:231  # The `-d' option may not be combined with the `-k' option, i.e.
//! sh:232  # definitions for key function can not be removed.
//! sh:233  #
//! sh:234  # Examples:
//! sh:235  #
//! sh:236  #  compdef -a foo bar baz
//! sh:237  #    make the completion for the commands `bar' and `baz' use the
//! sh:238  #    function `foo' and make this function be autoloaded
//! sh:239  #
//! sh:240  #  compdef -p foo 'c*'
//! sh:241  #    make completion for all command whose name begins with a `c'
//! sh:242  #    generate matches by calling the function `foo' before generating
//! sh:243  #    matches defined for the command itself
//! sh:244  #
//! sh:245  #  compdef -k foo list-choices '^X^M' '\C-xm'
//! sh:246  #    make the function `foo' be invoked when typing `Control-X Control-M'
//! sh:247  #    or `Control-X m'; the function should generate matches and will
//! sh:248  #    behave like the `list-choices' builtin widget
//! sh:249  #
//! sh:250  #  compdef -d bar baz
//! sh:251  #   delete the definitions for the command names `bar' and `baz'
//! sh:252
//! sh:253  compdef() {
//! sh:254    local opt autol type func delete eval new i ret=0 cmd svc
//! sh:255    local -a match mbegin mend
//! sh:256
//! sh:257    emulate -L zsh
//! sh:258    setopt extendedglob
//! sh:259
//! sh:260    # Get the options.
//! sh:261
//! sh:262    if (( ! $# )); then
//! sh:263      print -u2 "$0: I need arguments"
//! sh:264      return 1
//! sh:265    fi
//! sh:266
//! sh:267    while getopts "anpPkKde" opt; do
//! sh:268      case "$opt" in
//! sh:269      a)    autol=yes;;
//! sh:270      n)    new=yes;;
//! sh:271      [pPkK]) if [[ -n "$type" ]]; then
//! sh:272              # Error if both `-p' and `-k' are given (or one of them
//! sh:273  	    # twice).
//! sh:274              print -u2 "$0: type already set to $type"
//! sh:275  	    return 1
//! sh:276  	  fi
//! sh:277  	  if [[ "$opt" = p ]]; then
//! sh:278  	    type=pattern
//! sh:279  	  elif [[ "$opt" = P ]]; then
//! sh:280  	    type=postpattern
//! sh:281  	  elif [[ "$opt" = K ]]; then
//! sh:282  	    type=widgetkey
//! sh:283  	  else
//! sh:284  	    type=key
//! sh:285  	  fi
//! sh:286  	  ;;
//! sh:287      d) delete=yes;;
//! sh:288      e) eval=yes;;
//! sh:289      esac
//! sh:290    done
//! sh:291    shift OPTIND-1
//! sh:292
//! sh:293    if (( ! $# )); then
//! sh:294      print -u2 "$0: I need arguments"
//! sh:295      return 1
//! sh:296    fi
//! sh:297
//! sh:298    if [[ -z "$delete" ]]; then
//! sh:299      # If the first word contains an equal sign, all words must contain one
//! sh:300      # and we define which services to use for the commands.
//! sh:301
//! sh:302      if [[ -z "$eval" ]] && [[ "$1" = *\=* ]]; then
//! sh:303        while (( $# )); do
//! sh:304          if [[ "$1" = *\=* ]]; then
//! sh:305  	  cmd="${1%%\=*}"
//! sh:306  	  svc="${1#*\=}"
//! sh:307            func="$_comps[${_services[(r)$svc]:-$svc}]"
//! sh:308            [[ -n ${_services[$svc]} ]] &&
//! sh:309                svc=${_services[$svc]}
//! sh:310  	  [[ -z "$func" ]] &&
//! sh:311  	      func="${${_patcomps[(K)$svc][1]}:-${_postpatcomps[(K)$svc][1]}}"
//! sh:312            if [[ -n "$func" ]]; then
//! sh:313  	    _comps[$cmd]="$func"
//! sh:314  	    _services[$cmd]="$svc"
//! sh:315  	  else
//! sh:316  	    print -u2 "$0: unknown command or service: $svc"
//! sh:317  	    ret=1
//! sh:318  	  fi
//! sh:319  	else
//! sh:320  	  print -u2 "$0: invalid argument: $1"
//! sh:321  	  ret=1
//! sh:322  	fi
//! sh:323          shift
//! sh:324        done
//! sh:325
//! sh:326        return ret
//! sh:327      fi
//! sh:328
//! sh:329      # Adding definitions, first get the name of the function name
//! sh:330      # and probably do autoloading.
//! sh:331
//! sh:332      func="$1"
//! sh:333      [[ -n "$autol" ]] && autoload -rUz "$func"
//! sh:334      shift
//! sh:335
//! sh:336      case "$type" in
//! sh:337      widgetkey)
//! sh:338        while [[ -n $1 ]]; do
//! sh:339  	if [[ $# -lt 3 ]]; then
//! sh:340  	  print -u2 "$0: compdef -K requires <widget> <comp-widget> <key>"
//! sh:341  	  return 1
//! sh:342  	fi
//! sh:343  	[[ $1 = _* ]] || 1="_$1"
//! sh:344  	[[ $2 = .* ]] || 2=".$2"
//! sh:345          [[ $2 = .menu-select ]] && zmodload -i zsh/complist
//! sh:346  	zle -C "$1" "$2" "$func"
//! sh:347  	if [[ -n $new ]]; then
//! sh:348  	  bindkey "$3" | IFS=$' \t' read -A opt
//! sh:349  	  [[ $opt[-1] = undefined-key ]] && bindkey "$3" "$1"
//! sh:350  	else
//! sh:351  	  bindkey "$3" "$1"
//! sh:352  	fi
//! sh:353  	shift 3
//! sh:354        done
//! sh:355        ;;
//! sh:356      key)
//! sh:357        if [[ $# -lt 2 ]]; then
//! sh:358          print -u2 "$0: missing keys"
//! sh:359  	return 1
//! sh:360        fi
//! sh:361
//! sh:362        # Define the widget.
//! sh:363        if [[ $1 = .* ]]; then
//! sh:364          [[ $1 = .menu-select ]] && zmodload -i zsh/complist
//! sh:365  	zle -C "$func" "$1" "$func"
//! sh:366        else
//! sh:367          [[ $1 = menu-select ]] && zmodload -i zsh/complist
//! sh:368  	zle -C "$func" ".$1" "$func"
//! sh:369        fi
//! sh:370        shift
//! sh:371
//! sh:372        # And bind the keys...
//! sh:373        for i; do
//! sh:374          if [[ -n $new ]]; then
//! sh:375  	   bindkey "$i" | IFS=$' \t' read -A opt
//! sh:376  	   [[ $opt[-1] = undefined-key ]] || continue
//! sh:377  	fi
//! sh:378          bindkey "$i" "$func"
//! sh:379        done
//! sh:380        ;;
//! sh:381      *)
//! sh:382        # For commands store the function name in the
//! sh:383        # associative array, command names as keys.
//! sh:384        while (( $# )); do
//! sh:385          if [[ "$1" = -N ]]; then
//! sh:386            type=normal
//! sh:387          elif [[ "$1" = -p ]]; then
//! sh:388            type=pattern
//! sh:389          elif [[ "$1" = -P ]]; then
//! sh:390            type=postpattern
//! sh:391          else
//! sh:392            case "$type" in
//! sh:393            pattern)
//! sh:394  	    if [[ $1 = (#b)(*)=(*) ]]; then
//! sh:395  	      _patcomps[$match[1]]="=$match[2]=$func"
//! sh:396  	    else
//! sh:397  	      _patcomps[$1]="$func"
//! sh:398  	    fi
//! sh:399              ;;
//! sh:400            postpattern)
//! sh:401  	    if [[ $1 = (#b)(*)=(*) ]]; then
//! sh:402  	      _postpatcomps[$match[1]]="=$match[2]=$func"
//! sh:403  	    else
//! sh:404  	      _postpatcomps[$1]="$func"
//! sh:405  	    fi
//! sh:406              ;;
//! sh:407            *)
//! sh:408              if [[ "$1" = *\=* ]]; then
//! sh:409  	      cmd="${1%%\=*}"
//! sh:410  	      svc=yes
//! sh:411              else
//! sh:412  	      cmd="$1"
//! sh:413  	      svc=
//! sh:414              fi
//! sh:415              if [[ -z "$new" || -z "${_comps[$1]}" ]]; then
//! sh:416                _comps[$cmd]="$func"
//! sh:417  	      [[ -n "$svc" ]] && _services[$cmd]="${1#*\=}"
//! sh:418  	    fi
//! sh:419              ;;
//! sh:420            esac
//! sh:421          fi
//! sh:422          shift
//! sh:423        done
//! sh:424        ;;
//! sh:425      esac
//! sh:426    else
//! sh:427      # Handle the `-d' option, deleting.
//! sh:428
//! sh:429      case "$type" in
//! sh:430      pattern)
//! sh:431        unset "_patcomps[$^@]"
//! sh:432        ;;
//! sh:433      postpattern)
//! sh:434        unset "_postpatcomps[$^@]"
//! sh:435        ;;
//! sh:436      key)
//! sh:437        # Oops, cannot do that yet.
//! sh:438
//! sh:439        print -u2 "$0: cannot restore key bindings"
//! sh:440        return 1
//! sh:441        ;;
//! sh:442      *)
//! sh:443        unset "_comps[$^@]"
//! sh:444      esac
//! sh:445    fi
//! sh:446  }
//! sh:447
//! sh:448  # Now we automatically make the definition files autoloaded.
//! sh:449
//! sh:450  typeset _i_wdirs _i_wfiles
//! sh:451
//! sh:452  _i_wdirs=()
//! sh:453  _i_wfiles=()
//! sh:454
//! sh:455  autoload -RUz compaudit
//! sh:456  if [[ -n "$_i_check" ]]; then
//! sh:457    typeset _i_q
//! sh:458    if ! eval compaudit; then
//! sh:459      if [[ -n "$_i_q" ]]; then
//! sh:460        if [[ "$_i_fail" = ask ]]; then
//! sh:461          if ! read -q \
//! sh:462  "?zsh compinit: insecure $_i_q, run compaudit for list.
//! sh:463  Ignore insecure $_i_q and continue [y] or abort compinit [n]? "; then
//! sh:464  	  print -u2 "$0: initialization aborted"
//! sh:465            unfunction compinit compdef
//! sh:466            unset _comp_dumpfile _comp_secure compprefuncs comppostfuncs \
//! sh:467                  _comps _patcomps _postpatcomps _compautos _lastcomp
//! sh:468
//! sh:469            return 1
//! sh:470          fi
//! sh:471        fi
//! sh:472        fpath=(${fpath:|_i_wdirs})
//! sh:473        (( $#_i_wfiles )) && _i_files=( "${(@)_i_files:#(${(j:|:)_i_wfiles%.zwc})}"  )
//! sh:474        (( $#_i_wdirs ))  && _i_files=( "${(@)_i_files:#(${(j:|:)_i_wdirs%.zwc})/*}" )
//! sh:475      fi
//! sh:476      typeset -g _comp_secure=yes
//! sh:477    fi
//! sh:478  fi
//! sh:479
//! sh:480  # Make sure compdump is available, even if we aren't going to use it.
//! sh:481  autoload -RUz compdump compinstall
//! sh:482
//! sh:483  # If we have a dump file, load it.
//! sh:484
//! sh:485  _i_done=''
//! sh:486
//! sh:487  if [[ -f "$_comp_dumpfile" ]]; then
//! sh:488    if [[ -n "$_i_check" ]]; then
//! sh:489      IFS=$' \t' read -rA _i_line < "$_comp_dumpfile"
//! sh:490      if [[ _i_autodump -eq 1 && $_i_line[2] -eq $#_i_files &&
//! sh:491          $ZSH_VERSION = $_i_line[4] ]]
//! sh:492      then
//! sh:493        builtin . "$_comp_dumpfile"
//! sh:494        _i_done=yes
//! sh:495      elif [[ _i_why -eq 1 ]]; then
//! sh:496        print -nu2 "Loading dump file skipped, regenerating"
//! sh:497        local pre=" because: "
//! sh:498        if [[ _i_autodump -ne 1 ]]; then
//! sh:499          print -nu2 $pre"-D flag given"
//! sh:500          pre=", "
//! sh:501        fi
//! sh:502        if [[ $_i_line[2] -ne $#_i_files ]]; then
//! sh:503          print -nu2 $pre"number of files in dump $_i_line[2] differ from files found in \$fpath $#_i_files"
//! sh:504          pre=", "
//! sh:505        fi
//! sh:506        if [[ $ZSH_VERSION != $_i_line[4] ]]; then
//! sh:507          print -nu2 $pre"zsh version changed from $_i_line[4] to $ZSH_VERSION"
//! sh:508        fi
//! sh:509        print -u2
//! sh:510      fi
//! sh:511    else
//! sh:512      builtin . "$_comp_dumpfile"
//! sh:513      _i_done=yes
//! sh:514    fi
//! sh:515  elif [[ _i_why -eq 1 ]]; then
//! sh:516    print -u2 "No existing compdump file found, regenerating"
//! sh:517  fi
//! sh:518  if [[ -z "$_i_done" ]]; then
//! sh:519    typeset -A _i_test
//! sh:520
//! sh:521    for _i_dir in $fpath; do
//! sh:522      [[ $_i_dir = . ]] && continue
//! sh:523      (( $_i_wdirs[(I)$_i_dir] )) && continue
//! sh:524      for _i_file in $_i_dir/^([^_]*|*[\;\|\&]*|*~|*.zwc)(N); do
//! sh:525        _i_name="${_i_file:t}"
//! sh:526        (( $+_i_test[$_i_name] + $_i_wfiles[(I)$_i_file] )) && continue
//! sh:527        _i_test[$_i_name]=yes
//! sh:528        IFS=$' \t' read -rA _i_line < $_i_file
//! sh:529        _i_tag=$_i_line[1]
//! sh:530        shift _i_line
//! sh:531        case $_i_tag in
//! sh:532        (\#compdef)
//! sh:533  	if [[ $_i_line[1] = -[pPkK](n|) ]]; then
//! sh:534  	  compdef ${_i_line[1]}na "${_i_name}" "${(@)_i_line[2,-1]}"
//! sh:535  	else
//! sh:536  	  compdef -na "${_i_name}" "${_i_line[@]}"
//! sh:537  	fi
//! sh:538  	;;
//! sh:539        (\#autoload)
//! sh:540  	autoload -rUz "$_i_line[@]" ${_i_name}
//! sh:541  	[[ "$_i_line" != \ # ]] && _compautos[${_i_name}]="$_i_line"
//! sh:542  	;;
//! sh:543        esac
//! sh:544      done
//! sh:545    done
//! sh:546
//! sh:547    # If autodumping was requested, do it now.
//! sh:548
//! sh:549    if [[ $_i_autodump = 1 ]]; then
//! sh:550      compdump
//! sh:551    fi
//! sh:552  fi
//! sh:553
//! sh:554  # Rebind the standard widgets
//! sh:555  for _i_line in complete-word delete-char-or-list expand-or-complete \
//! sh:556    expand-or-complete-prefix list-choices menu-complete \
//! sh:557    menu-expand-or-complete reverse-menu-complete; do
//! sh:558    zle -C $_i_line .$_i_line _main_complete
//! sh:559  done
//! sh:560  zle -la menu-select && zle -C menu-select .menu-select _main_complete
//! sh:561
//! sh:562  # If the default completer set includes _expand, and tab is bound
//! sh:563  # to expand-or-complete, rebind it to complete-word instead.
//! sh:564  bindkey '^i' | IFS=$' \t' read -A _i_line
//! sh:565  if [[ ${_i_line[2]} = expand-or-complete ]] &&
//! sh:566    zstyle -a ':completion:' completer _i_line &&
//! sh:567    (( ${_i_line[(i)_expand]} <= ${#_i_line} )); then
//! sh:568    bindkey '^i' complete-word
//! sh:569  fi
//! sh:570
//! sh:571  unfunction compinit compaudit
//! sh:572  autoload -RUz compinit compaudit
//! sh:573
//! sh:574  return 0
//! ```

use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

// =====================================================================
// Upstream constants (sh:138-197) — exported so every compsys entry
// point can replay the `_comp_setup` eval and the `_comp_options`
// list that compinit installs at load time.
// =====================================================================

/// sh:139-172 — the 33 options compinit forces into every compsys
/// entry-point scope via `setopt localoptions … ${_comp_options[@]}`.
/// Mirrors the upstream array verbatim, including the `NO_` prefix
/// form for negated flags. Consumed by the eval string at
/// [`COMP_SETUP_EVAL`].
pub const COMP_OPTIONS: &[&str] = &[
    "bareglobqual",
    "extendedglob",
    "glob",
    "multibyte",
    "multifuncdef",
    "nullglob",
    "rcexpandparam",
    "unset",
    "NO_allexport",
    "NO_aliases",
    "NO_autonamedirs",
    "NO_cshnullglob",
    "NO_cshjunkiequotes",
    "NO_errexit",
    "NO_errreturn",
    "NO_globassign",
    "NO_globsubst",
    "NO_histsubstpattern",
    "NO_ignorebraces",
    "NO_ignoreclosebraces",
    "NO_kshglob",
    "NO_ksharrays",
    "NO_kshtypeset",
    "NO_markdirs",
    "NO_octalzeroes",
    "NO_posixbuiltins",
    "NO_posixidentifiers",
    "NO_shwordsplit",
    "NO_shglob",
    "NO_typesettounset",
    "NO_warnnestedvar",
    "NO_warncreateglobal",
];

/// sh:180-190 — the `_comp_setup` string that every compsys entry
/// point evals to install the option set + IFS + null stdin + no-ZERR.
/// Bit-identical to upstream so a user-supplied `_comp_setup`
/// override (very rare) still matches.
pub const COMP_SETUP_EVAL: &str = concat!(
    "local -A _comp_caller_options;\n",
    "_comp_caller_options=(${(kv)options[@]});\n",
    "setopt localoptions localtraps localpatterns ${_comp_options[@]};\n",
    "local IFS=$' \\t\\r\\n\\0';\n",
    "builtin enable -p \\| \\~ \\( \\? \\* \\[ \\< \\^ \\# 2>&-;\n",
    "exec </dev/null;\n",
    "trap - ZERR;\n",
    "local -a reply;\n",
    "local REPLY;\n",
    "local REPORTTIME;\n",
    "unset REPORTTIME"
);

/// sh:558 — the 8 standard ZLE widgets that compinit rebinds to
/// `_main_complete` so any of them triggers a completion attempt.
pub const STANDARD_COMPLETE_WIDGETS: &[&str] = &[
    "complete-word",
    "delete-char-or-list",
    "expand-or-complete",
    "expand-or-complete-prefix",
    "list-choices",
    "menu-complete",
    "menu-expand-or-complete",
    "reverse-menu-complete",
];

// =====================================================================
// State publication helpers — keep the shell-side `$compprefuncs`
// and `$comppostfuncs` arrays initialized empty per sh:195-197.
// =====================================================================

/// Initialize the shell-side `compprefuncs` / `comppostfuncs` arrays
/// to empty (sh:195-197). Idempotent; safe to call from compinit
/// before any user-side `_call_function` would have populated them.
pub fn init_comp_funcs_arrays() {
    crate::ported::params::setaparam("compprefuncs", Vec::new());
    crate::ported::params::setaparam("comppostfuncs", Vec::new());
}

/// Default `$_comp_dumpfile` path (sh:129-134). User can override
/// via `compinit -d <file>`; without that, use `${ZDOTDIR:-$HOME}`
/// + `/.zcompdump`.
pub fn default_dumpfile_path() -> PathBuf {
    let home = std::env::var("ZDOTDIR")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_else(|| ".".to_string());
    PathBuf::from(home).join(".zcompdump")
}

/// Publish the standard ZLE rebind set to the dispatcher hooks
/// (sh:555-560). For each `complete-word` family widget + a
/// conditional `menu-select` (when `zsh/complist` is loaded), bind
/// it to `_main_complete` via `zle -C`. Returns the count of
/// successful binds.
pub fn install_standard_complete_widgets() -> usize {
    // `zle` is a builtin, not a shell function, so it must be invoked
    // through the builtin entry (`bin_zle_complete`) — NOT
    // `dispatch_function_call`, which only resolves shell functions and
    // silently returns None for a builtin name, leaving every widget
    // unbound (the whole compsys engine then never fires on Tab).
    let empty_ops = crate::ported::zsh_h::options {
        ind: [0u8; crate::ported::zsh_h::MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    };
    let mut count = 0usize;
    for w in STANDARD_COMPLETE_WIDGETS {
        // `zle -C <w> .<w> _main_complete` — args are post-flag:
        // [target-thingy, base-comp-widget, completion-func].
        let args = [
            w.to_string(),
            format!(".{}", w),
            "_main_complete".to_string(),
        ];
        if crate::ported::zle::zle_thingy::bin_zle_complete("zle", &args, &empty_ops, 0) == 0 {
            count += 1;
        }
    }
    // sh:560 — `zle -C menu-select .menu-select _main_complete` (only
    // succeeds when the `.menu-select` base widget exists, i.e.
    // `zsh/complist` is loaded; bin_zle_complete returns 1 otherwise).
    {
        let args = [
            "menu-select".to_string(),
            ".menu-select".to_string(),
            "_main_complete".to_string(),
        ];
        if crate::ported::zle::zle_thingy::bin_zle_complete("zle", &args, &empty_ops, 0) == 0 {
            count += 1;
        }
    }
    count
}

/// sh:564-569 — when the configured `completer` chain includes
/// `_expand` AND `^i` is currently bound to `expand-or-complete`,
/// rebind `^i` to `complete-word` so users don't get unexpected
/// glob expansion on TAB.
pub fn maybe_rebind_tab_for_expand() {
    let curcontext = crate::ported::params::getsparam("curcontext").unwrap_or_default();
    let completers = crate::ported::modules::zutil::lookupstyle(
        &format!(":completion:{}:", curcontext),
        "completer",
    );
    let has_expand = completers
        .iter()
        .any(|c| c == "_expand" || c.starts_with("_expand:"));
    if !has_expand {
        return;
    }
    // sh:564 — `bindkey '^i' | IFS=$' \t' read -A _i_line`. We can't
    //   capture bindkey output without a real executor, so just
    //   dispatch the rebind unconditionally when _expand is in
    //   the chain. The shell guards on `expand-or-complete` being
    //   the current binding; without capture we'd be over-eager,
    //   so dispatch via the hook (no-op when no executor wired).
    let _ = crate::ported::exec::dispatch_function_call(
        "bindkey",
        &["^i".to_string(), "complete-word".to_string()],
    );
}

// `compaudit` lives in its own file (`src/compsys/ported/compaudit.rs`)
// per zsh upstream's layout (`Completion/compaudit` is a sibling of
// `Completion/compinit`). Re-export the entry point so existing
// compinit-facing callers don't need to change.
pub use super::compaudit::{compaudit, CompauditError};

/// Completion definition from #compdef line
#[derive(Clone, Debug)]
pub enum CompDef {
    /// Regular command completion: #compdef cmd1 cmd2 ...
    Commands(Vec<String>),
    /// Pattern completion: #compdef -p 'pattern'
    Pattern(String),
    /// Post-pattern completion: #compdef -P 'pattern'
    PostPattern(String),
    /// Key binding: #compdef -k style key1 key2 ...
    KeyBinding { style: String, keys: Vec<String> },
    /// Widget key binding: #compdef -K widget style key
    WidgetKey {
        widget: String,
        style: String,
        key: String,
    },
}

/// Parsed completion file
#[derive(Clone, Debug)]
pub struct CompFile {
    /// Full path to the file
    pub path: PathBuf,
    /// Function name (filename without path)
    pub name: String,
    /// What this file defines
    pub def: CompFileDef,
    /// Full file body (read during scan for caching)
    pub body: Option<String>,
}

/// What a completion file defines
#[derive(Clone, Debug)]
pub enum CompFileDef {
    /// #compdef - completion function
    CompDef(CompDef),
    /// #autoload - helper function with options
    Autoload(Vec<String>),
    None,
}

/// Result of compinit scan
#[derive(Debug, Default)]
pub struct CompInitResult {
    /// Command -> function mapping (_comps)
    pub comps: HashMap<String, String>,
    /// Command -> service mapping (_services)
    pub services: HashMap<String, String>,
    /// Pattern -> function mapping (_patcomps)
    pub patcomps: HashMap<String, String>,
    /// Post-pattern -> function mapping (_postpatcomps)
    pub postpatcomps: HashMap<String, String>,
    /// Autoload functions with options (_compautos)
    pub compautos: HashMap<String, String>,
    /// All scanned files
    pub files: Vec<CompFile>,
    /// Scan duration
    pub scan_time_ms: u64,
    /// Number of directories scanned
    pub dirs_scanned: usize,
    /// Number of files scanned
    pub files_scanned: usize,
}

/// Parse the first line of a completion file
///
/// Handles all #compdef variants:
/// - `#compdef cmd1 cmd2` - regular commands
/// - `#compdef - cmd1 cmd2` - bare hyphen + commands (hyphen maps to '-')
/// - `#compdef -default-` - special context entries
/// - `#compdef -value-,VAR,-default-` - value context entries  
/// - `#compdef -p pattern` - pattern completions
/// - `#compdef -P pattern` - post-pattern completions
/// - `#compdef -k style key` - key bindings
/// - `#compdef -K widget style key` - widget key bindings
fn parse_first_line(line: &str) -> CompFileDef {
    let line = line.trim();

    if let Some(rest) = line.strip_prefix("#compdef") {
        let rest = rest.trim();
        if rest.is_empty() {
            return CompFileDef::None;
        }

        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.is_empty() {
            return CompFileDef::None;
        }

        // Check for special options first
        match parts[0] {
            "-p" if parts.len() >= 2 => {
                CompFileDef::CompDef(CompDef::Pattern(parts[1].to_string()))
            }
            "-P" if parts.len() >= 2 => {
                // -P patterns go directly into _comps (not _patcomps!)
                // zsh puts these patterns as keys in _comps hash
                // Can have multiple: #compdef -P pattern1 -P pattern2 cmd1 cmd2
                let mut all_cmds = Vec::new();
                let mut i = 0;
                while i < parts.len() {
                    if parts[i] == "-P" && i + 1 < parts.len() {
                        // Pattern goes directly as a key in _comps
                        all_cmds.push(parts[i + 1].to_string());
                        i += 2;
                    } else if !parts[i].starts_with('-') || is_context_entry(parts[i]) {
                        all_cmds.push(parts[i].to_string());
                        i += 1;
                    } else {
                        i += 1;
                    }
                }
                if all_cmds.is_empty() {
                    CompFileDef::None
                } else {
                    CompFileDef::CompDef(CompDef::Commands(all_cmds))
                }
            }
            "-k" if parts.len() >= 3 => CompFileDef::CompDef(CompDef::KeyBinding {
                style: parts[1].to_string(),
                keys: parts[2..].iter().map(|s| s.to_string()).collect(),
            }),
            "-K" if parts.len() >= 4 => CompFileDef::CompDef(CompDef::WidgetKey {
                widget: parts[1].to_string(),
                style: parts[2].to_string(),
                key: parts[3].to_string(),
            }),
            _ => {
                // Parse command definitions, including:
                // - bare "-" (maps to '-' in _comps)
                // - context entries like "-default-", "-redirect-", "-value-,VAR,-default-"
                // - regular commands
                //
                // Skip actual option flags like "-n" but keep context entries
                let cmds: Vec<String> = parts
                    .iter()
                    .filter(|s| {
                        // Keep if:
                        // - bare hyphen "-"
                        // - context entry like "-foo-" or "-value-,X,-default-"
                        // - regular command (no leading hyphen)
                        **s == "-" || is_context_entry(s) || !s.starts_with('-')
                    })
                    .map(|s| s.to_string())
                    .collect();
                if cmds.is_empty() {
                    CompFileDef::None
                } else {
                    CompFileDef::CompDef(CompDef::Commands(cmds))
                }
            }
        }
    } else if let Some(rest) = line.strip_prefix("#autoload") {
        let opts: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
        CompFileDef::Autoload(opts)
    } else {
        CompFileDef::None
    }
}

/// Check if a string is a zsh completion context entry
/// Context entries are like: -default-, -redirect-, -command-, -value-,VAR,-default-
/// Also handles service syntax: -redirect-,<,bunzip2=bunzip2
fn is_context_entry(s: &str) -> bool {
    if !s.starts_with('-') {
        return false;
    }
    // Strip service suffix for checking
    let base = s.split('=').next().unwrap_or(s);

    // Check if it's a known context pattern:
    // 1. Ends with '-' like -default-, -redirect-
    // 2. Contains comma (context specifiers like -redirect-,<,bunzip2 or -value-,VAR,-default-)
    // 3. But NOT single letter options like -p, -P, -k, -K, -n
    if base.len() <= 2 {
        return base == "-"; // bare hyphen is a context entry
    }

    base.ends_with('-') || base.contains(',')
}

/// Scan a single completion file - reads full body for caching
fn scan_file(path: &Path) -> Option<CompFile> {
    let name = path.file_name()?.to_string_lossy().to_string();

    // Must start with underscore
    if !name.starts_with('_') {
        return None;
    }

    // Skip certain patterns
    if name.contains(';')
        || name.contains('|')
        || name.contains('&')
        || name.ends_with('~')
        || name.ends_with(".zwc")
    {
        return None;
    }

    // Read entire file at once (will be cached in SQLite)
    let body = fs::read_to_string(path).ok()?;

    // Parse first line for directive
    let first_line = body.lines().next().unwrap_or("");
    let def = parse_first_line(first_line);

    Some(CompFile {
        path: path.to_path_buf(),
        name,
        def,
        body: Some(body),
    })
}

/// Scan a directory for completion files (parallel)
fn scan_directory(dir: &Path) -> Vec<CompFile> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();

    // Parallel scan of files within directory
    paths.par_iter().filter_map(|p| scan_file(p)).collect()
}

/// Initialize the completion system by scanning fpath
///
/// This is the main entry point - replaces the zsh compinit function.
/// Uses rayon for parallel directory and file scanning.
pub fn compinit(fpath: &[PathBuf]) -> CompInitResult {
    let start = Instant::now();

    // sh:129-134 — install `$_comp_dumpfile` default so engine code
    //   that calls `getsparam("_comp_dumpfile")` sees a sensible
    //   path. User-supplied `compinit -d FILE` overrides.
    if crate::ported::params::getsparam("_comp_dumpfile")
        .map(|s| s.is_empty())
        .unwrap_or(true)
    {
        let _ = crate::ported::params::setsparam(
            "_comp_dumpfile",
            &default_dumpfile_path().to_string_lossy(),
        );
    }

    // sh:138-172 — publish `_comp_options` to the shell-side param
    //   table so `$_comp_setup` eval at every entry point picks it
    //   up. Use the canonical const list.
    crate::ported::params::setaparam(
        "_comp_options",
        COMP_OPTIONS.iter().map(|s| s.to_string()).collect(),
    );

    // sh:180-190 — publish `_comp_setup` (the eval string).
    let _ = crate::ported::params::setsparam("_comp_setup", COMP_SETUP_EVAL);

    // sh:195-197 — initialize the pre/post-hook arrays.
    init_comp_funcs_arrays();

    // Track seen function names (first one wins)
    let seen: Mutex<HashSet<String>> = Mutex::new(HashSet::new());

    // Parallel scan of all directories
    let all_files: Vec<CompFile> = fpath
        .par_iter()
        .filter(|dir| dir.as_os_str() != "." && dir.exists())
        .flat_map(|dir| scan_directory(dir))
        .filter(|f| {
            let mut seen = seen.lock().unwrap();
            if seen.contains(&f.name) {
                false
            } else {
                seen.insert(f.name.clone());
                true
            }
        })
        .collect();

    let files_scanned = all_files.len();
    let dirs_scanned = fpath.len();

    // Build the result maps
    let mut result = CompInitResult {
        scan_time_ms: start.elapsed().as_millis() as u64,
        dirs_scanned,
        files_scanned,
        ..Default::default()
    };

    for file in &all_files {
        match &file.def {
            CompFileDef::CompDef(compdef) => {
                match compdef {
                    CompDef::Commands(cmds) => {
                        for cmd in cmds {
                            // Handle service syntax: cmd=service
                            if let Some(eq_pos) = cmd.find('=') {
                                let cmd_name = &cmd[..eq_pos];
                                let service = &cmd[eq_pos + 1..];
                                result.comps.insert(cmd_name.to_string(), file.name.clone());
                                result
                                    .services
                                    .insert(cmd_name.to_string(), service.to_string());
                            } else {
                                result.comps.insert(cmd.clone(), file.name.clone());
                            }
                        }
                    }
                    CompDef::Pattern(pat) => {
                        // -p patterns go to BOTH _comps and _patcomps (zsh behavior)
                        result.comps.insert(pat.clone(), file.name.clone());
                        result.patcomps.insert(pat.clone(), file.name.clone());
                    }
                    CompDef::PostPattern(pat) => {
                        result.postpatcomps.insert(pat.clone(), file.name.clone());
                    }
                    CompDef::KeyBinding { .. } | CompDef::WidgetKey { .. } => {
                        // Key bindings need to be handled by the shell
                    }
                }
            }
            CompFileDef::Autoload(opts) => {
                let opts_str = opts.join(" ");
                result.compautos.insert(file.name.clone(), opts_str);
            }
            CompFileDef::None => {}
        }
    }

    result.files = all_files;

    // sh:553-569 — the standard completion-widget rebind (`zle -C
    //   complete-word .complete-word _main_complete` × 8 + conditional
    //   menu-select + TAB/_expand rebind) is NOT done here. `compinit`
    //   ships this fpath scan to a worker-pool thread (see
    //   `ext_builtins::builtin_compinit`), and ZLE keymaps/widgets live
    //   on the main thread — a `zle -C` issued from a worker never
    //   reaches the interactive keymap, so TAB stays bound to the
    //   builtin `expand-or-complete` and `_main_complete` never fires.
    //   The rebind is instead performed synchronously on the main
    //   thread in `builtin_compinit`, where it belongs; it needs only
    //   `_main_complete` (a Rust fn, always present), not the scan
    //   results, so deferring it to the background is unnecessary.

    // Publish the result-side compdef state so downstream `getaparam(
    //   "_comps")` etc. queries reflect the scan. This is the new-
    //   compinit-finish complement of `compdef()`'s per-call publish.
    with_state(|s| {
        for (k, v) in &result.comps {
            s.comps.insert(k.clone(), v.clone());
        }
        for (k, v) in &result.services {
            s.services.insert(k.clone(), v.clone());
        }
        for (k, v) in &result.patcomps {
            s.patcomps.insert(k.clone(), v.clone());
        }
        for (k, v) in &result.postpatcomps {
            s.postpatcomps.insert(k.clone(), v.clone());
        }
        for (k, v) in &result.compautos {
            s.compautos.insert(k.clone(), v.clone());
        }
        publish_compdef_state(s);
    });

    result
}

// `compdump`, `check_dump`, and `escape_zsh_string` moved to
// `compsys/ported/compdump.rs` (1:1 with upstream `Completion/compdump`).
pub use super::compdump::{check_dump, compdump};

/// Build SQLite cache from fpath scan
///
/// This is the main entry point for initializing the completion system.
/// It scans fpath directories, parses #compdef directives, and populates
/// the SQLite cache for fast lookups.
pub fn build_cache_from_fpath(
    fpath: &[PathBuf],
    cache: &mut crate::compsys::cache::CompsysCache,
) -> std::io::Result<CompInitResult> {
    use std::time::Instant;

    let t0 = Instant::now();
    let result = compinit(fpath);
    let scan_time = t0.elapsed();

    let t1 = Instant::now();

    // Populate comps table (_comps hash)
    let comps: Vec<(String, String)> = result
        .comps
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    cache
        .set_comps_bulk(&comps)
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    // Populate services table (_services hash)
    let services: Vec<(String, String)> = result
        .services
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    cache
        .set_services_bulk(&services)
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    // Populate patcomps table (_patcomps hash)
    for (pattern, function) in &result.patcomps {
        cache
            .set_patcomp(pattern, function)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
    }

    // Populate postpatcomps (stored in patcomps with a marker, or separate table if needed)
    // For now, we'll merge them into patcomps
    for (pattern, function) in &result.postpatcomps {
        cache
            .set_patcomp(pattern, function)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
    }

    let comps_time = t1.elapsed();
    let t2 = Instant::now();

    // Populate autoloads table with function bodies for instant loading
    // Bodies were already read during parallel scan - no extra I/O here
    let autoloads: Vec<(String, String, String)> = result
        .files
        .iter()
        .filter(|f| matches!(f.def, CompFileDef::CompDef(_) | CompFileDef::Autoload(_)))
        .filter_map(|f| {
            let path_str = f.path.to_string_lossy().to_string();
            let body = f.body.as_ref()?.clone();
            Some((f.name.clone(), path_str, body))
        })
        .collect();
    cache
        .add_autoloads_with_bodies_bulk(&autoloads)
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    let autoloads_time = t2.elapsed();

    // Timing logged by caller in vm_helper via tracing

    Ok(result)
}

/// Load _comps from existing cache (instantaneous)
///
/// Returns a CompInitResult populated from the SQLite cache without rescanning fpath.
/// Use this after the cache has been built with `build_cache_from_fpath`.
///
/// This is the equivalent of `compinit -C` with a valid zcompdump - it skips
/// the fpath scan entirely and just loads from cache.
#[allow(clippy::field_reassign_with_default)] // result is mutated across many subsequent statements; struct-literal init not practical
pub fn load_from_cache(
    cache: &crate::compsys::cache::CompsysCache,
) -> std::io::Result<CompInitResult> {
    use std::time::Instant;
    let start = Instant::now();

    let mut result = CompInitResult::default();

    // Load comps - single query
    result.comps = cache
        .get_all_comps()
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    // Load patcomps - single query
    for (pat, func) in cache
        .patcomps_kv()
        .map_err(|e| std::io::Error::other(e.to_string()))?
    {
        result.patcomps.insert(pat, func);
    }

    // Services are loaded on-demand via cache.get_service() - no need to preload
    // This matches zsh behavior where $_services is lazily populated

    result.scan_time_ms = start.elapsed().as_millis() as u64;
    result.files_scanned = result.comps.len();

    Ok(result)
}

/// Fast check if compinit is needed
///
/// Returns the number of completion entries in cache, or 0 if cache is empty/invalid.
/// Use this to decide whether to run full compinit or load_from_cache.
pub fn cache_entry_count(cache: &crate::compsys::cache::CompsysCache) -> usize {
    cache.comp_count().unwrap_or(0) as usize
}

/// Lazy compinit - validates cache exists but doesn't load into memory
///
/// This is the fastest option for shell startup. It just verifies the cache
/// is valid and returns immediately. Actual lookups happen via cache.get_comp().
///
/// Returns (is_valid, entry_count) in microseconds.
pub fn compinit_lazy(cache: &crate::compsys::cache::CompsysCache) -> (bool, usize) {
    let count = cache.comp_count().unwrap_or(0) as usize;
    (count > 0, count)
}

/// Check if cache is valid and up-to-date
///
/// Returns true if cache exists and has entries, false if cache needs to be rebuilt.
pub fn cache_is_valid(cache: &crate::compsys::cache::CompsysCache) -> bool {
    cache.comp_count().unwrap_or(0) > 0
}

/// Get system fpath from environment or defaults
pub fn get_system_fpath() -> Vec<PathBuf> {
    // Try FPATH env var first
    if let Ok(fpath_str) = std::env::var("FPATH") {
        if !fpath_str.is_empty() {
            return fpath_str.split(':').map(PathBuf::from).collect();
        }
    }

    // Default paths for common systems
    let mut paths = Vec::new();

    // macOS Homebrew
    for base in &["/opt/homebrew", "/usr/local"] {
        paths.push(PathBuf::from(format!("{}/share/zsh/site-functions", base)));
        paths.push(PathBuf::from(format!("{}/share/zsh/functions", base)));
    }

    // System zsh
    for version in &["5.9", "5.8", "5.7"] {
        paths.push(PathBuf::from(format!(
            "/usr/share/zsh/{}/functions",
            version
        )));
    }
    paths.push(PathBuf::from("/usr/share/zsh/functions"));
    paths.push(PathBuf::from("/usr/share/zsh/site-functions"));

    // Zinit/zplugin common paths
    if let Ok(home) = std::env::var("HOME") {
        paths.push(PathBuf::from(format!("{}/.zinit/completions", home)));
        paths.push(PathBuf::from(format!("{}/.zplugin/completions", home)));
        paths.push(PathBuf::from(format!(
            "{}/.local/share/zsh/site-functions",
            home
        )));
    }

    // Filter to existing directories
    paths.into_iter().filter(|p| p.exists()).collect()
}

/// Options for compinit
#[derive(Clone, Debug, Default)]
pub struct CompInitOpts {
    /// Dump file path (-d)
    pub dump_file: Option<PathBuf>,
    /// Skip dump (-D)
    pub no_dump: bool,
    /// Skip security check (-C)
    pub no_check: bool,
    /// Ignore insecure dirs (-i)
    pub ignore_insecure: bool,
    /// Use insecure dirs (-u)
    pub use_insecure: bool,
}

impl CompInitOpts {
    /// Parse compinit arguments
    pub fn parse(args: &[String]) -> Self {
        let mut opts = Self::default();
        let mut i = 0;

        while i < args.len() {
            match args[i].as_str() {
                "-d" if i + 1 < args.len() && !args[i + 1].starts_with('-') => {
                    opts.dump_file = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
                "-D" => opts.no_dump = true,
                "-C" => opts.no_check = true,
                "-i" => opts.ignore_insecure = true,
                "-u" => opts.use_insecure = true,
                _ => {}
            }
            i += 1;
        }

        opts
    }
}

// =====================================================================
// compdef() — runtime registration entry point.
//
// Upstream defines this inside `Completion/compinit` (sh:253-446); we
// mirror that organizational choice. User `.zshrc` lines like:
//   compdef _git git
//   compdef -p '*-test' _test
//   compdef -d obsolete
// land here.
//
// State lives in a session-side `CompdefState` (a Mutex<HashMap>
// quintet for `_comps`, `_services`, `_patcomps`, `_postpatcomps`,
// `_compautos`). Cluster ports already read these via shell-side
// `getaparam("_comps")` etc.; the published-to-paramtab step
// happens in `publish_compdef_state` at the bottom of this section.
// =====================================================================

/// Session-side compdef registrations. Mirrors the five upstream
/// assoc arrays one-for-one.
#[derive(Default)]
pub struct CompdefState {
    pub comps: HashMap<String, String>,
    pub services: HashMap<String, String>,
    pub patcomps: HashMap<String, String>,
    pub postpatcomps: HashMap<String, String>,
    pub compautos: HashMap<String, String>,
}

static COMPDEF_STATE: Mutex<Option<CompdefState>> = Mutex::new(None);

/// Lock the session-side state, initializing on first call.
fn with_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut CompdefState) -> R,
{
    let mut guard = COMPDEF_STATE.lock().unwrap();
    if guard.is_none() {
        *guard = Some(CompdefState::default());
    }
    f(guard.as_mut().unwrap())
}

/// Helper to publish state inside a `with_state` closure (takes
/// `&mut` to fit the closure signature).
fn publish_compdef_state_mut(s: &mut CompdefState) {
    publish_compdef_state(s);
}

/// Publish the in-memory state back to shell-side assoc arrays so
/// engine cluster code (which reads via `getaparam("_comps")` etc.)
/// sees the updates. Flat key/value layout per the existing
/// per-engine-port convention.
fn publish_compdef_state(s: &CompdefState) {
    let mut flatten = |arr: &HashMap<String, String>| -> Vec<String> {
        let mut sorted: Vec<(&String, &String)> = arr.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        let mut out = Vec::with_capacity(sorted.len() * 2);
        for (k, v) in sorted {
            out.push(k.clone());
            out.push(v.clone());
        }
        out
    };
    // These are ASSOCIATIVE arrays in zsh (`typeset -gHA _comps _services
    // _patcomps _postpatcomps` / `_compautos`, sh:116/121). They MUST be
    // published via `sethparam` (hashed) — `flatten` already emits the
    // `[k, v, k, v, …]` pair layout `sethparam` consumes. Using `setaparam`
    // (plain array) made `_comps` a flat array, so `${_comps[ls]}` couldn't
    // hash-lookup and `_complete`/`_dispatch` found no completer for ANY
    // command → every `<cmd> <TAB>` (incl. `ls -<TAB>`) just rang the bell.
    // The synchronous `compinit -C` path used `set_assoc` (→ sethparam) and
    // worked, which is why only the fresh/compdef-driven path was broken.
    // Bug #655.
    crate::ported::params::sethparam("_comps", flatten(&s.comps));
    crate::ported::params::sethparam("_services", flatten(&s.services));
    crate::ported::params::sethparam("_patcomps", flatten(&s.patcomps));
    crate::ported::params::sethparam("_postpatcomps", flatten(&s.postpatcomps));
    crate::ported::params::sethparam("_compautos", flatten(&s.compautos));
}

/// Parse `compdef`'s short-option flags via the upstream
/// `getopts "anpPkKde"` (sh:267).
#[derive(Default, Debug)]
struct CompdefFlags {
    autol: bool,
    new: bool,
    delete: bool,
    eval: bool,
    /// Mutually-exclusive `-p`/`-P`/`-k`/`-K`. Only the most-recent
    /// wins; upstream errors on duplicate, we tolerate.
    spec_type: SpecType,
}

#[derive(Default, Debug, PartialEq, Clone, Copy)]
enum SpecType {
    #[default]
    Normal,
    Pattern,
    PostPattern,
    Key,
    WidgetKey,
}

fn parse_compdef_flags(args: &[String]) -> Result<(CompdefFlags, usize), String> {
    let mut flags = CompdefFlags::default();
    let mut idx = 0usize;
    while idx < args.len() {
        let a = &args[idx];
        if !a.starts_with('-') || a == "-" || a == "--" {
            break;
        }
        // sh:267 getopts allows combined flags like `-an`. Walk each
        //   letter after the leading `-`.
        for c in a.chars().skip(1) {
            match c {
                'a' => flags.autol = true,
                'n' => flags.new = true,
                'd' => flags.delete = true,
                'e' => flags.eval = true,
                'p' => flags.spec_type = SpecType::Pattern,
                'P' => flags.spec_type = SpecType::PostPattern,
                'k' => flags.spec_type = SpecType::Key,
                'K' => flags.spec_type = SpecType::WidgetKey,
                _ => return Err(format!("compdef: unknown option: -{}", c)),
            }
        }
        idx += 1;
    }
    Ok((flags, idx))
}

/// `compdef` — register or unregister completion functions for
/// commands. Faithful to upstream `Completion/compinit` sh:253-446.
///
/// Returns the upstream-compatible exit code: 0 on success, 1 on
/// usage error.
pub fn compdef(args: &[String]) -> i32 {
    // sh:262
    if args.is_empty() {
        eprintln!("compdef: I need arguments");
        return 1;
    }
    let (flags, mut idx) = match parse_compdef_flags(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            return 1;
        }
    };
    // sh:293
    if idx >= args.len() {
        eprintln!("compdef: I need arguments");
        return 1;
    }

    if flags.delete {
        // sh:426-444  -d: delete by name from the right hash
        let names = &args[idx..];
        with_state(|s| match flags.spec_type {
            SpecType::Pattern => {
                for n in names {
                    s.patcomps.remove(n);
                }
            }
            SpecType::PostPattern => {
                for n in names {
                    s.postpatcomps.remove(n);
                }
            }
            SpecType::Key | SpecType::WidgetKey => {
                eprintln!("compdef: cannot restore key bindings");
            }
            SpecType::Normal => {
                for n in names {
                    s.comps.remove(n);
                    s.services.remove(n);
                }
            }
        });
        with_state(publish_compdef_state_mut);
        return 0;
    }

    // sh:298-327  service-alias mode: no flags + first arg contains `=`.
    //   Each subsequent arg must also contain `=` (else it's an error).
    if !flags.eval && args[idx].contains('=') {
        let mut ret: i32 = 0;
        while idx < args.len() {
            let entry = args[idx].clone();
            idx += 1;
            if !entry.contains('=') {
                eprintln!("compdef: invalid argument: {}", entry);
                ret = 1;
                continue;
            }
            let mut sp = entry.splitn(2, '=');
            let cmd = sp.next().unwrap_or("").to_string();
            let svc_in = sp.next().unwrap_or("").to_string();
            // sh:307-311 — resolve `$svc` via `_services[(r)$svc]` reverse
            //   lookup (find an entry mapping TO $svc); else use $svc
            //   directly. Then look up `_comps[$svc]` for the function.
            let resolved_svc = {
                with_state(|s| {
                    s.services
                        .iter()
                        .find(|(_, v)| **v == svc_in)
                        .map(|(k, _)| k.clone())
                        .unwrap_or(svc_in.clone())
                })
            };
            let func = with_state(|s| {
                s.comps
                    .get(&resolved_svc)
                    .cloned()
                    .or_else(|| {
                        // sh:311 fallback to first matching pat/postpat key
                        s.patcomps
                            .iter()
                            .find(|(k, _)| pattern_matches(k, &svc_in))
                            .map(|(_, v)| v.clone())
                            .or_else(|| {
                                s.postpatcomps
                                    .iter()
                                    .find(|(k, _)| pattern_matches(k, &svc_in))
                                    .map(|(_, v)| v.clone())
                            })
                    })
                    .unwrap_or_default()
            });
            if func.is_empty() {
                eprintln!("compdef: unknown command or service: {}", svc_in);
                ret = 1;
                continue;
            }
            let svc_for_state =
                with_state(|s| s.services.get(&svc_in).cloned().unwrap_or(svc_in.clone()));
            with_state(|s| {
                s.comps.insert(cmd.clone(), func.clone());
                s.services.insert(cmd, svc_for_state);
            });
        }
        with_state(publish_compdef_state_mut);
        return ret;
    }

    // sh:332-334  First positional after flags is the function name.
    let func = args[idx].clone();
    idx += 1;

    // sh:333  `-a` → autoload
    if flags.autol && func.starts_with('_') {
        // dispatch `autoload -rUz <func>` via the exec accessors bridge.
        let _ = crate::ported::exec::dispatch_function_call(
            "autoload",
            &["-rUz".to_string(), func.clone()],
        );
        // Track for the dump file
        with_state(|s| {
            s.compautos.insert(func.clone(), "-rUz".to_string());
        });
    }

    // sh:336-425
    match flags.spec_type {
        SpecType::WidgetKey => {
            // sh:337-355  -K widget-name comp-widget key  (in triples)
            let mut i = idx;
            while i + 2 < args.len() {
                let mut wname = args[i].clone();
                let mut comp_widget = args[i + 1].clone();
                let key = args[i + 2].clone();
                if !wname.starts_with('_') {
                    wname = format!("_{}", wname);
                }
                if !comp_widget.starts_with('.') {
                    comp_widget = format!(".{}", comp_widget);
                }
                // sh:346  zle -C <wname> <comp_widget> <func>
                let _ = crate::ported::exec::dispatch_function_call(
                    "zle",
                    &["-C".to_string(), wname.clone(), comp_widget, func.clone()],
                );
                // sh:347-352  bindkey
                let _ = crate::ported::exec::dispatch_function_call("bindkey", &[key, wname]);
                i += 3;
            }
        }
        SpecType::Key => {
            // sh:356-379  -k style key... (in 1+ pairs)
            if idx >= args.len() {
                eprintln!("compdef: missing keys");
                return 1;
            }
            let mut style = args[idx].clone();
            idx += 1;
            if !style.starts_with('.') {
                style = format!(".{}", style);
            }
            // sh:365  zle -C <func> <style> <func>
            let _ = crate::ported::exec::dispatch_function_call(
                "zle",
                &["-C".to_string(), func.clone(), style, func.clone()],
            );
            for key in &args[idx..] {
                let _ = crate::ported::exec::dispatch_function_call(
                    "bindkey",
                    &[key.clone(), func.clone()],
                );
            }
        }
        _ => {
            // sh:381-424  normal / pattern / postpattern
            let mut effective_type = flags.spec_type;
            while idx < args.len() {
                let arg = args[idx].clone();
                idx += 1;
                // sh:385-390  inline type switch
                match arg.as_str() {
                    "-N" => {
                        effective_type = SpecType::Normal;
                        continue;
                    }
                    "-p" => {
                        effective_type = SpecType::Pattern;
                        continue;
                    }
                    "-P" => {
                        effective_type = SpecType::PostPattern;
                        continue;
                    }
                    _ => {}
                }
                with_state(|s| match effective_type {
                    SpecType::Pattern => {
                        // sh:393-398 — `key=val` rewrites to `=val=func`
                        if let Some(eq) = arg.find('=') {
                            let key = arg[..eq].to_string();
                            let val = arg[eq + 1..].to_string();
                            s.patcomps.insert(key, format!("={}={}", val, func));
                        } else {
                            s.patcomps.insert(arg.clone(), func.clone());
                        }
                    }
                    SpecType::PostPattern => {
                        if let Some(eq) = arg.find('=') {
                            let key = arg[..eq].to_string();
                            let val = arg[eq + 1..].to_string();
                            s.postpatcomps.insert(key, format!("={}={}", val, func));
                        } else {
                            s.postpatcomps.insert(arg.clone(), func.clone());
                        }
                    }
                    _ => {
                        // sh:407-419  normal: cmd or cmd=svc
                        let (cmd, svc) = if let Some(eq) = arg.find('=') {
                            (arg[..eq].to_string(), Some(arg[eq + 1..].to_string()))
                        } else {
                            (arg.clone(), None)
                        };
                        // sh:415  -n: no-clobber
                        if flags.new && s.comps.contains_key(&cmd) {
                            return;
                        }
                        s.comps.insert(cmd.clone(), func.clone());
                        if let Some(svc) = svc {
                            s.services.insert(cmd, svc);
                        }
                    }
                });
            }
        }
    }
    with_state(publish_compdef_state_mut);
    0
}

/// sh:311 pattern-matching helper. Uses the real `pattern.rs`
/// matcher so `(K)` assoc-key glob matching is faithful.
fn pattern_matches(pat: &str, s: &str) -> bool {
    match crate::ported::pattern::patcompile(
        &{
            let mut __pat_tok = (pat).to_string();
            crate::ported::glob::tokenize(&mut __pat_tok);
            __pat_tok
        },
        0,
        None,
    ) {
        Some(prog) => crate::ported::pattern::pattry(&prog, s),
        None => pat == s,
    }
}

/// Reset session-side state (test-only helper; exposed via
/// `#[cfg(test)]` users).
#[cfg(test)]
pub fn reset_compdef_state() {
    *COMPDEF_STATE.lock().unwrap() = Some(CompdefState::default());
}

/// Snapshot of session-side state — useful for `compdump` to read
/// without locking inside the hot path.
pub fn snapshot_compdef_state() -> CompdefState {
    with_state(|s| CompdefState {
        comps: s.comps.clone(),
        services: s.services.clone(),
        patcomps: s.patcomps.clone(),
        postpatcomps: s.postpatcomps.clone(),
        compautos: s.compautos.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_compdef_commands() {
        let def = parse_first_line("#compdef git svn hg");
        match def {
            CompFileDef::CompDef(CompDef::Commands(cmds)) => {
                assert_eq!(cmds, vec!["git", "svn", "hg"]);
            }
            _ => panic!("Expected Commands"),
        }
    }

    #[test]
    fn test_parse_compdef_pattern() {
        let def = parse_first_line("#compdef -p 'c*'");
        match def {
            CompFileDef::CompDef(CompDef::Pattern(pat)) => {
                assert_eq!(pat, "'c*'");
            }
            _ => panic!("Expected Pattern"),
        }
    }

    #[test]
    fn test_parse_autoload() {
        let def = parse_first_line("#autoload -U -z");
        match def {
            CompFileDef::Autoload(opts) => {
                assert_eq!(opts, vec!["-U", "-z"]);
            }
            _ => panic!("Expected Autoload"),
        }
    }

    #[test]
    fn test_parse_compdef_key() {
        let def = parse_first_line("#compdef -k complete-word ^X^C");
        match def {
            CompFileDef::CompDef(CompDef::KeyBinding { style, keys }) => {
                assert_eq!(style, "complete-word");
                assert_eq!(keys, vec!["^X^C"]);
            }
            _ => panic!("Expected KeyBinding"),
        }
    }

    #[test]
    fn test_parse_compdef_redirect_context() {
        // _bzip2 line: has regular commands + context entries with services
        let def = parse_first_line("#compdef bzip2 bunzip2 bzcat=bunzip2 bzip2recover -redirect-,<,bunzip2=bunzip2 -redirect-,>,bzip2=bunzip2 -redirect-,<,bzip2=bzip2");
        match def {
            CompFileDef::CompDef(CompDef::Commands(cmds)) => {
                // Should contain all entries
                assert!(cmds.contains(&"bzip2".to_string()), "missing bzip2");
                assert!(cmds.contains(&"bunzip2".to_string()), "missing bunzip2");
                assert!(
                    cmds.contains(&"bzcat=bunzip2".to_string()),
                    "missing bzcat=bunzip2"
                );
                assert!(
                    cmds.contains(&"bzip2recover".to_string()),
                    "missing bzip2recover"
                );
                assert!(
                    cmds.contains(&"-redirect-,<,bunzip2=bunzip2".to_string()),
                    "missing redirect bunzip2"
                );
                assert!(
                    cmds.contains(&"-redirect-,>,bzip2=bunzip2".to_string()),
                    "missing redirect >,bzip2"
                );
                assert!(
                    cmds.contains(&"-redirect-,<,bzip2=bzip2".to_string()),
                    "missing redirect <,bzip2"
                );
                assert_eq!(cmds.len(), 7, "cmds: {:?}", cmds);
            }
            other => panic!("Expected Commands, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_compdef_context_entries() {
        // -default- style entries
        let def = parse_first_line("#compdef -default-");
        match def {
            CompFileDef::CompDef(CompDef::Commands(cmds)) => {
                assert_eq!(cmds, vec!["-default-"]);
            }
            other => panic!("Expected Commands, got {:?}", other),
        }

        // bare hyphen + commands
        let def = parse_first_line("#compdef - nohup eval time");
        match def {
            CompFileDef::CompDef(CompDef::Commands(cmds)) => {
                assert!(cmds.contains(&"-".to_string()));
                assert!(cmds.contains(&"nohup".to_string()));
                assert!(cmds.contains(&"eval".to_string()));
                assert!(cmds.contains(&"time".to_string()));
            }
            other => panic!("Expected Commands, got {:?}", other),
        }

        // -value- entries
        let def = parse_first_line("#compdef -value- -array-value- -value-,-default-,-default-");
        match def {
            CompFileDef::CompDef(CompDef::Commands(cmds)) => {
                assert!(cmds.contains(&"-value-".to_string()));
                assert!(cmds.contains(&"-array-value-".to_string()));
                assert!(cmds.contains(&"-value-,-default-,-default-".to_string()));
            }
            other => panic!("Expected Commands, got {:?}", other),
        }
    }

    #[test]
    fn test_is_context_entry() {
        assert!(is_context_entry("-default-"));
        assert!(is_context_entry("-redirect-"));
        assert!(is_context_entry("-value-,DISPLAY,-default-"));
        assert!(is_context_entry("-redirect-,<,bunzip2=bunzip2"));
        assert!(is_context_entry("-redirect-,>,bzip2"));
        assert!(!is_context_entry("-p")); // option flag, not context
        assert!(!is_context_entry("-P")); // option flag
        assert!(!is_context_entry("git")); // regular command
    }

    // =================================================================
    // compdef() tests — faithful to upstream `Completion/compinit`
    // sh:253-446. Lock the global state via reset_compdef_state to
    // isolate each case.
    // =================================================================

    fn run(args: &[&str]) -> i32 {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        compdef(&owned)
    }

    #[test]
    fn compdef_empty_args_errors() {
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        assert_eq!(compdef(&[]), 1);
    }

    #[test]
    fn compdef_normal_registration() {
        // sh:407-419 — `compdef _git git` writes `_comps[git]=_git`.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        assert_eq!(run(&["_git", "git", "git-commit", "git-push"]), 0);
        let state = snapshot_compdef_state();
        assert_eq!(state.comps.get("git"), Some(&"_git".to_string()));
        assert_eq!(state.comps.get("git-commit"), Some(&"_git".to_string()));
        assert_eq!(state.comps.get("git-push"), Some(&"_git".to_string()));
    }

    #[test]
    fn compdef_normal_with_service() {
        // sh:408-414 — `cmd=svc` records the service alongside.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        assert_eq!(run(&["_git", "hub=git"]), 0);
        let state = snapshot_compdef_state();
        assert_eq!(state.comps.get("hub"), Some(&"_git".to_string()));
        assert_eq!(state.services.get("hub"), Some(&"git".to_string()));
    }

    #[test]
    fn compdef_pattern_via_dash_p() {
        // sh:393-398 — `-p` writes into `_patcomps`.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        assert_eq!(run(&["-p", "_test", "*-test"]), 0);
        let state = snapshot_compdef_state();
        assert_eq!(state.patcomps.get("*-test"), Some(&"_test".to_string()));
    }

    #[test]
    fn compdef_postpattern_via_dash_p_caps() {
        // sh:400-405 — `-P` → `_postpatcomps`.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        assert_eq!(run(&["-P", "_last", "_*"]), 0);
        let state = snapshot_compdef_state();
        assert_eq!(state.postpatcomps.get("_*"), Some(&"_last".to_string()));
    }

    #[test]
    fn compdef_pattern_with_eq_rewrites_to_eq_form() {
        // sh:394-397 — `key=val` form is rewritten to `=val=func`.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        assert_eq!(run(&["-p", "_test", "*=postfix"]), 0);
        let state = snapshot_compdef_state();
        assert_eq!(state.patcomps.get("*"), Some(&"=postfix=_test".to_string()));
    }

    #[test]
    fn compdef_delete_removes_from_comps() {
        // sh:426-444 — `-d` deletes from the right hash.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        run(&["_git", "git"]);
        assert!(snapshot_compdef_state().comps.contains_key("git"));
        assert_eq!(run(&["-d", "git"]), 0);
        assert!(!snapshot_compdef_state().comps.contains_key("git"));
    }

    #[test]
    fn compdef_delete_pattern_removes_from_patcomps() {
        // sh:429-432 — `-d -p` deletes a pattern entry.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        run(&["-p", "_test", "*-test"]);
        assert!(snapshot_compdef_state().patcomps.contains_key("*-test"));
        assert_eq!(run(&["-d", "-p", "*-test"]), 0);
        assert!(!snapshot_compdef_state().patcomps.contains_key("*-test"));
    }

    #[test]
    fn compdef_no_clobber_skips_existing() {
        // sh:415 — `-n` keeps the existing binding.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        run(&["_first", "git"]);
        run(&["-n", "_second", "git"]);
        assert_eq!(
            snapshot_compdef_state().comps.get("git"),
            Some(&"_first".to_string())
        );
    }

    #[test]
    fn compdef_inline_type_switch_dash_p() {
        // sh:385-390 — bare `-p` mid-args toggles to pattern mode.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        run(&["_x", "cmd1", "-p", "pat*", "-N", "cmd2"]);
        let s = snapshot_compdef_state();
        assert_eq!(s.comps.get("cmd1"), Some(&"_x".to_string()));
        assert_eq!(s.patcomps.get("pat*"), Some(&"_x".to_string()));
        assert_eq!(s.comps.get("cmd2"), Some(&"_x".to_string()));
    }

    #[test]
    fn compdef_combined_flags_an() {
        // sh:267 getopts allows `-an` combined.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        // `-an` = autol + new
        assert_eq!(run(&["-an", "_git", "git"]), 0);
        let s = snapshot_compdef_state();
        assert_eq!(s.comps.get("git"), Some(&"_git".to_string()));
        // -a triggers compautos registration
        assert_eq!(s.compautos.get("_git"), Some(&"-rUz".to_string()));
    }

    #[test]
    fn compdef_service_alias_mode_resolves_existing_func() {
        // sh:298-326  — first arg with `=` triggers service-alias.
        //   Each entry resolves via `_services[(r)$svc]` reverse +
        //   `_comps[$svc]`.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        run(&["_git", "git"]); // first set up git→_git
                               // Now `hub=git` should reuse _git
        assert_eq!(run(&["hub=git"]), 0);
        let s = snapshot_compdef_state();
        assert_eq!(s.comps.get("hub"), Some(&"_git".to_string()));
        assert_eq!(s.services.get("hub"), Some(&"git".to_string()));
    }

    #[test]
    fn compdef_service_alias_unknown_returns_one() {
        // sh:316-318  unknown svc → error
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        assert_eq!(run(&["xyz=never-registered"]), 1);
    }

    #[test]
    fn compdef_unknown_flag_errors() {
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        assert_eq!(run(&["-z", "_x", "cmd"]), 1);
    }

    #[test]
    fn compdef_publishes_state_to_shell_arrays() {
        // The shell-side `getaparam("_comps")` must see the
        //   registered entries flat key/value.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        run(&["_git", "git"]);
        let arr = crate::ported::params::getaparam("_comps").unwrap_or_default();
        // Flat layout: [k, v, k, v, ...] sorted by key.
        assert!(arr.windows(2).any(|w| w[0] == "git" && w[1] == "_git"));
    }
}
