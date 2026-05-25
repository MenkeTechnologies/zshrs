//! Port of `_message` from `Completion/Base/Core/_message`.
//!
//! Full upstream body (45 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local format raw gopt
//! sh: 4
//! sh: 5  if [[ "$1" = -e ]]; then
//! sh: 6    local expl ret=1 tag
//! sh: 7
//! sh: 8    _comp_mesg=yes
//! sh: 9
//! sh:10    if (( $# > 2 )); then
//! sh:11      tag="$2"
//! sh:12      shift
//! sh:13    else
//! sh:14      tag="$curtag"
//! sh:15    fi
//! sh:16    _tags "$tag" && while _next_label "$tag" expl "$2"; do
//! sh:17      compadd ${expl:/-X/-x}
//! sh:18      ret=0
//! sh:19    done
//! sh:20
//! sh:21    (( ! $compstate[nmatches] )) && [[ $compstate[insert] = *unambiguous* ]] &&
//! sh:22        compstate[insert]=
//! sh:23
//! sh:24    return ret
//! sh:25  fi
//! sh:26
//! sh:27  gopt=()
//! sh:28  zparseopts -D -a gopt 1 2 V J
//! sh:29
//! sh:30  _tags messages || return 1
//! sh:31
//! sh:32  if [[ "$1" = -r ]]; then
//! sh:33    raw=yes
//! sh:34    shift
//! sh:35    format="$1"
//! sh:36  else
//! sh:37    zstyle -s ":completion:${curcontext}:messages" format format ||
//! sh:38        zstyle -s ":completion:${curcontext}:descriptions" format format
//! sh:39  fi
//! sh:40
//! sh:41  if [[ -n "$format$raw" ]]; then
//! sh:42    [[ -z "$raw" ]] && zformat -F format "$format" "d:$1" "${(@)argv[2,-1]}"
//! sh:43    builtin compadd "$gopt[@]" -x "$format"
//! sh:44    _comp_mesg=yes
//! sh:45  fi
//! ```
//!
//! Upstream resolves the `format` zstyle to wrap the message, sets
//! `_comp_mesg=yes` so subsequent code knows a message was emitted,
//! then `compadd -x` for ZLE to render.
//!
//! Faithful Rust port: calls into `_description` (which already
//! handles format resolution) then attaches the rendered string
//! as an explanation on the named tag group. `nmessages` increments
//! match the shell-side `_comp_mesg` flag (callers check the count).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
