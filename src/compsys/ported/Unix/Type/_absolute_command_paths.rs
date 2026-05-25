//! Port of `_absolute_command_paths` from `Completion/Unix/Type/_absolute_command_paths`.
//!
//! Full upstream body (37 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This function completes 'ls' to '/bin/ls'
//! sh: 4  _hashed_absolute_command_paths() {
//! sh: 5    local -aU set_of_dirs_of_hashed_commands=( ${^commands%/*}/ )
//! sh: 6    local i
//! sh: 7    integer ret=1
//! sh: 8    for i in $set_of_dirs_of_hashed_commands
//! sh: 9    do
//! sh:10      local -a matches=( "${(@)commands[(R)${~i}[^/]#]}" )
//! sh:11      local -a descs=( $matches:t )
//! sh:12      compadd -M "l:|=$i" -d descs "$@" -a matches
//! sh:13      ret=0
//! sh:14    done
//! sh:15    return ret
//! sh:16  }
//! sh:17
//! sh:18  # This function completes absolute pathnames of executables, e.g., /etc/rc.local
//! sh:19  _typed-in_absolute_command_paths() {
//! sh:20    # TODO: the description "full path to an executable" and tag in the caller are ignored by _path_files
//! sh:21    if [[ -z $PREFIX ]]; then
//! sh:22      _path_files -/ -g '*(-*)' -P / -W /
//! sh:23    elif [[ $PREFIX[1] == / ]]; then
//! sh:24      _path_files -/ -g '*(-*)' -W /
//! sh:25    else
//! sh:26      return 1
//! sh:27    fi
//! sh:28  }
//! sh:29
//! sh:30  _absolute_command_paths() {
//! sh:31    _alternative \
//! sh:32      'commands:hashed command by absolute path:_hashed_absolute_command_paths' \
//! sh:33      'commands:full path to an executable:_typed-in_absolute_command_paths'
//! sh:34  }
//! sh:35
//! sh:36
//! sh:37  _absolute_command_paths "$@"
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
