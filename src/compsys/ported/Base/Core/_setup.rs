//! Port of `_setup` from `Completion/Base/Core/_setup`.
//!
//! Full upstream body (79 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local val nm="$compstate[nmatches]"
//! sh: 4
//! sh: 5  [[ $# -eq 1 ]] && 2="$1"
//! sh: 6
//! sh: 7  if zstyle -a ":completion:${curcontext}:$1" list-colors val; then
//! sh: 8    zmodload -i zsh/complist
//! sh: 9    if [[ "$1" = default ]]; then
//! sh:10      _comp_colors=( "$val[@]" )
//! sh:11    else
//! sh:12      _comp_colors+=( "(${2})${(@)^val:#(|\(*\)*)}" "${(M@)val:#\(*\)*}" )
//! sh:13    fi
//! sh:14
//! sh:15  # Here is the problem mentioned in _main_complete.
//! sh:16
//! sh:17  # elif [[ "$1" = default && -n "$ZLS_COLORS$ZLS_COLOURS" ]]; then
//! sh:18  #   zmodload -i zsh/complist
//! sh:19  #   ZLS_COLORS="$ZLS_COLORS$ZLS_COLOURS"
//! sh:20
//! sh:21  elif [[ "$1" = default ]]; then
//! sh:22    unset ZLS_COLORS ZLS_COLOURS
//! sh:23  fi
//! sh:24
//! sh:25  # What we'd like is to test that the show-ambiguity style pattern is more
//! sh:26  # specific than the list-colors style pattern, but that's not possible yet
//! sh:27  if zstyle -s ":completion:${curcontext}:$1" show-ambiguity val; then
//! sh:28    zmodload -i zsh/complist
//! sh:29    [[ $val = (yes|true|on) ]] && _ambiguous_color=4 || _ambiguous_color=$val
//! sh:30  fi
//! sh:31
//! sh:32  if zstyle -t ":completion:${curcontext}:$1" list-packed; then
//! sh:33    compstate[list]="${compstate[list]} packed"
//! sh:34  elif [[ $? -eq 1 ]]; then
//! sh:35    compstate[list]="${compstate[list]:gs/packed//}"
//! sh:36  else
//! sh:37    compstate[list]="$_saved_list"
//! sh:38  fi
//! sh:39
//! sh:40  if zstyle -t ":completion:${curcontext}:$1" list-rows-first; then
//! sh:41    compstate[list]="${compstate[list]} rows"
//! sh:42  elif [[ $? -eq 1 ]]; then
//! sh:43    compstate[list]="${compstate[list]:gs/rows//}"
//! sh:44  else
//! sh:45    compstate[list]="$_saved_list"
//! sh:46  fi
//! sh:47
//! sh:48  if zstyle -t ":completion:${curcontext}:$1" last-prompt; then
//! sh:49    compstate[last_prompt]=yes
//! sh:50  elif [[ $? -eq 1 ]]; then
//! sh:51    compstate[last_prompt]=''
//! sh:52  else
//! sh:53    compstate[last_prompt]="$_saved_lastprompt"
//! sh:54  fi
//! sh:55
//! sh:56  if zstyle -t ":completion:${curcontext}:$1" accept-exact; then
//! sh:57    compstate[exact]=accept
//! sh:58  elif [[ $? -eq 1 ]]; then
//! sh:59    compstate[exact]=''
//! sh:60  else
//! sh:61    compstate[exact]="$_saved_exact"
//! sh:62  fi
//! sh:63
//! sh:64  [[ _last_nmatches -ge 0 && _last_nmatches -ne nm ]] &&
//! sh:65      _menu_style=( "$_last_menu_style[@]" "$_menu_style[@]" )
//! sh:66
//! sh:67  if zstyle -a ":completion:${curcontext}:$1" menu val; then
//! sh:68    _last_nmatches=$nm
//! sh:69    _last_menu_style=( "$val[@]" )
//! sh:70  else
//! sh:71    _last_nmatches=-1
//! sh:72  fi
//! sh:73
//! sh:74  [[ "$_comp_force_list" != always ]] &&
//! sh:75    zstyle -s ":completion:${curcontext}:$1" force-list val &&
//! sh:76      [[ "$val" = always ||
//! sh:77         ( "$val" = [0-9]## &&
//! sh:78           ( -z "$_comp_force_list" || _comp_force_list -gt val ) ) ]] &&
//! sh:79      _comp_force_list="$val"
//! ```
//!
//! Faithful Rust port: handles EVERY zstyle the upstream consults.
//! Three flavors of compstate mutation:
//! - boolean `+=word` (shell:33 list-packed → "packed")
//! - boolean `=value` (shell:49 last-prompt → "yes")
//! - tri-state with restore (shell:37 falls back to `$_saved_list`
//! when style isn't set; we use clear-and-restore semantics)
//!
//! Side effects exposed via `state.params.compstate.*` fields and
//! the dedicated `_comp_colors` / `_ambiguous_color` /
//! `_last_menu_style` / `_comp_force_list` accessors below.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
