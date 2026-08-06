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

/// `typeset -g[H][A|a] NAME` — declare a global parameter with the
/// given type/attribute bits WITHOUT disturbing an existing value.
///
/// Port of the `bin_typeset` path upstream's declarations take
/// (`Src/builtin.c:2469-2575`): when the name is absent the parameter
/// is created with the requested type; when it is already present only
/// the attribute bits are OR'd in. `compinit`'s `typeset -gHA _comps`
/// on a re-`compinit` must not empty the table it just loaded, which is
/// exactly the "already present" arm.
fn declare_global(name: &str, kind: u32, attrs: u32) {
    use crate::ported::params::{paramtab, setaparam, sethparam, setsparam};
    use crate::ported::zsh_h::{PM_ARRAY, PM_HASHED};

    let exists = paramtab()
        .read()
        .ok()
        .map(|t| t.contains_key(name))
        .unwrap_or(false);
    if !exists {
        // Create through the canonical typed setters, NOT a bare
        // `createparam`: for a hashed parameter the values live in
        // `paramtab_hashed_storage`, and only `sethparam` allocates
        // that side. A raw `createparam(PM_HASHED)` produced a
        // `_comps` that existed but could never be filled — `${#_comps}`
        // stayed 0 and `_dispatch` found no completer for ANY command,
        // so every `<cmd> <TAB>` silently did nothing.
        if kind & PM_HASHED != 0 {
            sethparam(name, Vec::new());
        } else if kind & PM_ARRAY != 0 {
            setaparam(name, Vec::new());
        } else {
            let _ = setsparam(name, "");
        }
    }
    // c:Src/builtin.c:2575 — attribute-only update on an existing
    // parameter; the value is left alone, which is what a re-`compinit`
    // needs (`typeset -gHA _comps` must not empty a loaded table).
    if let Ok(mut tab) = paramtab().write() {
        if let Some(pm) = tab.get_mut(name) {
            pm.node.flags |= attrs as i32;
        }
    }
}

/// compinit sh:116-197 — the global parameters `compinit` itself
/// declares, before any of its branches run.
///
/// ```text
/// sh:116  typeset -gHA _comps _services _patcomps _postpatcomps
/// sh:121  typeset -gHA _compautos
/// sh:126  typeset -gHA _lastcomp
/// sh:131  typeset -g _comp_dumpfile="$_i_dumpfile"      (-d FILE)
/// sh:133  typeset -g _comp_dumpfile="${ZDOTDIR:-$HOME}/.zcompdump"
/// sh:138  typeset -gHa _comp_options
/// sh:180  typeset -gH _comp_setup='…'
/// sh:195  typeset -ga compprefuncs comppostfuncs
/// ```
///
/// These are unconditional lines in `compinit`'s body — they run on
/// every path, dump-hit and fresh-scan alike. zshrs keeps the completer
/// tables in Rust and only published `_comps`/`_services`/`_patcomps`
/// on the `-C` cache-hit path, so a real session was missing eight
/// names that upstream always has, and the four assocs it did publish
/// carried no `-H` (`PM_HIDEVAL`) bit. That is directly observable:
/// `unset <TAB>` runs `_vars` → `_parameters`, which offers every
/// parameter whose `${(t)}` lacks `local`, so the missing declarations
/// were missing completions.
pub fn declare_compinit_globals(dumpfile: Option<&str>) {
    use crate::ported::zsh_h::{PM_ARRAY, PM_HASHED, PM_HIDEVAL, PM_UNIQUE};

    // sh:116 / sh:121 / sh:126 — `typeset -gHA …`.
    for name in [
        "_comps",
        "_services",
        "_patcomps",
        "_postpatcomps",
        "_compautos",
        "_lastcomp",
    ] {
        declare_global(name, PM_HASHED, PM_HIDEVAL);
    }

    // sh:129-134 — `_comp_dumpfile` defaults to
    // `${ZDOTDIR:-$HOME}/.zcompdump` and is overridden by `-d FILE`.
    // `typeset -g NAME=VALUE` assigns unconditionally, so an explicit
    // `-d` always wins; without one the default only fills an
    // empty/absent value.
    match dumpfile {
        Some(f) if !f.is_empty() => {
            let _ = crate::ported::params::setsparam("_comp_dumpfile", f); // sh:131
        }
        _ => {
            if crate::ported::params::getsparam("_comp_dumpfile")
                .map(|s| s.is_empty())
                .unwrap_or(true)
            {
                let _ = crate::ported::params::setsparam(
                    "_comp_dumpfile",
                    &default_dumpfile_path().to_string_lossy(),
                ); // sh:133
            }
        }
    }

    // sh:138-172 — `typeset -gHa _comp_options` + the option list.
    declare_global("_comp_options", PM_ARRAY, PM_HIDEVAL);
    crate::ported::params::setaparam(
        "_comp_options",
        COMP_OPTIONS.iter().map(|s| s.to_string()).collect(),
    );

    // sh:180-190 — `typeset -gH _comp_setup='…'`.
    declare_global("_comp_setup", 0, PM_HIDEVAL);
    let _ = crate::ported::params::setsparam("_comp_setup", COMP_SETUP_EVAL);

    // sh:195-197 — `typeset -ga compprefuncs comppostfuncs` then both
    // reset to empty.
    init_comp_funcs_arrays();

    // compdump sh:134-135 — `typeset -gUa _comp_assocs`. compdump
    // writes those two lines into every dump file, and `compinit -C`
    // reaches them by sourcing it (sh:493 `builtin . "$_comp_dumpfile"`).
    // zshrs parses the dump into its own cache instead of sourcing it,
    // so the declaration has to be made here to reach the same state.
    declare_global("_comp_assocs", PM_ARRAY, PM_UNIQUE);
}

/// sh:337 — `[[ -n "$autol" ]] && autoload -rUz "$func"`.
///
/// compinit registers every scanned completion file with `compdef -na
/// "${_i_name}" …` (sh:541), and the `-a` in that call is what makes
/// `compdef` run `autoload -rUz "$func"` at sh:337. The dump-file fast
/// path (sh:493 `builtin . "$_comp_dumpfile"`) reaches the same state
/// from compdump's single `autoload -Uz …` line. Either way a real zsh
/// finishes `compinit` with an autoload stub in `shfunctab` for EVERY
/// completer basename found in `$fpath`, and completers read that table:
/// `_tmux` builds its sub-command list from
/// `${(M)${(k)functions}:#_tmux-*}` (_tmux sh:1967).
///
/// zshrs bulk-loads `$_comps` from its own cache and materializes bodies
/// lazily, so it skipped this step entirely; `${(k)functions}` held only
/// the functions the session had actually defined.
///
/// Only names with no existing `shfunctab` entry get a stub, mirroring
/// `bin_functions`' behaviour of leaving an already-defined function
/// alone. Flags match `autoload -rUz`: `PM_UNDEFINED | PM_UNALIASED`
/// (c:Src/builtin.c:3352-3355) and `PM_ZSHSTORED` (c:3372). Returns the
/// number of stubs added.
pub fn register_autoload_stubs<I, S>(names: I) -> usize
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    use crate::ported::zsh_h::{PM_UNALIASED, PM_UNDEFINED, PM_ZSHSTORED};
    let flags = (PM_UNDEFINED | PM_UNALIASED | PM_ZSHSTORED) as i32;
    let mut added = 0usize;
    let names = names.into_iter();
    let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().write() else {
        return 0;
    };
    tab.reserve(names.size_hint().0);
    for name in names {
        let name = name.as_ref();
        if name.is_empty() || tab.contains_key(name) {
            continue;
        }
        let mut stub = crate::ported::hashtable::shfunc_autoload(name);
        stub.node.flags = flags;
        tab.add(stub);
        added += 1;
    }
    added
}

/// Autoload-stub names contributed by a completed scan — every file
/// whose header was `#compdef` or `#autoload`, which is exactly the set
/// compinit hands to `compdef -na` / `autoload -rUz` at sh:537-547.
pub fn autoload_stub_names(result: &CompInitResult) -> Vec<&str> {
    result
        .files
        .iter()
        .filter(|f| matches!(f.def, CompFileDef::CompDef(_) | CompFileDef::Autoload(_)))
        .map(|f| f.name.as_str())
        .collect()
}

