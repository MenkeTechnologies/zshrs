//! Port of `_user_expand` from `Completion/Base/Completer/_user_expand`.
//!
//! Full upstream body (147 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # This completer function is an addition to the _expand completer that
//! sh:  4  # allows the user to define their own expansions.  It does not replace
//! sh:  5  # the _expand completer.
//! sh:  6  #
//! sh:  7  # This function will allow other completer functions to be called if
//! sh:  8  # the expansions done produce no result or do not change the original
//! sh:  9  # word from the line.
//! sh: 10
//! sh: 11  setopt localoptions nonomatch
//! sh: 12
//! sh: 13  [[ _matcher_num -gt 1 ]] && return 1
//! sh: 14
//! sh: 15  local exp word sort expr expl subd suf=" " asp tmp spec REPLY
//! sh: 16  local -a specs reply
//! sh: 17
//! sh: 18  if [[ "$funcstack[2]" = _prefix ]]; then
//! sh: 19    word="$IPREFIX$PREFIX$SUFFIX"
//! sh: 20  else
//! sh: 21    word="$IPREFIX$PREFIX$SUFFIX$ISUFFIX"
//! sh: 22  fi
//! sh: 23
//! sh: 24  # In exp we will collect the expansions.
//! sh: 25
//! sh: 26  exp=("$word")
//! sh: 27
//! sh: 28  # Now look for user completions.
//! sh: 29
//! sh: 30  zstyle -a ":completion:${curcontext}:" user-expand specs || return 1
//! sh: 31
//! sh: 32  for spec in $specs; do
//! sh: 33    REPLY=
//! sh: 34    case $spec in
//! sh: 35      ('$'[[:IDENT:]]##)
//! sh: 36      # Spec is an associative array with explicit keys.
//! sh: 37      # Surely there's a better way of doing an associative array
//! sh: 38      # lookup from its name?
//! sh: 39      eval tmp='${'$spec[2,-1]'[$word]}'
//! sh: 40      if [[ -n $tmp ]]; then
//! sh: 41        exp=("$tmp")
//! sh: 42        break
//! sh: 43      fi
//! sh: 44      ;;
//! sh: 45
//! sh: 46      ('_'*)
//! sh: 47      reply=()
//! sh: 48      $spec $word
//! sh: 49      if (( ${#reply} )); then
//! sh: 50        exp=("${reply[@]}")
//! sh: 51        break
//! sh: 52      fi
//! sh: 53      ;;
//! sh: 54    esac
//! sh: 55  done
//! sh: 56
//! sh: 57  [[ $#exp -eq 1 && "$exp[1]" = "$word" ]] && return 1
//! sh: 58
//! sh: 59  # Now add as matches whatever the user requested.
//! sh: 60
//! sh: 61  zstyle -s ":completion:${curcontext}:" sort sort
//! sh: 62
//! sh: 63  [[ "$sort" = (yes|true|1|on) ]] && exp=( "${(@o)exp}" )
//! sh: 64
//! sh: 65  if zstyle -s ":completion:${curcontext}:" add-space tmp; then
//! sh: 66    if [[ "$tmp" != *subst* || "$word" != *\$* || "$exp[1]" = *\$* ]]; then
//! sh: 67      [[ "$tmp" = *file* ]] && asp=file
//! sh: 68      [[ "$tmp" = *(yes|true|1|on|subst)* ]] && asp="yes$asp"
//! sh: 69    fi
//! sh: 70  else
//! sh: 71    asp=file
//! sh: 72  fi
//! sh: 73
//! sh: 74  # If there is only one expansion, add a suitable suffix
//! sh: 75
//! sh: 76  if (( $#exp == 1 )); then
//! sh: 77    if [[ -d ${exp[1]} && "$exp[1]" != */ ]]; then
//! sh: 78      suf=/
//! sh: 79    elif [[ "$asp" = yes* ||
//! sh: 80            ( "$asp" = *file && -f "${exp[1]}" ) ]]; then
//! sh: 81      suf=' '
//! sh: 82    else
//! sh: 83      suf=
//! sh: 84    fi
//! sh: 85  fi
//! sh: 86
//! sh: 87  if [[ -z "$compstate[insert]" ]] ;then
//! sh: 88    if [[ "$sort" = menu ]]; then
//! sh: 89      _description expansions expl "expansions${REPLY:+: $REPLY}" "o:$word"
//! sh: 90    else
//! sh: 91      _description -V expansions expl "expansions${REPLY:+: $REPLY}" "o:$word"
//! sh: 92    fi
//! sh: 93
//! sh: 94    compadd "$expl[@]" -UQ -qS "$suf" -a exp
//! sh: 95  else
//! sh: 96    _tags all-expansions expansions original
//! sh: 97
//! sh: 98    if [[ $#exp -ge 1 ]] && _requested expansions; then
//! sh: 99      local i j normal space dir
//! sh:100
//! sh:101      if [[ "$sort" = menu ]]; then
//! sh:102        _description expansions expl "expansions${REPLY:+: $REPLY}" "o:$word"
//! sh:103      else
//! sh:104        _description -V expansions expl "expansions${REPLY:+: $REPLY}" "o:$word"
//! sh:105      fi
//! sh:106      normal=()
//! sh:107      space=()
//! sh:108      dir=()
//! sh:109
//! sh:110      for i in "$exp[@]"; do
//! sh:111        j="${i}"
//! sh:112        if [[ -d "$j" && "$i" != */ ]]; then
//! sh:113          dir=( "$dir[@]" "$i" )
//! sh:114        elif [[ "$asp" = yes* || ( "$asp" = *file && -f "$j" ) ]]; then
//! sh:115          space=( "$space[@]" "$i" )
//! sh:116        else
//! sh:117  	normal=( "$normal[@]" "$i" )
//! sh:118        fi
//! sh:119      done
//! sh:120      (( $#dir ))    && compadd "$expl[@]" -UQ -qS/ -a dir
//! sh:121      (( $#space ))  && compadd "$expl[@]" -UQ -qS " " -a space
//! sh:122      (( $#normal )) && compadd "$expl[@]" -UQ -qS "" -a normal
//! sh:123    fi
//! sh:124    if _requested all-expansions; then
//! sh:125      local disp dstr
//! sh:126
//! sh:127      if [[ "$sort" = menu ]]; then
//! sh:128        _description all-expansions expl "all expansions${REPLY:+: $REPLY}" "o:$word"
//! sh:129      else
//! sh:130        _description -V all-expansions expl "all expansions${REPLY:+: $REPLY}" "o:$word"
//! sh:131      fi
//! sh:132      if [[ "${#${exp}}" -ge COLUMNS ]]; then
//! sh:133        disp=( -ld dstr )
//! sh:134        dstr=( "${(r:COLUMNS-5:)exp} ..." )
//! sh:135      else
//! sh:136        disp=()
//! sh:137      fi
//! sh:138      [[ -o multios ]] && exp=($exp[1] $compstate[redirect]${^exp[2,-1]})
//! sh:139      compadd "$disp[@]" "$expl[@]" -UQ -qS "$suf" - "$exp"
//! sh:140    fi
//! sh:141
//! sh:142    _requested original expl original && compadd "$expl[@]" -UQ - "$word"
//! sh:143
//! sh:144    compstate[insert]=menu
//! sh:145  fi
//! sh:146
//! sh:147  return 0
//! ```
//!
//! Simplified Rust port: takes the user-expand pattern→expansion
//! map directly instead of looking it up via zstyle (upstream's
//! `user-expand` style holds shell-fn names that get eval'd; our
//! HashMap is the ready-to-use form). For each pattern that is a
//! prefix of PREFIX, emit the rewritten string via
//! `replacen(pattern, expansion, 1)`.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
