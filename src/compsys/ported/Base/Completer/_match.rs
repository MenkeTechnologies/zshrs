//! Port of `_match` from `Completion/Base/Completer/_match`.
//!
//! Full upstream body (81 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This is intended to be used as a completer function after the normal
//! sh: 4  # completer as in: `zstyle ":completion:::::" completer _complete _match'.
//! sh: 5  # It temporarily switches on pattern matching, allowing you to try
//! sh: 6  # completion on patterns without having to setopt glob_complete.
//! sh: 7  #
//! sh: 8  # Note, however, that this is only really useful if you don't use the
//! sh: 9  # expand-or-complete function because otherwise the pattern will
//! sh:10  # be expanded using globbing.
//! sh:11
//! sh:12  ### Shouldn't be needed any more: [[ _matcher_num -gt 1 ]] && return 1
//! sh:13
//! sh:14  local tmp opm="$compstate[pattern_match]" ret=1 orig ins
//! sh:15  local oms="$_old_match_string"
//! sh:16  local ocsi="$compstate[insert]" ocspi="$compstate[pattern_insert]"
//! sh:17
//! sh:18  # Do nothing if we don't have a pattern.
//! sh:19
//! sh:20  tmp="${${:-$PREFIX$SUFFIX}#[~=]}"
//! sh:21  [[ "$tmp:q" = "$tmp" ]] && return 1
//! sh:22
//! sh:23  _old_match_string="$PREFIX$SUFFIX$HISTNO"
//! sh:24
//! sh:25  _tags matches original
//! sh:26
//! sh:27  zstyle -s ":completion:${curcontext}:" match-original orig
//! sh:28  zstyle -s ":completion:${curcontext}:" insert-unambiguous ins
//! sh:29
//! sh:30  # Try completion without inserting a `*'?
//! sh:31
//! sh:32  if [[ -n "$orig" ]]; then
//! sh:33    compstate[pattern_match]='-'
//! sh:34    _complete && ret=0
//! sh:35    compstate[pattern_match]="$opm"
//! sh:36
//! sh:37    # No completion with inserting `*'?
//! sh:38
//! sh:39    [[ ret -eq 1 && "$orig" = only ]] && return 1
//! sh:40  fi
//! sh:41
//! sh:42  if (( ret )); then
//! sh:43    compstate[pattern_match]='*'
//! sh:44    _complete && ret=0
//! sh:45    compstate[pattern_match]="$opm"
//! sh:46  fi
//! sh:47
//! sh:48  if (( ! ret )); then
//! sh:49
//! sh:50    if [[ "$ins" = pattern && $compstate[nmatches] -gt 1 ]]; then
//! sh:51
//! sh:52      [[ "$oms" = "$PREFIX$SUFFIX$HISTNO" &&
//! sh:53         "$compstate[insert]" = automenu-unambiguous ]] &&
//! sh:54          compstate[insert]=automenu
//! sh:55      [[ "$compstate[insert]" != *menu ]] &&
//! sh:56          compstate[pattern_insert]= compstate[insert]=
//! sh:57
//! sh:58  # We tried to be clever here, making completion insert unambiguous
//! sh:59  # expansions as early as possible, but this is really hard to test
//! sh:60  # and the code below probably does more harm than good.
//! sh:61  #
//! sh:62  #    [[ $compstate[unambiguous_cursor] -gt $#compstate[unambiguous] ]] &&
//! sh:63  #        ins=yes compstate[insert]="$ocsi" compstate[pattern_insert]="$ocspi"
//! sh:64    fi
//! sh:65
//! sh:66    if [[ "$ins" = (true|yes|on|1) &&
//! sh:67        $#compstate[unambiguous] -ge ${#:-${PREFIX}${SUFFIX}} ]]
//! sh:68    then
//! sh:69      compstate[pattern_insert]=unambiguous
//! sh:70    elif _requested original &&
//! sh:71        { [[ compstate[nmatches] -gt 1 ]] ||
//! sh:72  	zstyle -t ":completion:${curcontext}:" original }; then
//! sh:73      local expl
//! sh:74
//! sh:75      _description -V original expl original
//! sh:76
//! sh:77      compadd "$expl[@]" -U -Q - "$PREFIX$SUFFIX"
//! sh:78    fi
//! sh:79  fi
//! sh:80
//! sh:81  return ret
//! ```
//!
//! Upstream flips `compstate[pattern_match]='*'` and re-runs the
//! previous completers so they accept glob-pattern input (user types
//! `*.rs<TAB>` and gets matches the literal-prefix completer
//! wouldn't produce).
//!
//! Simplified Rust port: takes the pattern + candidate list directly
//! and emits candidates that glob-match. Supports `*` and `?`.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
