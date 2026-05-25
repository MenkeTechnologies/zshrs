//! Port of `_expand_alias` from `Completion/Base/Completer/_expand_alias`.
//!
//! Full upstream body (67 lines verbatim):
//! ```text
//! sh: 1  #compdef -K _expand_alias complete-word \C-xa
//! sh: 2
//! sh: 3  local word expl tmp pre sel what
//! sh: 4  local -a tmpa suf
//! sh: 5
//! sh: 6  eval "$_comp_setup"
//! sh: 7
//! sh: 8  if [[ -n $funcstack[2] ]]; then
//! sh: 9    if [[ "$funcstack[2]" = _prefix ]]; then
//! sh:10      word="$IPREFIX$PREFIX$SUFFIX"
//! sh:11    else
//! sh:12      word="$IPREFIX$PREFIX$SUFFIX$ISUFFIX"
//! sh:13    fi
//! sh:14    pre=()
//! sh:15  else
//! sh:16    local curcontext="$curcontext"
//! sh:17
//! sh:18    if [[ -z "$curcontext" ]]; then
//! sh:19      curcontext="expand-alias-word:::"
//! sh:20    else
//! sh:21      curcontext="expand-alias-word:${curcontext#*:}"
//! sh:22    fi
//! sh:23
//! sh:24    word="$IPREFIX$PREFIX$SUFFIX$ISUFFIX"
//! sh:25    pre=(_main_complete - aliases)
//! sh:26  fi
//! sh:27
//! sh:28  zstyle -s ":completion:${curcontext}:" regular tmp || tmp=yes
//! sh:29  case $tmp in
//! sh:30  always) sel=r;;
//! sh:31  yes|1|true|on) [[ CURRENT -eq 1 ]] && sel=r;;
//! sh:32  esac
//! sh:33  zstyle -T ":completion:${curcontext}:" global && sel="g$sel"
//! sh:34  zstyle -t ":completion:${curcontext}:" disabled && sel="${sel}${(U)sel}"
//! sh:35
//! sh:36  tmp=
//! sh:37  [[ $sel = *r* ]] && tmp=$aliases[$word]
//! sh:38  [[ -z $tmp && $sel = *g* ]] && tmp=$galiases[$word]
//! sh:39  [[ -z $tmp && $sel = *R* ]] && tmp=$dis_aliases[$word]
//! sh:40  [[ -z $tmp && $sel = *G* ]] && tmp=$dis_galiases[$word]
//! sh:41
//! sh:42  if [[ -n $tmp ]]; then
//! sh:43    # We used to remove the quoting from the value in the parameter.
//! sh:44    # That was probably just an oversight: an alias is always replaced
//! sh:45    # literally.
//! sh:46    tmp=${tmp%%[[:blank:]]##}
//! sh:47    if [[ $tmp[1] = [[:alnum:]_] ]]; then
//! sh:48      tmpa=(${(z)tmp})
//! sh:49      if [[ $tmpa[1] = $word && $tmp = $aliases[$word] ]]; then
//! sh:50        # This is an active regular alias and the first word in the result
//! sh:51        # is the same as what was on the line already.  Quote it so
//! sh:52        # that it doesn't get reexpanded on execution.
//! sh:53        #
//! sh:54        # Strictly we also need to check if the original word matches
//! sh:55        # a later word in the expansion and the previous words are
//! sh:56        # all aliases where the expansion ends in " ", but I'm
//! sh:57        # too lazy.
//! sh:58        tmp="\\$tmp"
//! sh:59      fi
//! sh:60    fi
//! sh:61    zstyle -T ":completion:${curcontext}:" add-space || suf=( -S '' )
//! sh:62    $pre _wanted aliases expl alias compadd -UQ "$suf[@]" -- ${tmp%%[[:blank:]]##}
//! sh:63  elif (( $#pre )) && zstyle -t ":completion:${curcontext}:" complete; then
//! sh:64    $pre _aliases -s "$sel" -S ''
//! sh:65  else
//! sh:66    return 1
//! sh:67  fi
//! ```
//!
//! Strict Rust port: four kinds of alias tables (regular / global /
//! disabled regular / disabled global) keyed by the assembled
//! `IPREFIX+PREFIX+SUFFIX[+ISUFFIX]` word. Selector is built per
//! shell:24-30, then the four tables are queried in order. If the
//! resolved expansion starts with the SAME word the user typed and
//! came from the regular alias table, prepend `\\` to prevent
//! re-expansion (shell:43-46).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
