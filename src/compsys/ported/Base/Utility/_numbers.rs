//! Port of `_numbers` from `Completion/Base/Utility/_numbers`.
//!
//! Full upstream body (87 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Usage: _numbers [compadd options] [-t tag] [-f|-N] [-u units] [-l min] [-m max] \
//! sh: 4  #                 [-d default] ["description"] [unit-suffix...]
//! sh: 5
//! sh: 6  #   -t : specify a tag (defaults to 'numbers')
//! sh: 7  #   -u : indicate the units, e.g. seconds
//! sh: 8  #   -l : lowest possible value
//! sh: 9  #   -m : maximum possible value
//! sh:10  #   -d : default value
//! sh:11  #   -N : allow negative numbers (implied by range including a negative)
//! sh:12  #   -f : allow decimals (float)
//! sh:13
//! sh:14  # For a unit-suffix, an initial colon indicates a unit that asserts the default
//! sh:15  # otherwise, colons allow for descriptions, e.g:
//! sh:16
//! sh:17  #   :s:seconds m:minutes h:hours
//! sh:18
//! sh:19  # unit-suffixes are not sorted by the completion system when listed
//! sh:20  # Specify them in order of magnitude, this tends to be ascending unless
//! sh:21  # the default is of a higher magnitude, in which case, descending.
//! sh:22  # So for, example
//! sh:23  #   bytes kB MB GB
//! sh:24  #   s ms us ns
//! sh:25  # Where the compadd options include matching control or suffixes, these
//! sh:26  # are applied to the units
//! sh:27
//! sh:28  # For each unit-suffix, the format style is looked up with the
//! sh:29  # unit-suffixes tag and the results concatenated. Specs used are:
//! sh:30  #   x : the suffix
//! sh:31  #   X : suffix description
//! sh:32  #   d : indicate suffix is for the default unit
//! sh:33  #   i : list index
//! sh:34  #   r : reverse list index
//! sh:35  # The latter three of these are useful with ternary expressions.
//! sh:36
//! sh:37  # _description is called with the x token set to make the completed
//! sh:38  # list of suffixes available to the normal format style
//! sh:39
//! sh:40  local desc tag range suffixes suffix suffixfmt pat='<->' partial=''
//! sh:41  local -a expl formats
//! sh:42  local -a default max min keep tags units
//! sh:43  local -i i
//! sh:44  local -A opts
//! sh:45
//! sh:46  zparseopts -K -D -A opts M+:=keep q:=keep s+:=keep S+:=keep J+: V+: 1 2 o+: n F: x+: X+: \
//! sh:47    t:=tags u:=units l:=min m:=max d:=default f=type e=type N=type
//! sh:48
//! sh:49  desc="${1:-number}" tag="${tags[2]:-numbers}"
//! sh:50  (( $# )) && shift
//! sh:51
//! sh:52  [[ -n ${(M)type:#-f} ]] && pat='(<->.[0-9]#|[0-9]#.<->|<->)' partial='(|.)'
//! sh:53  [[ -n ${(M)type:#-N} || $min[2] = -* || $max[2] = -* ]] && \
//! sh:54      pat="(|-)$pat" partial="(|-)$partial"
//! sh:55
//! sh:56  if (( $#argv )) && compset -P "$pat"; then
//! sh:57    zstyle -s ":completion:${curcontext}:units" list-separator sep || sep=--
//! sh:58    _description -V units expl unit
//! sh:59    disp=( ${${argv#:}/:/ $sep } )
//! sh:60    compadd -M 'r:|/=* r:|=*' -d disp "$keep[@]" "$expl[@]" - ${${argv#:}%%:*}
//! sh:61    return
//! sh:62  elif [[ -prefix $~pat || $PREFIX = $~partial ]]; then
//! sh:63    formats=( "h:$desc" )
//! sh:64    (( $#units )) && formats+=( m:${units[2]} ) desc+=" ($units[2])"
//! sh:65    (( $#min )) && range="$min[2]-"
//! sh:66    (( $#max )) && range="${range:--}$max[2]"
//! sh:67    [[ -n $range ]] && formats+=( r:$range ) desc+=" ($range)"
//! sh:68    (( $#default )) && formats+=( o:${default[2]} ) desc+=" [$default[2]]"
//! sh:69
//! sh:70    zstyle -s ":completion:${curcontext}:unit-suffixes" format suffixfmt || \
//! sh:71        suffixfmt='%(d.%U.)%x%(d.%u.)%(r..|)'
//! sh:72    for ((i=0;i<$#;i++)); do
//! sh:73      zformat -f suffix "$suffixfmt" "x:${${argv[i+1]#:}%%:*}" \
//! sh:74          "X:${${argv[i+1]#:}#*:}" "d:${#${argv[i+1]}[1]#:}" \
//! sh:75  	i:i r:$(( $# - i - 1))
//! sh:76      suffixes+="${suffix//\%/%%}"
//! sh:77    done
//! sh:78    [[ -n $suffixes ]] && formats+=( x:$suffixes )
//! sh:79
//! sh:80    _comp_mesg=yes
//! sh:81    _description -x $tag expl "$desc" $formats
//! sh:82    [[ $compstate[insert] = *unambiguous* ]] && compstate[insert]=
//! sh:83    compadd "$expl[@]"
//! sh:84    return 0
//! sh:85  fi
//! sh:86
//! sh:87  return 1
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
