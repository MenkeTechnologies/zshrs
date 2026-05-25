//! Port of `_shadow` from `Completion/Base/Utility/_shadow`.
//!
//! Full upstream body (97 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  ## Recommended usage:
//! sh: 4  #  {
//! sh: 5  #    _shadow fname
//! sh: 6  #    function fname {
//! sh: 7  #      # Do your new thing
//! sh: 8  #    }
//! sh: 9  #    # Invoke callers of fname
//! sh:10  #  } always {
//! sh:11  #    _unshadow
//! sh:12  #  }
//! sh:13  ## Alternate usage:
//! sh:14  # {
//! sh:15  #   _shadow -s suffix fname
//! sh:16  #   function fname {
//! sh:17  #     # Do other stuff
//! sh:18  #     fname@suffix new args for fname
//! sh:19  #   }
//! sh:20  #   # Invoke callers of fname
//! sh:21  # } always {
//! sh:22  #   _unshadow
//! sh:23  # }
//! sh:24  ##
//! sh:25
//! sh:26  # BUGS:
//! sh:27  # * `functions -c` acts like `autoload +X`
//! sh:28  # * name collisions are possible in alternate usage
//! sh:29  # * functions that examine $0 probably misfire
//! sh:30
//! sh:31  zmodload zsh/parameter # Or what?
//! sh:32
//! sh:33  # This probably never comes up, but protect ourself from recursive call
//! sh:34  # chains that may duplicate the top elements of $funcstack by creating
//! sh:35  # a counter of _shadow calls and using it to make shadow names unique.
//! sh:36  builtin typeset -gHi .shadow.depth=0
//! sh:37  builtin typeset -gHa .shadow.stack
//! sh:38
//! sh:39  # Create a copy of each fname so that a caller may redefine
//! sh:40  _shadow() {
//! sh:41    emulate -L zsh
//! sh:42    local -A fsfx=( -s ${funcstack[2]}:${functrace[2]}:$((.shadow.depth+1)) )
//! sh:43    local fname shadowname
//! sh:44    local -a fnames
//! sh:45    zparseopts -K -A fsfx -D s:
//! sh:46    for fname; do
//! sh:47      shadowname=${fname}@${fsfx[-s]}
//! sh:48      if (( ${+functions[$shadowname]} ))
//! sh:49      then
//! sh:50        # Called again with the same -s, just ignore it
//! sh:51        continue
//! sh:52      elif (( ${+functions[$fname]} ))
//! sh:53      then
//! sh:54        builtin functions -c -- $fname $shadowname
//! sh:55        fnames+=(f@$fname)
//! sh:56      elif (( ${+builtins[$fname]} ))
//! sh:57      then
//! sh:58        eval "function -- ${(q-)shadowname} { builtin ${(q-)fname} \"\$@\" }"
//! sh:59        fnames+=(b@$fname)
//! sh:60      else
//! sh:61        eval "function -- ${(q-)shadowname} { command ${(q-)fname} \"\$@\" }"
//! sh:62        fnames+=(c@$fname)
//! sh:63      fi
//! sh:64    done
//! sh:65    [[ -z $REPLY ]] && REPLY=${fsfx[-s]}
//! sh:66    builtin set -A .shadow.stack ${fsfx[-s]} $fnames -- ${.shadow.stack}
//! sh:67    ((.shadow.depth++))
//! sh:68  }
//! sh:69
//! sh:70  # Remove the redefined function and shadowing name
//! sh:71  _unshadow() {
//! sh:72    emulate -L zsh
//! sh:73    local fname shadowname fsfx=${.shadow.stack[1]}
//! sh:74    local -a fnames
//! sh:75    [[ -n $fsfx ]] || return 1
//! sh:76    shift .shadow.stack
//! sh:77    while [[ ${.shadow.stack[1]?no shadows} != -- ]]; do
//! sh:78      fname=${.shadow.stack[1]#?@}
//! sh:79      shadowname=${fname}@${fsfx}
//! sh:80      if (( ${+functions[$fname]} )); then
//! sh:81        builtin unfunction -- $fname
//! sh:82      fi
//! sh:83      case ${.shadow.stack[1]} in
//! sh:84        (f@*) builtin functions -c -- $shadowname $fname ;&
//! sh:85        ([bc]@*) builtin unfunction -- $shadowname ;;
//! sh:86      esac
//! sh:87      shift .shadow.stack
//! sh:88    done
//! sh:89    [[ -z $REPLY ]] && REPLY=$fsfx
//! sh:90    shift .shadow.stack
//! sh:91    ((.shadow.depth--))
//! sh:92  }
//! sh:93
//! sh:94  # This is tricky.  When we call _shadow recursively from autoload,
//! sh:95  # there's an extra level of stack in $functrace that will confuse
//! sh:96  # the later call to _unshadow.  Fool ourself into working correctly.
//! sh:97  (( ARGC )) && _shadow -s ${funcstack[2]}:${functrace[2]}:1 "$@"
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
