//! Port of `_file_descriptors` from `Completion/Zsh/Type/_file_descriptors`.
//!
//! Full upstream body (59 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local i fds expl disp link sep
//! sh: 4  local -a list proc
//! sh: 5
//! sh: 6  fds=( /dev/fd/<3->(N:t) )
//! sh: 7  fds=( ${(n)fds} )
//! sh: 8
//! sh: 9  if zstyle -T ":completion:${curcontext}:file-descriptors" verbose; then
//! sh:10    zstyle -s ":completion:${curcontext}:file-descriptors" list-separator sep || sep=--
//! sh:11
//! sh:12    if [[ $OSTYPE = freebsd* ]]; then
//! sh:13      fds=( ${(f)"$(procstat -f $$|awk -v OFS=: '$3>2 && $3~/[0-9]/ {print $3,$10}')"} )
//! sh:14      zformat -a list " $sep " $fds
//! sh:15      fds=( ${fds%%:*} )
//! sh:16    elif
//! sh:17      proc=( /proc/$$/(fd|path)/<->(@N[-1]:h) )
//! sh:18      [[ -n $proc ]]
//! sh:19    then
//! sh:20      if zmodload -F zsh/stat b:zstat; then
//! sh:21        for i in "${fds[@]}"; do
//! sh:22  	if zstat +link -A link $proc/$i; then
//! sh:23  	  list+=( "${(r.$#fds[-1].)i} $sep ${(D)link[1]}" )
//! sh:24  	else
//! sh:25  	  fds[(i)$i]=()
//! sh:26  	fi
//! sh:27        done
//! sh:28      elif (( $+commands[readlink] )); then
//! sh:29        for i in "${fds[@]}"; do
//! sh:30  	if link=$(readlink $proc/$i); then
//! sh:31  	  list+=( "${(r.$#fds[-1].)i} $sep ${(D)link}" )
//! sh:32  	else
//! sh:33  	  fds[(i)$i]=()
//! sh:34  	fi
//! sh:35        done
//! sh:36      else
//! sh:37        for i in "${fds[@]}"; do
//! sh:38  	if link=$(ls -l $proc/$i); then
//! sh:39  	  list+=( "${(r.$#fds[-1].)i} $sep ${(D)link#* -> }" )
//! sh:40  	else
//! sh:41  	  fds[(i)$i]=()
//! sh:42  	fi
//! sh:43        done
//! sh:44      fi 2>/dev/null
//! sh:45    fi
//! sh:46
//! sh:47    if (( list[(I)* $sep ?*] )); then
//! sh:48      list=(
//! sh:49        "${(r.$#fds[-1].):-0} $sep standard input"
//! sh:50        "${(r.$#fds[-1].):-1} $sep standard output"
//! sh:51        "${(r.$#fds[-1].):-2} $sep standard error" $list
//! sh:52      )
//! sh:53      disp=( -d list )
//! sh:54    fi
//! sh:55  fi
//! sh:56  fds=( 0 1 2 $fds )
//! sh:57
//! sh:58  _description -V file-descriptors expl 'file descriptor'
//! sh:59  compadd $disp -o nosort "$@" "$expl[@]" -a fds
//! ```
//!
//! Strict Rust port: walks our own `/dev/fd/N` entries for N ≥ 3,
//! filters numeric basenames. Lists fd 3 onwards (0/1/2 are
//! stdin/stdout/stderr — generally not what the user wants when
//! they're redirecting).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
