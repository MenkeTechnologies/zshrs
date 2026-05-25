//! Port of `_dispatch` from `Completion/Base/Core/_dispatch`.
//!
//! Full upstream body (91 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local comp pat val name i ret=1 _compskip="$_compskip"
//! sh: 4  local curcontext="$curcontext" service str noskip
//! sh: 5  local -a match mbegin mend
//! sh: 6
//! sh: 7  # If we get the option `-s', we don't reset `_compskip'.
//! sh: 8
//! sh: 9  if [[ "$1" = -s ]]; then
//! sh:10    noskip=yes
//! sh:11    shift
//! sh:12  fi
//! sh:13
//! sh:14  [[ -z "$noskip" ]] && _compskip=
//! sh:15
//! sh:16  curcontext="${curcontext%:*:*}:${1}:"
//! sh:17
//! sh:18  shift
//! sh:19
//! sh:20  # See if there are any matching pattern completions.
//! sh:21
//! sh:22  if [[ "$_compskip" != (all|*patterns*) ]]; then
//! sh:23
//! sh:24    for str in "$@"; do
//! sh:25      [[ -n "$str" ]] || continue
//! sh:26      service="${_services[$str]:-$str}"
//! sh:27      for i in "${(@)_patcomps[(K)$str]}"; do
//! sh:28        if [[ $i = (#b)"="([^=]#)"="(*) ]]; then
//! sh:29  	service=$match[1]
//! sh:30  	i=$match[2]
//! sh:31        fi
//! sh:32        eval "$i" && ret=0
//! sh:33        if [[ "$_compskip" = *patterns* ]]; then
//! sh:34          break
//! sh:35        elif [[ "$_compskip" = all ]]; then
//! sh:36          _compskip=''
//! sh:37          return ret
//! sh:38        fi
//! sh:39      done
//! sh:40    done
//! sh:41  fi
//! sh:42
//! sh:43  # Now look up the names in the normal completion array.
//! sh:44
//! sh:45  ret=1
//! sh:46  for str in "$@"; do
//! sh:47    [[ -n "$str" ]] || continue
//! sh:48    # The following means we look up the names of commands
//! sh:49    # after stripping quotes.  This is presumably correct,
//! sh:50    # but do we need to do the same elsewhere?
//! sh:51    str=${(Q)str}
//! sh:52    name="$str"
//! sh:53    comp="${_comps[$str]}"
//! sh:54    service="${_services[$str]:-$str}"
//! sh:55
//! sh:56    [[ -z "$comp" ]] || break
//! sh:57  done
//! sh:58
//! sh:59  # And generate the matches, probably using default completion.
//! sh:60
//! sh:61  if [[ -n "$comp" && "$name" != "${argv[-1]}" ]]; then
//! sh:62    _compskip=patterns
//! sh:63    eval "$comp" && ret=0
//! sh:64    [[ "$_compskip" = (all|*patterns*) ]] && return ret
//! sh:65  fi
//! sh:66
//! sh:67  if [[ "$_compskip" != (all|*patterns*) ]]; then
//! sh:68    for str; do
//! sh:69      [[ -n "$str" ]] || continue
//! sh:70      service="${_services[$str]:-$str}"
//! sh:71      for i in "${(@)_postpatcomps[(K)$str]}"; do
//! sh:72        _compskip=default
//! sh:73        eval "$i" && ret=0
//! sh:74        if [[ "$_compskip" = *patterns* ]]; then
//! sh:75          break
//! sh:76        elif [[ "$_compskip" = all ]]; then
//! sh:77          _compskip=''
//! sh:78          return ret
//! sh:79        fi
//! sh:80      done
//! sh:81    done
//! sh:82  fi
//! sh:83
//! sh:84  [[ "$name" = "${argv[-1]}" && -n "$comp" &&
//! sh:85     "$_compskip" != (all|*default*) ]] &&
//! sh:86    service="${_services[$name]:-$name}" &&
//! sh:87     eval "$comp" && ret=0
//! sh:88
//! sh:89  _compskip=''
//! sh:90
//! sh:91  return ret
//! ```
//!
//! Faithful Rust port:
//! - Honors `compskip` flags ("all", "patterns", "default") via
//! module-level functions.
//! - shell:16 — curcontext rewrite: strip last two `:`-segments,
//! append `:$1:`.
//! - shell:26 — `_services[$str]:-$str` service-aliasing.
//! - shell:27-32 — pattern completions via `_patcomps` walk.
//! - shell:39-41 — `_comps[$service]` lookup + invoke via the
//! `_call_function` registry.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