/// sh:516 `builtin . "$_comp_dumpfile"` — the autoload half of sourcing
/// a dump file, extracted without executing it.
///
/// When a dump file exists, upstream compinit does NOT scan `$fpath` at
/// all (sh:491-518 sources the dump and sets `_i_done`, which skips the
/// whole sh:523-550 scan). The names in `${(k)functions}` after such a
/// compinit therefore come from the dump's `autoload` lines, and
/// compdump writes those from a DIFFERENT rule than compinit's scan:
///
///   compdump:113  `_d_als=($^fpath/(${(o~j.|.)$(typeset +fm '_*')})(N:t))`
///
/// i.e. every currently-DEFINED function whose name starts with `_` and
/// which also has a file somewhere in `$fpath` — no `#compdef` /
/// `#autoload` first line required. That is a strict superset of the
/// header-driven set `autoload_stub_names` derives from a scan: a
/// headerless helper the user's rc autoloaded by hand is in the dump but
/// invisible to any header-based scan. It can even name a function whose
/// file has since been deleted from `$fpath`, because the dump is a
/// snapshot (`autoload` on a missing file still creates the stub and only
/// errors when called).
///
/// Two line shapes are produced by compdump and both are parsed here:
///   compdump:118-129  one `autoload -Uz a b c \` + continuations, and
///   compdump:135-138  one `autoload -Uz <opts> <name>` per `$_compautos`.
/// Word-splitting mirrors the shell's: `autoload` must be the first word,
/// tokens starting with `-` or `+` are option words, the rest are names.
pub fn dump_autoload_names(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    let mut in_autoload = false;
    for line in text.lines() {
        let mut rest = line.trim();
        if !in_autoload {
            let Some(tail) = rest.strip_prefix("autoload") else {
                continue;
            };
            // "autoloadfoo" is not the `autoload` command.
            if !tail.is_empty() && !tail.starts_with(|c: char| c.is_ascii_whitespace()) {
                continue;
            }
            rest = tail.trim_start();
        }
        // A trailing `\` continues the command onto the next line.
        in_autoload = rest.ends_with('\\');
        if in_autoload {
            rest = rest[..rest.len() - 1].trim_end();
        }
        for word in rest.split_ascii_whitespace() {
            if word.starts_with('-') || word.starts_with('+') {
                continue; // `-Uz`, `+X`, …
            }
            names.push(word.to_string());
        }
    }
    names
}

/// The five association tables a dump file defines (compdump sh:31-72).
///
/// `IndexMap`, not `HashMap`: compdump writes every table in `${(ok)…}`
/// order and sourcing it reproduces exactly that insertion order, which
/// `${(k)_comps}` and — for `_patcomps`/`_postpatcomps` — pattern-match
/// precedence are both observable through.
#[derive(Debug, Default)]
pub struct DumpTables {
    /// `_comps=(…)`   — command -> completer
    pub comps: indexmap::IndexMap<String, String>,
    /// `_services=(…)` — command -> service name
    pub services: indexmap::IndexMap<String, String>,
    /// `_patcomps=(…)` — pattern -> completer (tried before `_comps`)
    pub patcomps: indexmap::IndexMap<String, String>,
    /// `_postpatcomps=(…)` — pattern -> completer (tried after `_comps`)
    pub postpatcomps: indexmap::IndexMap<String, String>,
    /// `_compautos=(…)` — autoload name -> extra `autoload` options
    pub compautos: indexmap::IndexMap<String, String>,
}

