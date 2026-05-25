//! Port of `_sequence` from `Completion/Base/Utility/_sequence`.
//!
//! Full upstream body (40 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # a separated list where each component of the list uses the same
//! sh: 4  # function.
//! sh: 5
//! sh: 6  # -n num : number of items in list [default is unlimited]
//! sh: 7  # -s sep : specify separator [defaults to comma]
//! sh: 8  # -d     : duplicate values allowed
//! sh: 9
//! sh:10  local curcontext="$curcontext" nm="$compstate[nmatches]" pre qsep nosep minus
//! sh:11  local -a opts sep num pref suf cont end uniq dedup garbage
//! sh:12
//! sh:13  zparseopts -D -a opts s:=sep n:=num p:=pref i:=pref P:=pref I:=suf S:=suf \
//! sh:14      q=suf r:=suf R:=suf C:=cont F:=garbage d=uniq M+: J+: V+: 1 2 o+: X+: x+:
//! sh:15  (( $#cont )) && curcontext="${curcontext%:*}:$cont[2]"
//! sh:16  (( $#sep )) || sep[2]=,
//! sh:17
//! sh:18  if (( $+suf[(r)-S] )); then
//! sh:19    end="${(q)suf[suf[(i)-S]+1]}"
//! sh:20    (( $#end )) && compset -S ${end}\* && suf=() && nosep=1
//! sh:21  fi
//! sh:22
//! sh:23  qsep="${sep[2]}"
//! sh:24  compquote -p qsep
//! sh:25  if (( ! $#uniq )); then
//! sh:26    (( $+pref[(r)-P] )) && pre="${(q)pref[pref[(i)-P]+1]}"
//! sh:27    dedup=( "${(@)${(@ps.$qsep.)PREFIX#$pre}[1,-2]}" "${(@)${(@ps.$qsep.)SUFFIX}[2,-1]}" )
//! sh:28    [[ -n $compstate[quoting] ]] || dedup=( ${(Q)dedup} )
//! sh:29  fi
//! sh:30
//! sh:31  if (( $#num )) && compset -P $(( num[2] - 1 )) \*${(q)qsep}; then
//! sh:32    pref=()
//! sh:33  else
//! sh:34    (( ! nosep && (!$#num || num[2] > 1) )) && suf=( -S ${qsep} -r "$end[1]${(q)qsep[1]} \t\n\-" )
//! sh:35    compset -S ${(q)qsep}\* && suf=()
//! sh:36    compset -P \*${(q)qsep} && pref=()
//! sh:37  fi
//! sh:38
//! sh:39  (( minus = argv[(ib:2:)-] ))
//! sh:40  "${(@)argv[1,minus-1]}" "$opts[@]" -F dedup "$pref[@]" "$suf[@]" "${(@)argv[minus+1,-1]}"
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
