//! Port of `_files` from `Completion/Unix/Type/_files`.
//!
//! Full upstream body (153 lines verbatim):
//! ```text
//! sh:  1  #compdef -redirect-,-default-,-default-
//! sh:  2
//! sh:  3  local -a match mbegin mend
//! sh:  4  local -a subtree
//! sh:  5  local ret=1
//! sh:  6
//! sh:  7  # Look for glob qualifiers. This is duplicated from _path_files because
//! sh:  8  # we don't want to complete them multiple times (for each file pattern).
//! sh:  9  if _have_glob_qual $PREFIX; then
//! sh: 10    compset -p ${#match[1]}
//! sh: 11    compset -S '[^\)\|\~]#(|\))'
//! sh: 12    if [[ $_comp_caller_options[extendedglob] == on ]] && compset -P '\#'; then
//! sh: 13      _globflags && ret=0
//! sh: 14    else
//! sh: 15      if [[ $_comp_caller_options[extendedglob] == on ]]; then
//! sh: 16        _describe -t globflags "glob flag" '(\#:introduce\ glob\ flag)' -Q -S '' && ret=0
//! sh: 17      fi
//! sh: 18      _globquals && ret=0
//! sh: 19    fi
//! sh: 20    return ret
//! sh: 21  elif [[ $_comp_caller_options[extendedglob] == on && $PREFIX = \(\#[^\)]# ]] && compset -P '\(\#'; then
//! sh: 22    # Globbing flags can start at beginning of word, even though
//! sh: 23    # glob qualifiers can't.
//! sh: 24    _globflags && return
//! sh: 25  fi
//! sh: 26
//! sh: 27  local opts tmp glob pat pats expl tag i def descr end ign tried
//! sh: 28  local type sdef ignvars ignvar prepath oprefix rfiles rfile
//! sh: 29
//! sh: 30  zparseopts -a opts \
//! sh: 31      '/=tmp' 'f=tmp' 'g+:-=tmp' q n 1 2 P: S: r: R: W: x+: X+: M+: F: J+: V+: o+:
//! sh: 32
//! sh: 33  type="${(@j::M)${(@)tmp#-}#?}"
//! sh: 34  if (( $tmp[(I)-g*] )); then
//! sh: 35    glob="${${${(@)${(@M)tmp:#-g*}#-g}##[[:blank:]]#}%%[[:blank:]]#}"
//! sh: 36    [[ "$glob" = *[^\\][[:blank:]]* ]] &&
//! sh: 37        glob="{${glob//(#b)([^\\])[[:blank:]]##/${match[1]},}}"
//! sh: 38
//! sh: 39    # add `#q' to the beginning of any glob qualifier if not there already
//! sh: 40    [[ "$glob" = (#b)(*\()([^\|\~]##\)) && $match[2] != \#q* ]] &&
//! sh: 41        glob="${match[1]}#q${match[2]}"
//! sh: 42  elif [[ $type = */* ]]; then
//! sh: 43    glob="*(#q-/)"
//! sh: 44  fi
//! sh: 45  tmp=$opts[(I)-F]
//! sh: 46  if (( tmp )); then
//! sh: 47    ignvars=($=opts[tmp+1])
//! sh: 48    if [[ $ignvars = _comp_ignore ]]; then
//! sh: 49      ign=( $_comp_ignore )
//! sh: 50    elif [[ $ignvars = \(* ]]; then
//! sh: 51      ign=( ${=ignvars[2,-2]} )
//! sh: 52    else
//! sh: 53      ign=()
//! sh: 54      for ignvar in $ignvars; do
//! sh: 55        ign+=(${(P)ignvar})
//! sh: 56      done
//! sh: 57      opts[tmp+1]=_comp_ignore
//! sh: 58    fi
//! sh: 59  else
//! sh: 60    ign=()
//! sh: 61  fi
//! sh: 62
//! sh: 63  if zstyle -a ":completion:${curcontext}:" file-patterns tmp; then
//! sh: 64    pats=()
//! sh: 65
//! sh: 66    for i in ${tmp//\%p/${${glob:-\*}//:/\\:}}; do
//! sh: 67      if [[ $i = *[^\\]:* ]]; then
//! sh: 68        pats+=( " $i " )
//! sh: 69      else
//! sh: 70        pats+=( " ${i}:files " )
//! sh: 71      fi
//! sh: 72    done
//! sh: 73  elif zstyle -t ":completion:${curcontext}:" list-dirs-first; then
//! sh: 74    pats=( " *(-/):directories:directory ${${glob:-*}//:/\\:}(#q^-/):globbed-files" '*:all-files' )
//! sh: 75  else
//! sh: 76    # People prefer to have directories shown on first try as default.
//! sh: 77    # Even if the calling function didn't use -/.
//! sh: 78    pats=( "${${glob:-*}//:/\\:}:globbed-files *(-/):directories" '*:all-files ' )
//! sh: 79  fi
//! sh: 80
//! sh: 81  tried=()
//! sh: 82  for def in "$pats[@]"; do
//! sh: 83    eval "def=( ${${def//\\:/\\\\\\:}//(#b)([][()|*?^#~<>])/\\${match[1]}} )"
//! sh: 84
//! sh: 85    tmp="${(@M)def#*[^\\]:}"
//! sh: 86    (( $tried[(I)${(q)tmp}] )) && continue
//! sh: 87    tried=( "$tried[@]" "$tmp" )
//! sh: 88
//! sh: 89    for sdef in "$def[@]"; do
//! sh: 90
//! sh: 91      tag="${${sdef#*[^\\]:}%%:*}"
//! sh: 92      pat="${${sdef%%:${tag}*}//\\:/:}"
//! sh: 93
//! sh: 94      if [[ "$sdef" = *:${tag}:* ]]; then
//! sh: 95        # If the file-patterns spec includes a description, use it and give the
//! sh: 96        # group/description options from it precedence over passed in parameters.
//! sh: 97        descr="${(Q)sdef#*:${tag}:}"
//! sh: 98        end=
//! sh: 99      else
//! sh:100        if (( $opts[(I)-X] )); then
//! sh:101          descr=
//! sh:102        else
//! sh:103          descr=file
//! sh:104        fi
//! sh:105        end=yes
//! sh:106      fi
//! sh:107
//! sh:108      _tags "$tag"
//! sh:109      while _tags; do
//! sh:110        _comp_ignore=()
//! sh:111        while _next_label "$tag" expl "$descr"; do
//! sh:112          _comp_ignore=( $_comp_ignore $ign )
//! sh:113          if [[ -n "$end" ]]; then
//! sh:114            expl=( "$opts[@]" "$expl[@]" )
//! sh:115          else
//! sh:116            expl+=( "$opts[@]" )
//! sh:117          fi
//! sh:118
//! sh:119          if _path_files -g "$pat" "$expl[@]"; then
//! sh:120            ret=0
//! sh:121          elif [[ $PREFIX$SUFFIX != */* ]] && \
//! sh:122              zstyle -a ":completion:${curcontext}:$tag" recursive-files rfiles
//! sh:123          then
//! sh:124            for rfile in $rfiles; do
//! sh:125              if [[ $PWD/ = ${~rfile} ]]; then
//! sh:126                if [[ -z $subtree ]]; then
//! sh:127                  subtree=( **/*(/) )
//! sh:128                fi
//! sh:129                for prepath in $subtree; do
//! sh:130                  oprefix=$PREFIX
//! sh:131                  PREFIX=$prepath/$PREFIX
//! sh:132                  _path_files -g "$pat" "$expl[@]" && ret=0
//! sh:133                  PREFIX=$oprefix
//! sh:134                done
//! sh:135                break
//! sh:136              fi
//! sh:137            done
//! sh:138          fi
//! sh:139        done
//! sh:140        (( ret )) || break
//! sh:141      done
//! sh:142
//! sh:143      ### For that _next_tags change mentioned above we would have to
//! sh:144      ### comment out the following line. (Or not, depending on the order
//! sh:145      ### of the patterns.)
//! sh:146
//! sh:147      [[ "$pat" = '*' ]] && return ret
//! sh:148
//! sh:149    done
//! sh:150    (( ret )) || return 0
//! sh:151  done
//! sh:152
//! sh:153  return 1
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
