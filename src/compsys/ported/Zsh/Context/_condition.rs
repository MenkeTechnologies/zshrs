//! Port of `_condition` from `Completion/Zsh/Context/_condition`.
//!
//! Full upstream body (60 lines verbatim):
//! ```text
//! sh: 1  #compdef -condition-
//! sh: 2
//! sh: 3  local prev="$words[CURRENT-1]" ret=1
//! sh: 4
//! sh: 5  if [[ "$prev" = -o ]]; then
//! sh: 6    _tags -C -o options && _options
//! sh: 7  elif [[ "$prev" = -([a-hkprsuwxLOGSN]|[no]t|ef) ]]; then
//! sh: 8    _tags -C "$prev" files && _files
//! sh: 9  elif [[ "$prev" = -t ]]; then
//! sh:10    _file_descriptors
//! sh:11  elif [[ "$prev" = -v ]]; then
//! sh:12    _parameters -r "\= \t\n\[\-"
//! sh:13  else
//! sh:14    if [[ "$PREFIX" = -* ]] ||
//! sh:15       ! zstyle -T ":completion:${curcontext}:options" prefix-needed; then
//! sh:16
//! sh:17      if [[ "$prev" = (\[\[|\|\||\&\&|\!|\() ]]; then
//! sh:18        _describe -o 'condition code' \
//! sh:19                  '( -a:existing\ file
//! sh:20  	           -b:block\ special\ file
//! sh:21  	           -c:character\ special\ file
//! sh:22  	           -d:directory
//! sh:23  	           -e:existing\ file
//! sh:24  	           -f:regular\ file
//! sh:25  	           -g:setgid\ bit
//! sh:26  	           -h:symbolic\ link
//! sh:27  	           -k:sticky\ bit
//! sh:28  	           -n:non-empty\ string
//! sh:29  	           -o:option
//! sh:30  	           -p:named\ pipe
//! sh:31  	           -r:readable\ file
//! sh:32  	           -s:non-empty\ file
//! sh:33  	           -t:terminal\ file\ descriptor
//! sh:34  	           -u:setuid\ bit
//! sh:35  		   -v:set\ variable
//! sh:36  	           -w:writable\ file
//! sh:37  	           -x:executable\ file
//! sh:38  	           -z:empty\ string
//! sh:39  	           -L:symbolic\ link
//! sh:40  	           -O:own\ file
//! sh:41  	           -G:group-owned\ file
//! sh:42  	           -S:socket
//! sh:43  	           -N:unread\ file)' && ret=0
//! sh:44      else
//! sh:45        _describe -o 'condition code' \
//! sh:46  	        '( -nt:newer\ than
//! sh:47  	           -ot:older\ than
//! sh:48  	           -ef:same\ file
//! sh:49  	           -eq:numerically\ equal
//! sh:50  	           -ne:numerically\ not\ equal
//! sh:51  	           -lt:numerically\ less\ than
//! sh:52  	           -le:numerically\ less\ than\ or\ equal
//! sh:53  	           -gt:numerically\ greater\ than
//! sh:54  	           -ge:numerically\ greater\ than\ or\ equal)' && ret=0
//! sh:55      fi
//! sh:56    fi
//! sh:57    _alternative 'files:: _files' 'parameters:: _parameters' && ret=0
//! sh:58
//! sh:59    return ret
//! sh:60  fi
//! ```
//!
//! Strict Rust port: faithful dispatch based on `prev` (the
//! previous word on the line). Operators get their right-hand
//! side completed per the upstream branch table. Caller supplies
//! file/options/parameter handlers since `_options` /
//! `_file_descriptors` / `_parameters` need data injection.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