/// Split one dump line into words the way the shell would, honouring the
/// single-quoting `${(qq)}` applies in compdump (sh:38, 45, 52, 59, 64, 70).
///
/// `(qq)` renders an embedded `'` as `'\''`, so `'brew` is written
/// `''\''brew'` — three concatenated segments. Adjacent segments join into
/// one word, which is what makes plain `split_whitespace` wrong here.
fn split_quoted_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut it = line.chars();
    while let Some(c) = it.next() {
        match c {
            ' ' | '\t' => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                for d in it.by_ref() {
                    if d == '\'' {
                        break;
                    }
                    cur.push(d);
                }
            }
            '\\' => {
                started = true;
                if let Some(d) = it.next() {
                    cur.push(d);
                }
            }
            _ => {
                started = true;
                cur.push(c);
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// sh:494 `builtin . "$_comp_dumpfile"` — the association-table half of
/// sourcing a dump file, extracted without executing it.
///
/// On the `-C` path (sh:493-496) upstream sources the dump and sets
/// `_i_done`, and sh:501's `if [[ -z "$_i_done" ]]` then skips the whole
/// sh:504-528 `$fpath` scan. The dump is therefore the SOLE definition of
/// `_comps`/`_services`/`_patcomps`/`_postpatcomps`/`_compautos` for that
/// session — nothing else contributes a single key.
///
/// zshrs substituted its own SQLite cache for that payload, which is
/// refreshed on a different schedule than the shared `.zcompdump` and can
/// hold a partial scan. On this host the cache carried 1849 `_comps`
/// entries against the dump's 51745, so `$_comps[zpwr]` (and `cargo`,
/// `brew`, …) was simply absent, `_dispatch` fell through to `-default-`
/// and `zpwr <TAB>` completed FILES where zsh runs `_zpwr`.
///
/// compdump writes each table as a bare `NAME=(` line, one `'key' 'value'`
/// pair per line, and a bare `)` (sh:31-72); this parses exactly that
/// shape and ignores everything else in the file.
pub fn dump_assoc_tables(path: &Path) -> Option<DumpTables> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut tables = DumpTables::default();
    let mut open: Option<usize> = None;
    for line in text.lines() {
        if let Some(idx) = open {
            if line.trim_end() == ")" {
                open = None;
                continue;
            }
            let words = split_quoted_words(line);
            if words.len() >= 2 {
                let table = match idx {
                    0 => &mut tables.comps,
                    1 => &mut tables.services,
                    2 => &mut tables.patcomps,
                    3 => &mut tables.postpatcomps,
                    _ => &mut tables.compautos,
                };
                table.insert(words[0].clone(), words[1].clone());
            }
            continue;
        }
        open = match line.trim_end() {
            "_comps=(" => Some(0),
            "_services=(" => Some(1),
            "_patcomps=(" => Some(2),
            "_postpatcomps=(" => Some(3),
            "_compautos=(" => Some(4),
            _ => None,
        };
    }
    Some(tables)
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
    tracing::debug!(target: "compsys_args", "install_standard_complete_widgets ENTER");
    for w in STANDARD_COMPLETE_WIDGETS {
        // `zle -C <w> .<w> _main_complete` — args are post-flag:
        // [target-thingy, base-comp-widget, completion-func].
        let args = [
            w.to_string(),
            format!(".{}", w),
            "_main_complete".to_string(),
        ];
        let rc_w = crate::ported::zle::zle_thingy::bin_zle_complete("zle", &args, &empty_ops, 0);
        tracing::debug!(target: "compsys_args", widget = %w, rc_w, "zle -C standard widget");
        if rc_w == 0 {
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
        let rc_w = crate::ported::zle::zle_thingy::bin_zle_complete("zle", &args, &empty_ops, 0);
        tracing::debug!(target: "compsys_args", widget = "menu-select", rc_w, "zle -C standard widget");
        if rc_w == 0 {
            count += 1;
        }
    }
    count
}

/// `zmodload -i NAME` — mark a statically-linked module booted.
///
/// zsh has no `zmodload` call for these: referencing an autoloadable
/// parameter or builtin loads its module implicitly (`autoparamfn` /
/// `autobinfn`). zshrs registers those parameters and builtins at init
/// without going through the module, so the module's `MOD_INIT_B` bit —
/// the one `zmodload` / `zmodload -L` list on — never got set. Routing
/// the implicit load through the real builtin reaches the same state.
fn load_module_i(name: &str) {
    let mut ops = crate::ported::zsh_h::options {
        ind: [0u8; crate::ported::zsh_h::MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    };
    ops.ind[b'i' as usize] = 1;
    let _ = crate::ported::module::bin_zmodload("zmodload", &[name.to_string()], &ops, 0);
}

/// sh:201 — `: $funcstack`, with the upstream comment "Loading it now
/// ensures that the `funcstack' parameter is always correct."
///
/// `funcstack` is one of `zsh/parameter`'s autoloadable parameters
/// (Src/Modules/parameter.mdd), so that bare `:` command is a module
/// load in disguise — after any compinit, a real zsh lists
/// `zsh/parameter` in `zmodload -L`.
pub fn touch_funcstack_param() {
    load_module_i("zsh/parameter");
}

/// sh:564-569 — when the configured `completer` chain includes
/// `_expand` AND `^i` is currently bound to `expand-or-complete`,
/// rebind `^i` to `complete-word` so users don't get unexpected
/// glob expansion on TAB.
pub fn maybe_rebind_tab_for_expand() {
    // sh:572 — `zstyle -a ':completion:' completer _i_line`. `zstyle` is
    // `zsh/zutil`'s builtin (Src/Modules/zutil.mdd), so this line is what
    // pulls the module in on every compinit and leaves it in
    // `zmodload -L`. zshrs answers the query through the native
    // `lookupstyle` below instead of the builtin, so the module was never
    // marked booted: `zmodload -L` printed four lines against zsh's six.
    load_module_i("zsh/zutil");
    // sh:565 is the literal context `':completion:'` — NOT
    // `:completion:$curcontext:`. The two agree under a `:completion:*`
    // fixture, so the difference only shows with a scoped zstyle, but the
    // spec string is the unadorned one.
    let completers = crate::ported::modules::zutil::lookupstyle(":completion:", "completer");
    // sh:566 `(( ${_i_line[(i)_expand]} <= ${#_i_line} ))` is an EXACT
    // element match, so a named completer such as `_expand:foo` does not
    // arm the rebind in zsh either.
    let has_expand = completers.iter().any(|c| c == "_expand");
    if !has_expand {
        return;
    }
    // sh:563-564 — `bindkey '^i' | IFS=$' \t' read -A _i_line` /
    // `[[ ${_i_line[2]} = expand-or-complete ]]`. `_i_line[2]` is the
    // widget name `bindkey` prints after the quoted key sequence, so the
    // guard asks exactly what `bin_bindkey_list` resolves at
    // `zle_keymap.rs:1840-1846`: `keybind(km, getkeystring("^i"))` on the
    // keymap `bindkey` picks with no `-M`/-e/-v/-a, which
    // `bin_bindkey` (zle_keymap.rs:1250-1263) fixes at `main`. Reading the
    // table directly instead of running the builtin keeps `bindkey`'s
    // listing off stdout. Without this guard a user who had already put
    // their own widget on TAB got it silently clobbered by every compinit.
    let seq = crate::ported::zle::zle_bindings::getkeystring("^i");
    // `openkeymap` misses until `default_bindings()` has run; mirror the
    // same emptiness gate `bin_bindkey` uses (zle_keymap.rs:1121-1127) so
    // a compinit that precedes any `bindkey` call still sees the defaults,
    // and an already-built keymap is never rebuilt (that would wipe the
    // user's bindings).
    let km = crate::ported::zle::zle_keymap::openkeymap("main").or_else(|| {
        crate::ported::zle::zle_keymap::default_bindings();
        crate::ported::zle::zle_keymap::openkeymap("main")
    });
    let bound = km
        .and_then(|km| crate::ported::zle::zle_keymap::keybind(&km, &seq).0)
        .map(|t| t.nam);
    if bound.as_deref() != Some("expand-or-complete") {
        return;
    }
    // sh:568 — `bindkey '^i' complete-word`. `bindkey` is a BUILTIN, so it
    // must go through `bin_bindkey`; `dispatch_function_call` resolves only
    // shell functions and returned None here, making the whole rebind a
    // silent no-op (see the same warning in
    // `install_standard_complete_widgets`).
    let empty_ops = crate::ported::zsh_h::options {
        ind: [0u8; crate::ported::zsh_h::MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    };
    let bk_args = ["^i".to_string(), "complete-word".to_string()];
    let _ = crate::ported::zle::zle_keymap::bin_bindkey("bindkey", &bk_args, &empty_ops, 0);
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
    /// Pattern completion: #compdef -p 'pattern' [pattern...]
    Pattern(Vec<String>),
    /// Post-pattern completion: #compdef -P 'pattern' [pattern...]
    PostPattern(Vec<String>),
    /// One header that registers MORE THAN ONE kind, because `-N`/`-p`/`-P`
    /// switch the target table for the words that FOLLOW them (compinit
    /// sh:384-420). `_gcc`'s header is the canonical case:
    /// `#compdef gcc g++ … -value-,CFLAGS,-default- … -P gcc-* -P g++-* -P c++-*`
    /// — eight command names, six `-value-` contexts, then three
    /// post-patterns, all from one line.
    Mixed {
        commands: Vec<String>,
        patterns: Vec<String>,
        postpatterns: Vec<String>,
    },
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
    /// `#compdef -k <style> <key>...` widget bindings collected from file
    /// headers: (func, style, keys). Applied by the foreground compinit
    /// caller via `zle -C` + `bindkey` (upstream compinit sh:356-379) —
    /// the scan itself may run on a background thread and must not touch
    /// keymaps.
    pub keybindings: Vec<(String, String, Vec<String>)>,
    /// `#compdef -K <widget> <style> <key>` triplets: (widget, style, key, func).
    pub widgetkeys: Vec<(String, String, String, String)>,
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

        // sh:534-539 — ONLY a leading `-[pPkK]` (optionally `n`-suffixed)
        // is passed to compdef as an option:
        //     if [[ $_i_line[1] = -[pPkK](n|) ]]; then
        //       compdef ${_i_line[1]}na "$name" "${(@)_i_line[2,-1]}"
        //     else
        //       compdef -na "$name" "${_i_line[@]}"
        // Everything else — `-n`, `-m`, `-default-`, a bare `-` — is an
        // ordinary positional word, not a flag. (`_squishy`'s
        // `#compdef squishy "python -m squishy"` is read by `read -rA`,
        // which does no quote processing, so zsh really does register a
        // command literally named `-m`.)
        let leading = parts[0].strip_suffix('n').filter(|f| f.len() == 2);
        match leading.unwrap_or(parts[0]) {
            "-k" if parts.len() >= 3 => CompFileDef::CompDef(CompDef::KeyBinding {
                style: parts[1].to_string(),
                keys: parts[2..].iter().map(|s| s.to_string()).collect(),
            }),
            "-K" if parts.len() >= 4 => CompFileDef::CompDef(CompDef::WidgetKey {
                widget: parts[1].to_string(),
                style: parts[2].to_string(),
                key: parts[3].to_string(),
            }),
            flag => {
                // sh:384-420 — the positional loop. `type` starts at whatever
                // the leading flag set (sh:277-285) and is RE-SET by any `-N`
                // / `-p` / `-P` met along the way, so one header can feed
                // `_comps`, `_patcomps` and `_postpatcomps` at once. Handling
                // the flag only in first position lost every trailing `-P` in
                // the tree — `_gcc`'s `gcc-*`/`g++-*`/`c++-*`, `_lua`'s
                // `lua[0-9.-]##`, `_ruby`, `_php`, `_shasum`, `_rmlint`,
                // `_urls`, `_directories`, `_locales`, `_ccache` … 15 of
                // zsh's 25 `_postpatcomps` entries were missing, and the
                // patterns were wrongly registered as literal command names
                // in `_comps` instead.
                let mut ty = match flag {
                    "-p" => 1,
                    "-P" => 2,
                    _ => 0,
                };
                let start = usize::from(ty != 0);
                let mut commands: Vec<String> = Vec::new();
                let mut patterns: Vec<String> = Vec::new();
                let mut postpatterns: Vec<String> = Vec::new();
                for word in &parts[start..] {
                    match *word {
                        "-N" => ty = 0, // sh:385-386
                        "-p" => ty = 1, // sh:387-388
                        "-P" => ty = 2, // sh:389-390
                        _ => match ty {
                            1 => patterns.push(word.to_string()),
                            2 => postpatterns.push(word.to_string()),
                            _ => commands.push(word.to_string()),
                        },
                    }
                }
                match (
                    commands.is_empty(),
                    patterns.is_empty(),
                    postpatterns.is_empty(),
                ) {
                    (true, true, true) => CompFileDef::None,
                    (false, true, true) => CompFileDef::CompDef(CompDef::Commands(commands)),
                    (true, false, true) => CompFileDef::CompDef(CompDef::Pattern(patterns)),
                    (true, true, false) => CompFileDef::CompDef(CompDef::PostPattern(postpatterns)),
                    _ => CompFileDef::CompDef(CompDef::Mixed {
                        commands,
                        patterns,
                        postpatterns,
                    }),
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

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    // sh:507 — `for _i_file in $_i_dir/^(…)(N)`. A zsh glob yields its
    // matches SORTED, and that order decides which file claims a command
    // name first (compdef -n keeps the first claim, sh:393). `read_dir`
    // returns filesystem order, so without this sort the winner inside a
    // directory varied by inode layout.
    //
    // The comparator has to be zsh's, not Rust's byte-wise `Ord`:
    // c:Src/glob.c:1976 sorts matches with `gmatchcmp`, whose GS_NAME arm
    // (c:946) is `zstrcmp(…, 0)` → `strcoll` under the current LC_COLLATE.
    // Byte order put `_act-runner` (`-` = 0x2D) ahead of `_act_runner`
    // (`_` = 0x5F) while en_US.UTF-8 collation puts `_act_runner` first,
    // so ~40 commands whose completer has a `-`/`_` twin in the same
    // directory (`act_runner`, `amdgpu_top`, `cloud_sql_proxy`, `dh_make`,
    // …) got the WRONG file's function registered in `$_comps`.
    paths.sort_by(|a, b| {
        let name = |p: &Path| {
            p.file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        crate::ported::sort::zstrcmp(&name(a), &name(b), 0)
    });

    // Parallel scan of files within directory
    paths.par_iter().filter_map(|p| scan_file(p)).collect()
}

/// Initialize the completion system by scanning fpath
///
/// This is the main entry point - replaces the zsh compinit function.
/// Uses rayon for parallel directory and file scanning.
/// Apply the `#compdef -k`/`-K` widget key bindings a scan collected —
/// the in-shell half the scan can't do itself (it may run on a background
/// thread). Mirrors compdef's key branch: sh:365 `zle -C <func> .<style>
/// <func>`, sh:378 `bindkey <key> <func>`. Without this, header-declared
/// completion widgets parsed but never bound — `^X?` (_complete_debug,
/// `#compdef -k complete-word \C-x?`) and `^Xh` (_complete_help)
/// self-inserted literally instead of running.
pub fn apply_keybindings(result: &CompInitResult) {
    for (func, style, keys) in &result.keybindings {
        for key in keys {
            install_comp_keybinding(func, style, key, func);
        }
    }
    for (widget, style, key, func) in &result.widgetkeys {
        install_comp_keybinding(widget, style, key, func);
    }
}

/// One `#compdef -k`/`-K` binding: sh:346/365 `zle -C <widget> .<style>
/// <func>` + sh:351/378 `bindkey <key> <widget>`. Goes through the builtin
/// entries directly — `dispatch_function_call` resolves only shell
/// functions and silently no-ops for builtins (see
/// install_standard_complete_widgets).
fn install_comp_keybinding(widget: &str, style: &str, key: &str, func: &str) {
    let empty_ops = crate::ported::zsh_h::options {
        ind: [0u8; crate::ported::zsh_h::MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    };
    let style_dotted = if style.starts_with('.') {
        style.to_string()
    } else {
        format!(".{}", style)
    };
    // sh:346/365 — `zle -C <widget> <.style> <func>`.
    let zle_args = [widget.to_string(), style_dotted, func.to_string()];
    let _ = crate::ported::zle::zle_thingy::bin_zle_complete("zle", &zle_args, &empty_ops, 0);
    // sh:351/378 — `bindkey <key> <widget>`.
    let bk_args = [key.to_string(), widget.to_string()];
    let _ = crate::ported::zle::zle_keymap::bin_bindkey("bindkey", &bk_args, &empty_ops, 0);
}

/// The `#compdef -k`/`-K` headers shipped in the upstream Completion tree —
/// the fixed set compinit produces for a stock install (each cited from its
/// file's first line). Installed synchronously with the standard widget
/// rebind because the background scan's results merge lazily (and the
/// cached path skips the scan entirely); user fpath files with -k headers
/// are additionally collected by the scan into CompInitResult.keybindings.
const STANDARD_COMP_KEYBINDINGS: &[(&str, &str, &str, &str)] = &[
    // (widget, style, key, func)
    (
        "_complete_debug",
        "complete-word",
        "\u{18}?",
        "_complete_debug",
    ), // Base/Widget/_complete_debug:1 \C-x?
    (
        "_complete_help",
        "complete-word",
        "\u{18}h",
        "_complete_help",
    ), // Base/Widget/_complete_help:1 \C-xh
    ("_complete_tag", "complete-word", "\u{18}t", "_complete_tag"), // Base/Widget/_complete_tag:1 \C-xt
    (
        "_correct_filename",
        "complete-word",
        "\u{18}C",
        "_correct_filename",
    ), // _correct_filename:1 \C-xC
    ("_correct_word", "complete-word", "\u{18}c", "_correct_word"), // _correct_word:1 \C-xc
    ("_read_comp", "complete-word", "\u{18}\u{12}", "_read_comp"),  // _read_comp:1 \C-x\C-r
    (
        "_most_recent_file",
        "complete-word",
        "\u{18}m",
        "_most_recent_file",
    ), // _most_recent_file:1 \C-xm
    ("_next_tags", "list-choices", "\u{18}n", "_next_tags"),        // _next_tags:1 \C-xn
    ("_expand_word", "complete-word", "\u{18}e", "_expand_word"),   // _expand_word:1 -K (1st pair)
    (
        "_list_expansions",
        "list-choices",
        "\u{18}d",
        "_expand_word",
    ), // _expand_word:1 -K (2nd pair)
    (
        "_bash_complete-word",
        "complete-word",
        "\u{1b}~",
        "_bash_completions",
    ), // _bash_completions:1 -K
    (
        "_bash_list-choices",
        "list-choices",
        "\u{18}~",
        "_bash_completions",
    ), // _bash_completions:1 -K
    (
        "_history-complete-older",
        "complete-word",
        "\u{1b}/",
        "_history_complete_word",
    ), // _history_complete_word:1 -K
    (
        "_history-complete-newer",
        "complete-word",
        "\u{1b},",
        "_history_complete_word",
    ), // _history_complete_word:1 -K
    ("_expand_alias", "complete-word", "\u{18}a", "_expand_alias"), // Base/Completer/_expand_alias:1 -K
];

/// Install the stock `#compdef -k`/`-K` bindings (see
/// STANDARD_COMP_KEYBINDINGS). Runs on the main thread next to
/// install_standard_complete_widgets.
pub fn install_standard_comp_keybindings() {
    for (widget, style, key, func) in STANDARD_COMP_KEYBINDINGS {
        install_comp_keybinding(widget, style, key, func);
    }
}

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

    // Parallel scan of all directories. Files are read concurrently but the
    // RESULT keeps `$fpath` order (rayon's collect is order-preserving), which
    // the dedup below depends on.
    let scanned: Vec<CompFile> = fpath
        .par_iter()
        .filter(|dir| dir.as_os_str() != "." && dir.exists())
        .flat_map(|dir| scan_directory(dir))
        .collect();

    // sh:509 — `(( $+_i_test[$_i_name] … )) && continue`: the FIRST `$fpath`
    // directory holding a given completer filename wins; later copies are
    // skipped. The previous code ran this dedup INSIDE the parallel filter
    // against a shared `seen` set, so the surviving copy was whichever thread
    // won the race — not the first in fpath order. Doing it here, over the
    // already-ordered vector, makes the winner deterministic and correct.
    let mut seen: HashSet<String> = HashSet::new();
    let all_files: Vec<CompFile> = scanned
        .into_iter()
        .filter(|f| seen.insert(f.name.clone()))
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
                            // sh:519 — compinit registers every scanned file with
                            // `compdef -na`, and `-n` (new) means sh:393
                            // `if [[ -z "$new" || -z "${_comps[$1]}" ]]` — an
                            // EXISTING entry is kept, so the first `$fpath`
                            // directory to claim a command owns it. zshrs inserted
                            // unconditionally (last writer won), so with two files
                            // claiming the same command (`_df` at fpath[24] has
                            // `#compdef df gdf`; zsh-more-completions'
                            // `_dwarffortress` at fpath[42] has `#compdef
                            // dwarffortress df`) `df` completed as Dwarf Fortress
                            // and `df -<TAB>` produced nothing.
                            //
                            // Handle service syntax: cmd=service
                            if let Some(eq_pos) = cmd.find('=') {
                                let cmd_name = &cmd[..eq_pos];
                                let service = &cmd[eq_pos + 1..];
                                if !result.comps.contains_key(cmd_name) {
                                    result.comps.insert(cmd_name.to_string(), file.name.clone());
                                    // sh:395 — `_services[$cmd]` is set inside the
                                    // same guard, never on its own.
                                    result
                                        .services
                                        .insert(cmd_name.to_string(), service.to_string());
                                }
                            } else if !result.comps.contains_key(cmd) {
                                result.comps.insert(cmd.clone(), file.name.clone());
                            }
                        }
                    }
                    CompDef::Pattern(pats) => {
                        // c:compinit sh:396 — `_patcomps[$1]="$func"`. Pattern
                        // compdefs go to `_patcomps` ONLY, never `_comps`.
                        for pat in pats {
                            result.patcomps.insert(pat.clone(), file.name.clone());
                        }
                    }
                    CompDef::PostPattern(pats) => {
                        // c:compinit sh:403 — `_postpatcomps[$1]="$func"`.
                        for pat in pats {
                            result.postpatcomps.insert(pat.clone(), file.name.clone());
                        }
                    }
                    // One header feeding several tables (sh:384-420). Each
                    // list lands in exactly the table its `-N`/`-p`/`-P`
                    // prefix selected; the command half keeps the same
                    // first-claim-wins + `cmd=service` handling as above.
                    CompDef::Mixed {
                        commands,
                        patterns,
                        postpatterns,
                    } => {
                        for cmd in commands {
                            if let Some(eq_pos) = cmd.find('=') {
                                let cmd_name = &cmd[..eq_pos];
                                let service = &cmd[eq_pos + 1..];
                                if !result.comps.contains_key(cmd_name) {
                                    result.comps.insert(cmd_name.to_string(), file.name.clone());
                                    result
                                        .services
                                        .insert(cmd_name.to_string(), service.to_string());
                                }
                            } else if !result.comps.contains_key(cmd) {
                                result.comps.insert(cmd.clone(), file.name.clone());
                            }
                        }
                        for pat in patterns {
                            result.patcomps.insert(pat.clone(), file.name.clone());
                        }
                        for pat in postpatterns {
                            result.postpatcomps.insert(pat.clone(), file.name.clone());
                        }
                    }
                    CompDef::KeyBinding { style, keys } => {
                        // sh:356-379 — `#compdef -k <style> <keys…>`: the widget
                        // takes the FILE's name (e.g. _complete_debug). Collected
                        // here; zle -C + bindkey happen in apply_keybindings
                        // (this scan may run on a background thread).
                        result
                            .keybindings
                            .push((file.name.clone(), style.clone(), keys.clone()));
                    }
                    CompDef::WidgetKey { widget, style, key } => {
                        // sh:336-354 — `#compdef -K <widget> <style> <key>`:
                        // the completion FUNCTION is the file's name, the
                        // widget name is explicit.
                        result.widgetkeys.push((
                            widget.clone(),
                            style.clone(),
                            key.clone(),
                            file.name.clone(),
                        ));
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
        publish_compdef_state_mut(s);
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

    // Populate postpatcomps table (_postpatcomps hash). These are the
    // `#compdef -P pat` entries (compinit sh:404). They must NOT go into
    // `patcomps`: `_dispatch` walks `_patcomps` before the `$_comps` name
    // lookup (sh:26) and `_postpatcomps` after it (sh:71), and only the
    // post pass sets `_compskip=default` (sh:72), which is what suppresses
    // the sh:84 default-completer fallback. Merging the two tables ran
    // every `-P` completer in the wrong phase without that flag, so
    // `PATH=/usr/bin:<TAB>` ran `_dir_list` and then ALSO fell through to
    // `_value` -> `_default`, listing every file instead of directories.
    for (pattern, function) in &result.postpatcomps {
        cache
            .set_postpatcomp(pattern, function)
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

    // Load postpatcomps - single query. Without this the `-C` cache-hit path
    // published an EMPTY `_postpatcomps` (ext_builtins.rs, sh:116/121), so
    // `_dispatch`'s post pass had nothing to walk and every `#compdef -P`
    // completer (`_dir_list`, `_urls`, `_locales`, `_gcc`, `_python`, …) was
    // dead on any shell that started from the cache.
    for (pat, func) in cache
        .postpatcomps_kv()
        .map_err(|e| std::io::Error::other(e.to_string()))?
    {
        result.postpatcomps.insert(pat, func);
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

/// Metadata key holding the `comps` row count a cache build finished
/// with. Written as the LAST statement of `build_cache_from_fpath`'s
/// caller, after every table is populated — see
/// `stamp_cache_complete` and `cache_is_valid`.
pub const CACHE_COMPLETE_KEY: &str = "comps_rows_at_build_end";

/// Record that a freshly built cache is complete.
///
/// Must be the final write of a build. Pairs with `cache_is_valid`.
pub fn stamp_cache_complete(cache: &crate::compsys::cache::CompsysCache) -> bool {
    match cache.comp_count() {
        Ok(n) => cache
            .set_metadata(CACHE_COMPLETE_KEY, &n.to_string())
            .is_ok(),
        Err(_) => false,
    }
}

/// Check if cache is valid and up-to-date
///
/// Returns true only for a cache some build finished writing; false when
/// it is empty, unstamped, or still filling.
///
/// A bare `comp_count() > 0` was NOT a validity test. The rebuild path
/// spends seconds inserting ~50k `comps` rows, and up to 16 of the user's
/// shells run against this one file, so a shell starting inside that
/// window saw a non-zero partial count, took the cache-hit branch, and
/// published a `_comps` holding a fraction of the registrations —
/// `_dispatch` then resolved no completer for any command and every
/// `<cmd> <TAB>` completed nothing. Matching the stamped count against
/// the live count makes "still filling" and "builder died midway" both
/// fail: the stamp is written once, at the end, and any later insert
/// moves the live count away from it.
pub fn cache_is_valid(cache: &crate::compsys::cache::CompsysCache) -> bool {
    let rows = cache.comp_count().unwrap_or(0);
    if rows <= 0 {
        return false;
    }
    match cache.get_metadata(CACHE_COMPLETE_KEY) {
        Ok(Some(stamp)) => stamp.parse::<i64>().map(|n| n == rows).unwrap_or(false),
        _ => false,
    }
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
// `assoc_get("_comps")` etc.; the published-to-paramtab step happens
// in `publish_compdef_state_mut` at the bottom of this section, which
// MERGES into those parameters rather than rebuilding them (see
// `merge_hparam`) — the state is one contributor to `_comps`, never
// its definition.
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
    /// Keys a `compdef -d` (sh:426-444 `unset "_comps[$^@]"`) removed and
    /// that the next publish still has to unset on the shell side.
    /// Removal is the one edit a merge cannot express, so it is carried
    /// explicitly instead of being implied by the absence of a key.
    removed: CompdefRemovals,
}

/// Per-array key lists for pending `compdef -d` removals.
#[derive(Default)]
struct CompdefRemovals {
    comps: Vec<String>,
    services: Vec<String>,
    patcomps: Vec<String>,
    postpatcomps: Vec<String>,
}

static COMPDEF_STATE: Mutex<Option<CompdefState>> = Mutex::new(None);

/// Depth of the enclosing `compdef_batch` calls. See that function.
static PUBLISH_DEPTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

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

/// Run `f` with shell-side publication held until it returns, then
/// publish once.
///
/// Publication is a whole-hash read-modify-write: `gethparam`/`sethparam`
/// are the only associative-array accessors `params.rs` exposes, so there
/// is no way to assign a single `_comps[$cmd]` the way sh:376 does. A bulk
/// replay (`cdreplay` after zinit turbo) calls `compdef` once per deferred
/// registration and would otherwise pay that read-modify-write over a
/// 50k-entry `_comps` on every one of them. The batched end state is
/// identical — the same accumulated `CompdefState`, published once.
pub fn compdef_batch<R>(f: impl FnOnce() -> R) -> R {
    use std::sync::atomic::Ordering;
    // Restores the depth even if `f` panics or returns early.
    struct Depth;
    impl Drop for Depth {
        fn drop(&mut self) {
            PUBLISH_DEPTH.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    PUBLISH_DEPTH.fetch_add(1, Ordering::Relaxed);
    let out = {
        let _depth = Depth;
        f()
    };
    with_state(publish_compdef_state_mut);
    out
}

/// Apply one array's pending edits to its shell-side associative array.
///
/// `set` is overlaid onto whatever the parameter already holds and
/// `remove` is unset from it — the parameter is never rebuilt from `set`
/// alone. That distinction is the whole fix: `CompdefState` holds only
/// what THIS process's `compdef` calls (and its own `$fpath` scan)
/// registered, while `compinit -C`'s cache-hit path fills the parameter
/// directly (`ext_builtins.rs`, `set_assoc`) without going through the
/// state at all. Publishing `flatten(&s.comps)` wholesale therefore
/// replaced a 51 647-entry `_comps` with however many keys the state
/// happened to have — one, for a session whose only `compdef` was
/// `_zstyle zstyle` — and `_dispatch` then resolved an empty completer
/// for EVERY command, so `man <TAB>`, `git <TAB>` and `kill <TAB>` all
/// completed nothing at all.
fn merge_hparam(name: &str, set: &HashMap<String, String>, remove: &[String]) {
    if set.is_empty() && remove.is_empty() {
        return;
    }
    // BTreeMap so the published pair order is deterministic, matching what
    // the previous sort-by-key flatten produced. The read goes through
    // `subst::assoc_get` because that is the accessor backed by the same
    // `paramtab_hashed_storage` `sethparam` writes — `gethparam` returns
    // the VALUES only (params.rs:5723-5728), not the key/value pairs.
    let mut merged: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    if let Some(existing) = crate::ported::subst::assoc_get(name) {
        for (k, v) in existing {
            merged.insert(k, v);
        }
    }
    for k in remove {
        merged.remove(k);
    }
    for (k, v) in set {
        merged.insert(k.clone(), v.clone());
    }
    let mut out = Vec::with_capacity(merged.len() * 2);
    for (k, v) in merged {
        out.push(k);
        out.push(v);
    }
    // These are ASSOCIATIVE arrays in zsh (`typeset -gHA _comps _services
    // _patcomps _postpatcomps` / `_compautos`, sh:116/121). They MUST be
    // published via `sethparam` (hashed) — the `[k, v, k, v, …]` pair
    // layout above is what `sethparam` consumes. Using `setaparam` (plain
    // array) made `_comps` a flat array, so `${_comps[ls]}` couldn't
    // hash-lookup and `_complete`/`_dispatch` found no completer for ANY
    // command → every `<cmd> <TAB>` (incl. `ls -<TAB>`) just rang the bell.
    // The synchronous `compinit -C` path used `set_assoc` (→ sethparam) and
    // worked, which is why only the fresh/compdef-driven path was broken.
    // Bug #655.
    crate::ported::params::sethparam(name, out);
}

/// Publish the in-memory state into the shell-side assoc arrays so
/// engine cluster code (which reads via `getaparam("_comps")` etc.)
/// sees the updates.
///
/// A no-op inside `compdef_batch`; the batch publishes on exit.
fn publish_compdef_state_mut(s: &mut CompdefState) {
    use std::sync::atomic::Ordering;
    if PUBLISH_DEPTH.load(Ordering::Relaxed) > 0 {
        return;
    }
    merge_hparam("_comps", &s.comps, &s.removed.comps);
    merge_hparam("_services", &s.services, &s.removed.services);
    merge_hparam("_patcomps", &s.patcomps, &s.removed.patcomps);
    merge_hparam("_postpatcomps", &s.postpatcomps, &s.removed.postpatcomps);
    merge_hparam("_compautos", &s.compautos, &[]);
    s.removed = CompdefRemovals::default();
}

/// Read one key out of a shell-side assoc array.
///
/// The parameter — not `CompdefState` — is the source of truth for what
/// is registered: `compinit -C` fills it from the dump/cache without
/// touching the state (see `merge_hparam`).
fn hparam_has_key(name: &str, key: &str) -> bool {
    crate::ported::subst::assoc_get(name)
        .map(|m| m.contains_key(key))
        .unwrap_or(false)
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
        // sh:426-444  -d: delete by name from the right hash. The key has
        // to be dropped from the session state AND queued for removal from
        // the shell parameter — a key registered by `compinit -C`'s cache
        // load is only ever in the parameter, so removing it from the state
        // alone would leave `compdef -d` a no-op for everything the dump
        // defined.
        let names = &args[idx..];
        with_state(|s| match flags.spec_type {
            SpecType::Pattern => {
                for n in names {
                    s.patcomps.remove(n);
                    s.removed.patcomps.push(n.clone());
                }
            }
            SpecType::PostPattern => {
                for n in names {
                    s.postpatcomps.remove(n);
                    s.removed.postpatcomps.push(n.clone());
                }
            }
            SpecType::Key | SpecType::WidgetKey => {
                eprintln!("compdef: cannot restore key bindings");
            }
            SpecType::Normal => {
                for n in names {
                    s.comps.remove(n);
                    s.services.remove(n);
                    s.removed.comps.push(n.clone());
                    s.removed.services.push(n.clone());
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
            // sh:307-311 — resolve `$svc` and look up its completion
            //   function. zsh reads the `_comps`/`_services` PARAMETERS
            //   (`func="$_comps[...]"`), which are the source of truth:
            //   the `compinit -C` dump-source path and third-party plugins
            //   populate the parameters directly, while the internal
            //   `s.comps` state only sees native `compdef` calls — so it
            //   lags and made `compdef func=cmd` wrongly report
            //   "unknown command or service" even though `$_comps[cmd]`
            //   was set. Read the parameters first, fall back to s.comps.
            let comps_param = crate::ported::subst::assoc_get("_comps").unwrap_or_default();
            // sh:307 — `${_services[(r)$svc]:-$svc}`. `(r)` returns the
            //   matching VALUE (== $svc for a literal), else the `:-$svc`
            //   default, so the effective service key is `$svc` itself.
            let resolved_svc = svc_in.clone();
            let func = comps_param
                .get(&resolved_svc)
                .filter(|f| !f.is_empty())
                .cloned()
                .or_else(|| {
                    with_state(|s| s.comps.get(&resolved_svc).cloned()).filter(|f| !f.is_empty())
                })
                .or_else(|| {
                    // sh:311 fallback to first matching pat/postpat key,
                    //   again preferring the parameters over s.* state.
                    let pat = crate::ported::subst::assoc_get("_patcomps").unwrap_or_default();
                    let postpat =
                        crate::ported::subst::assoc_get("_postpatcomps").unwrap_or_default();
                    pat.iter()
                        .find(|(k, _)| pattern_matches(k, &svc_in))
                        .map(|(_, v)| v.clone())
                        .or_else(|| {
                            postpat
                                .iter()
                                .find(|(k, _)| pattern_matches(k, &svc_in))
                                .map(|(_, v)| v.clone())
                        })
                        .or_else(|| {
                            with_state(|s| {
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
                        })
                })
                .unwrap_or_default();
            if func.is_empty() {
                eprintln!("compdef: unknown command or service: {}", svc_in);
                ret = 1;
                continue;
            }
            // sh:308-309 — `[[ -n ${_services[$svc]} ]] && svc=${_services[$svc]}`.
            let services_param = crate::ported::subst::assoc_get("_services").unwrap_or_default();
            let svc_for_state = services_param
                .get(&svc_in)
                .filter(|v| !v.is_empty())
                .cloned()
                .or_else(|| with_state(|s| s.services.get(&svc_in).cloned()))
                .unwrap_or(svc_in.clone());
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
                // sh:346 `zle -C` + sh:347-352 `bindkey` — through the
                // BUILTIN entries; dispatch_function_call resolves only
                // shell functions and silently no-ops for builtins, which
                // left every `#compdef -K` widget (^Xe _expand_word,
                // \e/ _history-complete-older, …) unbound.
                install_comp_keybinding(&wname, &comp_widget, &key, &func);
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
            // sh:365 `zle -C` + sh:373-378 `bindkey` — through the BUILTIN
            // entries (see the -K arm above): the dispatch_function_call
            // spelling silently no-op'd, so `#compdef -k` widgets — ^X?
            // _complete_debug, ^Xh _complete_help — never bound and the
            // keys self-inserted literally.
            for key in &args[idx..] {
                install_comp_keybinding(&func, &style, key, &func);
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
                        // sh:415 — `-n`: no-clobber. zsh tests
                        // `[[ -z ${_comps[$1]} ]]` against the PARAMETER, so
                        // an entry that came from the dump/cache load counts
                        // as already-defined; testing only the session state
                        // let every `compdef -na` from an fpath rescan
                        // overwrite the dump's registration.
                        if flags.new
                            && (s.comps.contains_key(&cmd) || hparam_has_key("_comps", &cmd))
                        {
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
///
/// Clears the shell-side parameters too. Publication merges into them
/// (`merge_hparam`), so they outlive the `CompdefState` and would carry
/// one case's registrations into the next — `-n`'s no-clobber test reads
/// `_comps` directly, and a leftover `_comps[git]` would silently skip
/// the registration the next case is asserting on.
#[cfg(test)]
pub fn reset_compdef_state() {
    *COMPDEF_STATE.lock().unwrap() = Some(CompdefState::default());
    for name in [
        "_comps",
        "_services",
        "_patcomps",
        "_postpatcomps",
        "_compautos",
    ] {
        crate::ported::params::sethparam(name, Vec::new());
    }
}

/// Snapshot of the session-side state — what THIS process's `compdef`
/// calls and `$fpath` scan registered.
///
/// Not the full `_comps`: a `compinit -C` that loaded the dump/SQLite
/// cache fills the shell parameter without going through this state, so
/// read `subst::assoc_get("_comps")` when the question is "what is
/// registered", and this when the question is "what did we register".
pub fn snapshot_compdef_state() -> CompdefState {
    with_state(|s| CompdefState {
        comps: s.comps.clone(),
        services: s.services.clone(),
        patcomps: s.patcomps.clone(),
        postpatcomps: s.postpatcomps.clone(),
        compautos: s.compautos.clone(),
        removed: CompdefRemovals::default(),
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
            CompFileDef::CompDef(CompDef::Pattern(pats)) => {
                assert_eq!(pats, vec!["'c*'".to_string()]);
            }
            _ => panic!("Expected Pattern"),
        }
    }

    // Bug #657 — `#compdef -P pat` must route to `_postpatcomps` (post-pattern),
    // NOT `_comps`; `#compdef -p pat` to `_patcomps` only, NOT `_comps`.
    #[test]
    fn test_parse_compdef_postpattern_routing() {
        match parse_first_line("#compdef -P 'pip[0-9.]#'") {
            CompFileDef::CompDef(CompDef::PostPattern(pats)) => {
                assert_eq!(pats, vec!["'pip[0-9.]#'".to_string()]);
            }
            other => panic!("Expected PostPattern, got {:?}", other),
        }
    }

    /// sh:384-420 — `-N`/`-p`/`-P` are POSITIONAL: they re-target the words
    /// that follow, anywhere in the line. Reading a flag only in first
    /// position registered `_gcc`'s three post-patterns as literal command
    /// names, so `gcc-14 <TAB>` never reached `_gcc` and `$_postpatcomps`
    /// held 10 entries where zsh 5.9.2 holds 25.
    #[test]
    fn test_parse_compdef_trailing_flags_switch_table() {
        let line = "#compdef gcc g++ -value-,CFLAGS,-default- -P gcc-* -P g++-* -p early*";
        match parse_first_line(line) {
            CompFileDef::CompDef(CompDef::Mixed {
                commands,
                patterns,
                postpatterns,
            }) => {
                assert_eq!(commands, vec!["gcc", "g++", "-value-,CFLAGS,-default-"]);
                assert_eq!(patterns, vec!["early*"]);
                assert_eq!(postpatterns, vec!["gcc-*", "g++-*"]);
            }
            other => panic!("Expected Mixed, got {:?}", other),
        }
        // `-N` switches back to plain command names (sh:385-386).
        match parse_first_line("#compdef -p pat* -N cmd") {
            CompFileDef::CompDef(CompDef::Mixed {
                commands,
                patterns,
                postpatterns,
            }) => {
                assert_eq!(commands, vec!["cmd"]);
                assert_eq!(patterns, vec!["pat*"]);
                assert!(postpatterns.is_empty());
            }
            other => panic!("Expected Mixed, got {:?}", other),
        }
        // sh:534-539 — only a leading `-[pPkK](n|)` is an option. Anything
        // else is a positional NAME, including `-m` (which is how zsh ends
        // up with a command literally named `-m` from `_squishy`'s header).
        match parse_first_line(r#"#compdef squishy "python -m squishy""#) {
            CompFileDef::CompDef(CompDef::Commands(cmds)) => {
                assert_eq!(cmds, vec!["squishy", "\"python", "-m", "squishy\""]);
            }
            other => panic!("Expected Commands, got {:?}", other),
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
    fn compdef_no_clobber_honours_a_registration_only_the_parameter_holds() {
        // sh:415 tests `[[ -z ${_comps[$1]} ]]` — the PARAMETER. Everything
        // `compinit -C` loaded from the dump lives only there, so checking
        // `CompdefState` alone let a later `compdef -n` overwrite it.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        crate::ported::params::sethparam(
            "_comps",
            vec!["git".to_string(), "_git_from_dump".to_string()],
        );
        run(&["-n", "_second", "git"]);
        assert_eq!(
            crate::ported::subst::assoc_get("_comps")
                .and_then(|m| m.get("git").cloned())
                .as_deref(),
            Some("_git_from_dump")
        );
    }

    #[test]
    fn compdef_keeps_registrations_it_did_not_make() {
        // The regression this whole merge-on-publish design exists for.
        // `compinit -C`'s cache-hit path fills `_comps` directly
        // (ext_builtins.rs, `set_assoc`) and never touches `CompdefState`,
        // so publishing the state wholesale replaced ~51k registrations
        // with the one key this process happened to register — after which
        // `_dispatch` resolved an empty completer for every command and
        // `man <TAB>` / `git <TAB>` / `kill <TAB>` all produced nothing.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        crate::ported::params::sethparam(
            "_comps",
            vec![
                "man".to_string(),
                "_man".to_string(),
                "git".to_string(),
                "_git".to_string(),
            ],
        );
        assert_eq!(run(&["_zstyle", "zstyle"]), 0);
        let comps = crate::ported::subst::assoc_get("_comps").expect("_comps must still be a hash");
        assert_eq!(comps.get("man").map(String::as_str), Some("_man"));
        assert_eq!(comps.get("git").map(String::as_str), Some("_git"));
        assert_eq!(comps.get("zstyle").map(String::as_str), Some("_zstyle"));
    }

    #[test]
    fn compdef_delete_removes_a_key_only_the_parameter_holds() {
        // The flip side: sh:442 `unset "_comps[$^@]"` has to reach an entry
        // that came from the dump, which a merge cannot express by omission.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        crate::ported::params::sethparam(
            "_comps",
            vec![
                "man".to_string(),
                "_man".to_string(),
                "git".to_string(),
                "_git".to_string(),
            ],
        );
        assert_eq!(run(&["-d", "man"]), 0);
        let comps = crate::ported::subst::assoc_get("_comps").expect("_comps must still be a hash");
        assert_eq!(comps.get("man"), None);
        assert_eq!(comps.get("git").map(String::as_str), Some("_git"));
    }

    #[test]
    fn compdef_batch_defers_publication_but_still_publishes() {
        // The batch must be a deferral, not a drop: a `cdreplay` whose
        // registrations never reached `_comps` would be the same outage
        // by another route.
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        compdef_batch(|| {
            run(&["_git", "git"]);
            assert!(
                crate::ported::subst::assoc_get("_comps")
                    .map(|m| m.is_empty())
                    .unwrap_or(true),
                "publication must be held until the batch ends"
            );
            run(&["_man", "man"]);
        });
        let comps = crate::ported::subst::assoc_get("_comps").expect("_comps must still be a hash");
        assert_eq!(comps.get("git").map(String::as_str), Some("_git"));
        assert_eq!(comps.get("man").map(String::as_str), Some("_man"));
    }

    #[test]
    fn cache_is_valid_rejects_a_cache_that_is_still_filling() {
        // A row count alone cannot distinguish "finished" from "another
        // shell is 200 rows into a 50k-row rebuild" — accepting the latter
        // is what published a `_comps` with a handful of entries.
        let cache = crate::compsys::cache::CompsysCache::memory().expect("in-memory cache");
        assert!(!cache_is_valid(&cache), "an empty cache is not valid");
        cache.set_comp("git", "_git").unwrap();
        assert!(
            !cache_is_valid(&cache),
            "a cache no build has stamped is not valid, however many rows it has"
        );
        assert!(stamp_cache_complete(&cache));
        assert!(cache_is_valid(&cache));
        cache.set_comp("man", "_man").unwrap();
        assert!(
            !cache_is_valid(&cache),
            "a row written after the stamp means the build was not the last writer"
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
        // `_comps` is an ASSOCIATIVE array in zsh (`typeset -gHA`), so the
        // shell-side view must be a hash where `${_comps[git]}` == `_git`.
        // (Previously published via setaparam as a flat array, which broke
        // `${_comps[cmd]}` key lookup and every completion — Bug #655.)
        let _g = crate::test_util::global_state_lock();
        reset_compdef_state();
        run(&["_git", "git"]);
        // Must be a proper association, not a flat array.
        let map = crate::ported::params::paramtab_hashed_storage()
            .lock()
            .unwrap()
            .get("_comps")
            .cloned()
            .expect("_comps must be a hashed (associative) param");
        assert_eq!(map.get("git").map(String::as_str), Some("_git"));
    }

    /// compinit registers every scanned file with `compdef -na`, and `-n`
    /// keeps an EXISTING `_comps` entry (Completion/compinit sh:393). So
    /// when two completers claim the same command, the one in the earlier
    /// `$fpath` directory owns it.
    ///
    /// Regression: the scan inserted unconditionally, so the LAST writer
    /// won. On this host `_df` (`#compdef df gdf`, fpath[24]) lost to
    /// zsh-more-completions' `_dwarffortress` (`#compdef dwarffortress
    /// df`, fpath[42]) and `df -<TAB>` completed Dwarf Fortress options,
    /// i.e. nothing.
    #[test]
    fn scan_keeps_first_fpath_claim_on_a_command() {
        let _g = crate::test_util::global_state_lock();
        let base = std::env::temp_dir().join("zshrs_compinit_firstwins_test");
        let early = base.join("early");
        let late = base.join("late");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&early).unwrap();
        fs::create_dir_all(&late).unwrap();
        // Same command claimed by two different files in two directories.
        fs::write(early.join("_zzcmd"), "#compdef zzcmd zzother\n").unwrap();
        fs::write(late.join("_zzgame"), "#compdef zzgame zzcmd\n").unwrap();

        let result = compinit(&[early.clone(), late.clone()]);
        assert_eq!(
            result.comps.get("zzcmd").map(String::as_str),
            Some("_zzcmd"),
            "earlier fpath dir must keep the command"
        );
        // The later file still owns the commands nobody claimed first.
        assert_eq!(
            result.comps.get("zzgame").map(String::as_str),
            Some("_zzgame")
        );

        // Reversing fpath order reverses the winner — order is what decides.
        let reversed = compinit(&[late.clone(), early.clone()]);
        assert_eq!(
            reversed.comps.get("zzcmd").map(String::as_str),
            Some("_zzgame")
        );
        let _ = fs::remove_dir_all(&base);
    }

    /// Regression: `compinit` must leave an autoload stub in `shfunctab`
    /// for every completer it registers (sh:337 `autoload -rUz "$func"`,
    /// reached via the `compdef -na` at sh:541), because completers read
    /// `$functions` to discover their siblings — `_tmux` derives its
    /// sub-command list from `${(M)${(k)functions}:#_tmux-*}`
    /// (_tmux sh:1967). zshrs bulk-loaded `$_comps` without this step, so
    /// `tmux <TAB>` was missing the five `_tmux-*` helpers in `$fpath`.
    #[test]
    fn scan_registers_autoload_stubs_for_every_completer() {
        let _g = crate::test_util::global_state_lock();
        let dir = std::env::temp_dir().join("zshrs_compinit_stubs_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("_zzt"), "#compdef zzt\n").unwrap();
        fs::write(dir.join("_zzt-helper"), "#compdef zzt-helper\n").unwrap();
        fs::write(dir.join("_zzt_util"), "#autoload\n").unwrap();
        // A file with neither header contributes no function.
        fs::write(dir.join("_zzt_readme"), "just text\n").unwrap();

        // A name already defined must survive untouched — `bin_functions`
        // leaves an existing function alone.
        for n in ["_zzt", "_zzt-helper", "_zzt_util", "_zzt_readme"] {
            if let Ok(mut t) = crate::ported::hashtable::shfunctab_lock().write() {
                t.remove(n);
            }
        }
        if let Ok(mut t) = crate::ported::hashtable::shfunctab_lock().write() {
            let mut defined = crate::ported::hashtable::shfunc_autoload("_zzt");
            defined.node.flags = 0;
            defined.body = Some("true".to_string());
            t.add(defined);
        }

        let result = compinit(&[dir.clone()]);
        let names = autoload_stub_names(&result);
        assert!(names.contains(&"_zzt-helper"), "got {names:?}");
        assert!(names.contains(&"_zzt_util"), "got {names:?}");
        assert!(!names.contains(&"_zzt_readme"), "got {names:?}");

        assert_eq!(
            register_autoload_stubs(&names),
            2,
            "the already-defined _zzt must not be re-stubbed"
        );

        let tab = crate::ported::hashtable::shfunctab_lock();
        let tab = tab.read().unwrap();
        for n in ["_zzt-helper", "_zzt_util"] {
            let shf = tab.get(n).unwrap_or_else(|| panic!("{n} has no stub"));
            let flags = shf.node.flags as u32;
            assert!(flags & crate::ported::zsh_h::PM_UNDEFINED != 0, "{n}");
            assert!(flags & crate::ported::zsh_h::PM_UNALIASED != 0, "{n}");
        }
        assert_eq!(
            tab.get("_zzt").and_then(|f| f.body.clone()),
            Some("true".to_string()),
            "an already-defined function must keep its body"
        );
        assert!(tab.get("_zzt_readme").is_none());
        drop(tab);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Regression: `compinit -C -d FILE` takes the sh:515-518 branch, which
    /// sources the dump instead of scanning `$fpath` — so the dump's
    /// `autoload` lines, not a header scan, decide what ends up in
    /// `${(k)functions}`. compdump lists every defined `_*` function that
    /// has a file in `$fpath` (compdump:113), which is why the real dump on
    /// a zpwr host names 12 headerless helpers (`_command_names`,
    /// `__zpwr_aliases`, …) that no `#compdef`/`#autoload` scan can find.
    /// This asserts the parse of both line shapes compdump emits: the one
    /// backslash-continued `autoload -Uz a b c` list (compdump:118-129) and
    /// the per-`$_compautos` `autoload -Uz <opts> <name>` lines
    /// (compdump:135-138).
    #[test]
    fn dump_autoload_names_reads_both_compdump_line_shapes() {
        let dir = std::env::temp_dir().join("zshrs_compinit_dumpnames_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let dump = dir.join("zcompdump");
        fs::write(
            &dump,
            concat!(
                "#files: 3\tversion: 5.9.2\n",
                "\n",
                "_comps=(\n",
                // A _comps KEY may literally be `autoload`; the quoting must
                // keep it out of the name list.
                "'autoload' '_autoload'\n",
                "'zzt' '_zzt'\n",
                ")\n",
                "\n",
                "zle -C _complete_help complete-word _complete_help\n",
                "bindkey '^Xh' _complete_help\n",
                "\n",
                "autoload -Uz _zzt _zzt_two \\\n",
                "           __zzt_headerless _zzt_gone\n",
                "autoload -Uz +X _call_program\n",
                "typeset -gUa _comp_assocs\n",
            ),
        )
        .unwrap();

        let names = dump_autoload_names(&dump);
        assert_eq!(
            names,
            vec![
                "_zzt",
                "_zzt_two",
                "__zzt_headerless",
                "_zzt_gone",
                "_call_program",
            ],
            "continuation lines, `+X`/`-Uz` option words and the quoted \
             `_comps` key must all be handled"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Regression: on the sh:493-496 `-C` branch the dump is sourced and
    /// sh:501's `[[ -z "$_i_done" ]]` skips the `$fpath` scan, so the dump
    /// alone defines all five association tables. zshrs read them from its
    /// SQLite cache instead, and a partially-built cache (1849 `_comps`
    /// keys against the real dump's 51745 on a zpwr host) silently dropped
    /// `$_comps[zpwr]`, `$_comps[cargo]`, `$_comps[brew]`, … — every one of
    /// those commands then fell through `_dispatch` to `-default-` and
    /// completed FILES where zsh runs the registered completer.
    ///
    /// The value shapes asserted here are the ones compdump's `${(qq)}`
    /// actually emits (compdump:38-70): a plain `'k' 'v'` pair, a key that
    /// starts with a literal quote (`''\''brew'` → `'brew`), and a key with
    /// an embedded one (`'services'\'''` → `services'`).
    #[test]
    fn dump_assoc_tables_reads_all_five_compdump_tables() {
        let dir = std::env::temp_dir().join("zshrs_compinit_dumptables_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let dump = dir.join("zcompdump");
        fs::write(
            &dump,
            concat!(
                "#files: 3\tversion: 5.9.2\n",
                "\n",
                "_comps=(\n",
                "'zpwr' '_zpwr'\n",
                "''\\''brew' '_brew_services'\n",
                "'services'\\''' '_brew_services'\n",
                ")\n",
                "\n",
                "_services=(\n",
                "'ftp' 'ftp'\n",
                ")\n",
                "\n",
                "_patcomps=(\n",
                "'*/(init|rc[0-9S]#).d/*' '_init_d'\n",
                ")\n",
                "\n",
                "_postpatcomps=(\n",
                "'_*' '_compadd'\n",
                "'gcc-*' '_gcc'\n",
                ")\n",
                "\n",
                "_compautos=(\n",
                "'_call_program' '+X'\n",
                ")\n",
                "\n",
                // Everything after the tables must be ignored, including a
                // `)` that does not close one.
                "zle -C _complete_help complete-word _complete_help\n",
                "autoload -Uz _zzt\n",
                "typeset -gUa _comp_assocs\n",
                "_comp_assocs=( '' )\n",
            ),
        )
        .unwrap();

        let t = dump_assoc_tables(&dump).expect("dump is readable");
        assert_eq!(t.comps.get("zpwr").map(String::as_str), Some("_zpwr"));
        assert_eq!(
            t.comps.get("'brew").map(String::as_str),
            Some("_brew_services"),
            "`''\\''brew'` is three concatenated (qq) segments = `'brew`"
        );
        assert_eq!(
            t.comps.get("services'").map(String::as_str),
            Some("_brew_services")
        );
        assert_eq!(t.comps.len(), 3, "no stray keys from the trailing lines");
        assert_eq!(t.services.get("ftp").map(String::as_str), Some("ftp"));
        assert_eq!(
            t.patcomps.get("*/(init|rc[0-9S]#).d/*").map(String::as_str),
            Some("_init_d")
        );
        // Order is load-bearing: `_postpatcomps` is tried in insertion order,
        // and compdump writes it in `${(ok)}` order (compdump:61-66).
        assert_eq!(
            t.postpatcomps.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["_*", "gcc-*"]
        );
        assert_eq!(t.compautos.get("_call_program").map(String::as_str), Some("+X"));
        let _ = fs::remove_dir_all(&dir);
    }
}
