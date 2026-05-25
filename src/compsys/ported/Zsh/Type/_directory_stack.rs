//! Port of `_directory_stack` from `Completion/Zsh/Type/_directory_stack`.
//!
//! Full upstream body (45 lines verbatim):
//! ```text
//! sh: 1  #compdef popd
//! sh: 2
//! sh: 3  # This just completes the numbers after +, showing the full directory list
//! sh: 4  # with numbers. For - we do the same thing, but reverse the numbering (other
//! sh: 5  # way round if pushdminus is set). Note that this function is also called
//! sh: 6  # from _cd for cd and pushd.
//! sh: 7
//! sh: 8  setopt localoptions nonomatch
//! sh: 9
//! sh:10  local expl list lines revlines disp sep
//! sh:11
//! sh:12  ### we decided against this, for now...
//! sh:13  #! zstyle -T ":completion:${curcontext}:directory-stack" prefix-needed ||
//! sh:14
//! sh:15  [[ $PREFIX = [-+]* ]] || return 1
//! sh:16
//! sh:17  zstyle -s ":completion:${curcontext}:directory-stack" list-separator sep || sep=--
//! sh:18
//! sh:19  if zstyle -T ":completion:${curcontext}:directory-stack" verbose; then
//! sh:20    # get the list of directories with their canonical number
//! sh:21    # and turn the lines into an array, removing the current directory
//! sh:22    lines=("${(D)dirstack[@]}")
//! sh:23
//! sh:24    if [[ ( $PREFIX[1] = - && ! -o pushdminus ) ||
//! sh:25          ( $PREFIX[1] = + && -o pushdminus ) ]]; then
//! sh:26      integer i
//! sh:27      revlines=( $lines )
//! sh:28      for (( i = 1; i <= $#lines; i++ )); do
//! sh:29        lines[$i]="$((i-1)) $sep ${revlines[-$i]##[0-9]#[	 ]#}"
//! sh:30      done
//! sh:31    else
//! sh:32      for (( i = 1; i <= $#lines; i++ )); do
//! sh:33        lines[$i]="$i $sep ${lines[$i]##[0-9]#[	 ]#}"
//! sh:34      done
//! sh:35    fi
//! sh:36    # get the array of numbers only
//! sh:37    list=( ${PREFIX[1]}${^lines%% *} )
//! sh:38    disp=( -ld lines )
//! sh:39  else
//! sh:40    list=( ${PREFIX[1]}{0..${#dirstack}} )
//! sh:41    disp=()
//! sh:42  fi
//! sh:43
//! sh:44  _wanted -V directory-stack expl 'directory stack' \
//! sh:45      compadd "$@" "$disp[@]" -Q -a list
//! ```
//!
//! `pushdminus` option flips the meaning of `+` vs `-`. Our port
//! takes both the dirstack (rendered) and the `pushdminus` bool
//! from the caller.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
