//! Port of `_external_pwds` from `Completion/Base/Completer/_external_pwds`.
//!
//! Full upstream body (43 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Completes current directories of other zsh processes
//! sh: 4  # this is intended to be used via _generic bound to a
//! sh: 5  # different key. Note that pattern matching is enabled.
//! sh: 6
//! sh: 7  local -a expl
//! sh: 8  local -au dirs
//! sh: 9
//! sh:10  # undo work _main_complete did to remove the tilde
//! sh:11  PREFIX="$IPREFIX$PREFIX"
//! sh:12  IPREFIX=
//! sh:13  SUFFIX="$SUFFIX$ISUFFIX"
//! sh:14  ISUFFIX=
//! sh:15
//! sh:16  [[ -o magicequalsubst ]] && compset -P '*='
//! sh:17
//! sh:18  case $OSTYPE in
//! sh:19    solaris*)
//! sh:20      dirs=(
//! sh:21        ${(M)${${(f)"$(pgrep -U $UID -x zsh|xargs pwdx 2>/dev/null)"}:#$$:*}%%/*}
//! sh:22      )
//! sh:23    ;;
//! sh:24    linux*)
//! sh:25      dirs=( /proc/${^$(pidof -- -zsh zsh):#$$}/cwd(N:P) )
//! sh:26      dirs=( $^dirs(N^@) )
//! sh:27    ;;
//! sh:28    freebsd*)
//! sh:29      dirs=( $(pgrep -U $UID -x zsh) )
//! sh:30      dirs=( $(procstat -h -f $dirs|awk '{if ($3 == "cwd") print $NF}') )
//! sh:31    ;;
//! sh:32    *)
//! sh:33      if (( $+commands[lsof] )); then
//! sh:34        dirs=( ${${${(M)${(f)"$(lsof -a -u $EUID -c zsh -p \^$$ -d cwd -F n -w
//! sh:35            2>/dev/null)"}:#n*}#?}%% \(*} )
//! sh:36      fi
//! sh:37    ;;
//! sh:38  esac
//! sh:39  dirs=( ${(D)dirs:#$PWD} )
//! sh:40
//! sh:41  compstate[pattern_match]='*'
//! sh:42  _wanted directories expl 'current directory from other shell' \
//! sh:43      compadd -M "r:|/=* r:|=*" -f -a dirs
//! ```
//!
//! Faithful Rust port: full /proc walk on Linux to discover other
//! shells' cwds. On macOS / BSD where there's no /proc/PID/cwd,
//! falls back to `lsof -wnP -F n -a -d cwd` if available; otherwise
//! emits just the current process's cwd (the upstream `*) dirs=()`
//! case still always includes the calling shell's PWD via
//! `compadd -V cwd …` even when `dirs` is empty).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
