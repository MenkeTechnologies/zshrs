//! Port of `_comp_locale` from `Completion/Base/Utility/_comp_locale`.
//!
//! Full upstream body (20 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Arrange that LC_CTYPE retains the current setting so characters in
//! sh: 4  # file names are handled properly, but other locales are set to C so
//! sh: 5  # that the completion system can process output without surprises.
//! sh: 6
//! sh: 7  # This exports new locale settings, so should only
//! sh: 8  # be run in a subshell.  A typical use is in a $(...).
//! sh: 9
//! sh:10  local ctype
//! sh:11
//! sh:12  if ctype=${${(f)"$(locale 2>/dev/null)"}:#^LC_CTYPE=*}; then
//! sh:13      unset -m LC_\*
//! sh:14      [[ -n $ctype ]] && eval export $ctype
//! sh:15  else
//! sh:16      ctype=${LC_ALL:-${LC_CTYPE:-${LANG:-C}}}
//! sh:17      unset -m LC_\*
//! sh:18      export LC_CTYPE=$ctype
//! sh:19  fi
//! sh:20  export LANG=C
//! ```
//!
//! Upstream sets a C-locale env for sane completion-tool output
//! while keeping LC_CTYPE for filename byte interpretation. The
//! comment says it MUST be run in a subshell (changes calling
//! shell's env).
//!
//! Faithful Rust port: actually performs the env mutation, mirroring
//! the shell exactly:
//! - shell:12 / 16 `unset -m LC_\*` — unset every LC_* env var
//! - shell:11-13 — preserve the system locale's LC_CTYPE setting
//! (read via `locale` command, mirroring the shell's command-
//! substitution-style read)
//! - shell:15-17 fallback — when `locale` fails, build LC_CTYPE
//! from `${LC_ALL:-${LC_CTYPE:-${LANG:-C}}}`
//! - shell:19 — `export LANG=C`
//!
//! Returns the LC_CTYPE value that was set, so callers can verify.
//! Callers should run this in a forked subprocess (matching shell's
//! "should only be run in a subshell" comment) to avoid mutating
//! the parent's env permanently.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
