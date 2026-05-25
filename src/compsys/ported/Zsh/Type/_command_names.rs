//! Port of `_command_names` from `Completion/Zsh/Type/_command_names`.
//!
//! Full upstream body (74 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # The option `-e' if given as the first argument says that we should
//! sh: 4  # complete only external commands and executable files. This and a
//! sh: 5  # `-' as the first argument is then removed from the arguments.
//! sh: 6
//! sh: 7  local args defs expl ffilt verbose
//! sh: 8
//! sh: 9  zstyle -t ":completion:${curcontext}:commands" rehash && rehash
//! sh:10
//! sh:11  zstyle -t ":completion:${curcontext}:functions" prefix-needed && \
//! sh:12   [[ $PREFIX != [_.]* ]] && \
//! sh:13   ffilt='[(I)[^_.]*]'
//! sh:14
//! sh:15  defs=(
//! sh:16    'commands:external command:_path_commands'
//! sh:17  )
//! sh:18
//! sh:19  if [[ -n "$path[(r).]" || $PREFIX = */* ]]; then
//! sh:20    defs+=( 'executables:executable file:_files -g \*\(-\*\)' )
//! sh:21  else
//! sh:22    # this is ignored but exists to facilitate the use of the fake style
//! sh:23    _description executables expl 'executable file'
//! sh:24  fi
//! sh:25
//! sh:26  if [[ "$1" = -e ]]; then
//! sh:27    shift
//! sh:28  elif (( ${#precommands:|builtin_precommands} )); then
//! sh:29    # precommand excludes internal options below
//! sh:30  else
//! sh:31    [[ "$1" = - ]] && shift
//! sh:32
//! sh:33    defs=( "$defs[@]"
//! sh:34      'builtins:builtin command:compadd -Qk builtins'
//! sh:35      "functions:shell function:compadd -k 'functions$ffilt'"
//! sh:36      'suffix-aliases:suffix alias:_suffix_alias_files'
//! sh:37      'reserved-words:reserved word:compadd -Qk reswords'
//! sh:38      'jobs:: _jobs -t'
//! sh:39      'parameters:: _parameters -g "^*(readonly|association)*" -qS= -r "\n\t\- =[+"'
//! sh:40      'parameters:: _parameters -g "*association*~*readonly*" -qS\[ -r "\n\t\- =[+"'
//! sh:41    )
//! sh:42
//! sh:43    if zstyle -T ":completion:${curcontext}:aliases" verbose; then
//! sh:44      printf -v verbose %s:%s\  ${(@q+)${(kv)aliases}[@]//\:/\\:}
//! sh:45      defs+=( "aliases:alias:(( $verbose ))" )
//! sh:46    else
//! sh:47      defs+=( 'aliases:alias:compadd -Qk aliases' )
//! sh:48    fi
//! sh:49  fi
//! sh:50
//! sh:51  args=( "$@" )
//! sh:52
//! sh:53  local -a cmdpath
//! sh:54
//! sh:55  zstyle -a ":completion:${curcontext}" command-path cmdpath
//! sh:56
//! sh:57  # Using the current PATH doesn't necessarily make sense when completing commands
//! sh:58  # to tools like sudo, which might set a different one. A common issue is that
//! sh:59  # /**/sbin appear in the PATH used by the tool, but not in the one used by the
//! sh:60  # unprivileged user who calls it. To do the right thing in the most common
//! sh:61  # cases, we'll simply ensure that the sbin variants always appear here when not
//! sh:62  # otherwise overridden (bash-completion's _sudo does something similar)
//! sh:63  if (( ! $#cmdpath && $#_comp_priv_prefix )); then
//! sh:64    cmdpath=( $path ${path/%\/bin//sbin} )
//! sh:65    cmdpath=( ${(u)^cmdpath}(/-N) )
//! sh:66  fi
//! sh:67
//! sh:68  if (( $#cmdpath )); then
//! sh:69    local -a +h path
//! sh:70    local -A +h commands
//! sh:71    path=( $cmdpath:A )
//! sh:72  fi
//! sh:73
//! sh:74  _alternative -O args "$defs[@]"
//! ```
//!
//! Faithful re-port using P0 bridge + P2-wired ():
//!   * sh:9, sh:11-13, sh:43, sh:55 zstyle queries → real shell zstyle
//!     via [`crate::compsys::builtin_bridge::zstyle_lookup_*`].
//!   * sh:15-49 `defs` array → built explicitly; each def emitted by
//!     a small dispatcher.
//!   * sh:51 `args` (extra `"$@"`) → currently no-op (compsys callers
//!     don't pass extra args to `_command_names`).
//!   * sh:74 `_alternative -O args "$defs[@]"` → in shell this loops
//!     each def, opens a tag, runs the action. We unroll the loop:
//!     each enabled def's emitter runs directly with the inventory
//!     the caller supplied.
//!
//! Signature divergence (`// rust:`):
//!   * Shell reads `$PREFIX`, `$path`, `$aliases`, `$functions`,
//!     `$builtins`, `$reswords`, etc. from process globals. Rust
//!     takes a [`ShellInventory`] snapshot from the caller so the
//!     leaf doesn't have to fork the shell.
//!   * `_command_names` (1-arg state form) for callers that already
//!     hold a `CompletionState`; `_command_names_with_ctx` (2-arg
//!     `MainCompleteState` form) when the caller has the full
//!     state + context for zstyle lookups.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
