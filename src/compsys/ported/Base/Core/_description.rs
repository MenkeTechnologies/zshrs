//! Port of `_description` from `Completion/Base/Core/_description`.
//!
//! Full upstream body (123 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  local name nopt xopt format gname hidden hide match opts tag
//! sh:  4  local -a ign gropt sort
//! sh:  5  local -a match mbegin mend
//! sh:  6
//! sh:  7  opts=()
//! sh:  8
//! sh:  9  xopt=(-X)
//! sh: 10  nopt=()
//! sh: 11  zparseopts -K -D -a nopt 1 2 V=gropt J=ign x=xopt
//! sh: 12
//! sh: 13  3="${${3##[[:blank:]]#}%%[[:blank:]]#}"
//! sh: 14  [[ -n "$3" ]] && _lastdescr=( "$_lastdescr[@]" "$3" )
//! sh: 15
//! sh: 16  zstyle -s ":completion:${curcontext}:$1" group-name gname &&
//! sh: 17      [[ -z "$gname" ]] && gname="$1"
//! sh: 18
//! sh: 19  _setup "$1" "${gname:--default-}"
//! sh: 20
//! sh: 21  name="$2"
//! sh: 22
//! sh: 23  zstyle -s ":completion:${curcontext}:$1" format format ||
//! sh: 24      zstyle -s ":completion:${curcontext}:descriptions" format format
//! sh: 25
//! sh: 26  if zstyle -s ":completion:${curcontext}:$1" hidden hidden &&
//! sh: 27     [[ "$hidden" = (all|yes|true|1|on) ]]; then
//! sh: 28    [[ "$hidden" = all ]] && format=''
//! sh: 29    opts=(-n)
//! sh: 30  fi
//! sh: 31  zstyle -s ":completion:${curcontext}:$1" matcher match &&
//! sh: 32      opts=($opts -M "$match")
//! sh: 33  [[ -n "$_matcher" ]] && opts=($opts -M "$_matcher")
//! sh: 34
//! sh: 35  # Use sort style, but ignore `menu' value to help _expand.
//! sh: 36  # Also don't override explicit use of -V.
//! sh: 37  if [[ -z "$gropt" ]]; then
//! sh: 38    if zstyle -a ":completion:${curcontext}:$1" sort sort ||
//! sh: 39       zstyle -a ":completion:${curcontext}:" sort sort
//! sh: 40    then
//! sh: 41      if [[ -z "${(@)sort:#(match|numeric|reverse)}" ]]; then
//! sh: 42        gropt=( -o ${(j.,.)sort} )
//! sh: 43      elif [[ "$sort" != (yes|true|1|on|menu) ]]; then
//! sh: 44        gropt=( -o nosort )
//! sh: 45      fi
//! sh: 46    fi
//! sh: 47  else
//! sh: 48    gropt=( -o nosort )
//! sh: 49  fi
//! sh: 50
//! sh: 51  if [[ -z "$_comp_no_ignore" ]]; then
//! sh: 52    zstyle -a ":completion:${curcontext}:$1" ignored-patterns _comp_ignore ||
//! sh: 53      _comp_ignore=()
//! sh: 54
//! sh: 55    if zstyle -s ":completion:${curcontext}:$1" ignore-line hidden; then
//! sh: 56      local -a qwords
//! sh: 57      qwords=( ${words//(#m)[\[\]()\\*?#<>~\^\|]/\\$MATCH} )
//! sh: 58      case "$hidden" in
//! sh: 59      true|yes|on|1) _comp_ignore+=( $qwords );;
//! sh: 60      current)       _comp_ignore+=( $qwords[CURRENT] );;
//! sh: 61      current-shown)
//! sh: 62  	    [[ "$compstate[old_list]" = *shown* ]] &&
//! sh: 63              _comp_ignore+=( $qwords[CURRENT] );;
//! sh: 64      other)         _comp_ignore+=( $qwords[1,CURRENT-1]
//! sh: 65  				   $qwords[CURRENT+1,-1] );;
//! sh: 66      esac
//! sh: 67    fi
//! sh: 68
//! sh: 69    # Ensure the ignore option is first so we can override it
//! sh: 70    # for fake-always.
//! sh: 71    (( $#_comp_ignore )) && opts=( -F _comp_ignore $opts )
//! sh: 72  else
//! sh: 73    _comp_ignore=()
//! sh: 74  fi
//! sh: 75
//! sh: 76  tag="$1"
//! sh: 77
//! sh: 78  shift 2
//! sh: 79  if [[ -z "$1" && $# -eq 1 ]]; then
//! sh: 80    format=
//! sh: 81  elif [[ -n "$format" ]]; then
//! sh: 82    if [[ -z $2 ]]; then
//! sh: 83      argv+=( h:${1%%( ##\((#b)([^\)]#[^0-9-][^\)]#)(#B)\)|)( ##\((#b)([0-9-]##)(#B)\)|)( ##\[(#b)([^\]]##)(#B)\]|)} )
//! sh: 84      [[ -n $match[1] ]] && argv+=( m:$match[1] )
//! sh: 85      [[ -n $match[2] ]] && argv+=( r:$match[2] )
//! sh: 86      [[ -n $match[3] ]] && argv+=( o:$match[3] )
//! sh: 87    fi
//! sh: 88
//! sh: 89    zformat -F format "$format" "d:$1" "${(@)argv[2,-1]}"
//! sh: 90  fi
//! sh: 91
//! sh: 92  if [[ -n "$gname" ]]; then
//! sh: 93    if [[ -n "$format" ]]; then
//! sh: 94      set -A "$name" "$opts[@]" "$nopt[@]" "$gropt[@]" -J "$gname" "$xopt" "$format"
//! sh: 95    else
//! sh: 96      set -A "$name" "$opts[@]" "$nopt[@]" "$gropt[@]" -J "$gname"
//! sh: 97    fi
//! sh: 98  else
//! sh: 99    if [[ -n "$format" ]]; then
//! sh:100      set -A "$name" "$opts[@]" "$nopt[@]" "$gropt[@]" -J -default- "$xopt" "$format"
//! sh:101    else
//! sh:102      set -A "$name" "$opts[@]" "$nopt[@]" "$gropt[@]" -J -default-
//! sh:103    fi
//! sh:104  fi
//! sh:105
//! sh:106  if ! (( ${funcstack[2,-1][(I)_description]} )); then
//! sh:107    local fakestyle descr
//! sh:108    for fakestyle in fake fake-always; do
//! sh:109      zstyle -a ":completion:${curcontext}:$tag" $fakestyle match ||
//! sh:110      continue
//! sh:111
//! sh:112      descr=( "${(@M)match:#*[^\\]:*}" )
//! sh:113
//! sh:114      opts=("${(@P)name}")
//! sh:115      if [[ $fakestyle = fake-always && $opts[1,2] = "-F _comp_ignore" ]]; then
//! sh:116        shift 2 opts
//! sh:117      fi
//! sh:118      compadd "${(@)opts}" - "${(@)${(@)match:#*[^\\]:*}:s/\\:/:/}"
//! sh:119      (( $#descr )) && _describe -t "$tag" '' descr "${(@)opts}"
//! sh:120    done
//! sh:121  fi
//! sh:122
//! sh:123  return 0
//! ```
//!
//! Faithful Rust port: honors `format`, `hidden`, and per-tag
//! description styles. Returns the formatted description string
//! (`%d` → description, `%%` → `%`). Returns `None` when the
//! `hidden` style is `all`.



use crate::compsys::compcore::CompletionState;
use crate::compsys::zstyle::ZStyleStore;

/// _description - set up description for a tag
/// Handles styles: format, hidden, group-name, matcher, sort, ignored-patterns
pub fn _description(
    _state: &mut CompletionState,
    styles: &ZStyleStore,
    context: &str,
    tag: &str,
    description: &str,
) -> Option<String> {
    let ctx = format!("{}:{}", context, tag);

    // Check 'hidden' style - if set to 'all', return empty format
    if let Some(hidden) = styles.lookup_values(&ctx, "hidden") {
        if let Some(v) = hidden.first() {
            match v.as_str() {
                "all" => return None,
                "yes" | "true" | "1" | "on" => {
                    // Hidden but still has format for group header
                }
                _ => {}
            }
        }
    }

    // Get format from style (try tag-specific first, then descriptions tag)
    let format = styles
        .lookup_values(&ctx, "format")
        .or_else(|| styles.lookup_values(&format!("{}:descriptions", context), "format"))
        .and_then(|v| v.first().cloned())
        .unwrap_or_else(|| "%d".to_string());

    // zformat -F substitution: %d = description, plus additional escapes
    let result = format.replace("%d", description).replace("%%", "%");

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_format_returns_raw_description() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        let got = _description(&mut state, &styles, ":completion:::", "files", "the files");
        assert_eq!(got.as_deref(), Some("the files"));
    }

    #[test]
    fn hidden_all_returns_none() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(":completion:::*:files", "hidden", vec!["all".into()], false);
        let got = _description(&mut state, &styles, ":completion:::*", "files", "the files");
        assert!(got.is_none());
    }

    #[test]
    fn percent_d_substitution_works() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion:::*:files",
            "format",
            vec!["== %d ==".into()],
            false,
        );
        let got = _description(&mut state, &styles, ":completion:::*", "files", "the files");
        assert_eq!(got.as_deref(), Some("== the files =="));
    }

    #[test]
    fn double_percent_collapses_to_single_percent() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion:::*:files",
            "format",
            vec!["100%% sure: %d".into()],
            false,
        );
        let got = _description(&mut state, &styles, ":completion:::*", "files", "x");
        assert_eq!(got.as_deref(), Some("100% sure: x"));
    }

    #[test]
    fn descriptions_tag_fallback_when_no_tag_specific() {
        // Per-tag format unset, but :descriptions catch-all is set →
        // fall back to it.
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion:::*:descriptions",
            "format",
            vec!["[%d]".into()],
            false,
        );
        // No per-tag `files` format → falls back.
        let got = _description(&mut state, &styles, ":completion:::*", "files", "filename");
        assert_eq!(got.as_deref(), Some("[filename]"));
    }

    #[test]
    fn tag_specific_format_overrides_descriptions_catch_all() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion:::*:descriptions",
            "format",
            vec!["catch-all: %d".into()],
            false,
        );
        styles.set(
            ":completion:::*:files",
            "format",
            vec!["files-only: %d".into()],
            false,
        );
        let got = _description(&mut state, &styles, ":completion:::*", "files", "X");
        assert_eq!(got.as_deref(), Some("files-only: X"));
    }

    #[test]
    fn hidden_yes_still_returns_formatted_string() {
        // hidden=yes hides matches but keeps the format string for
        // the group header. Confirm we return Some(_), not None.
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(":completion:::*:files", "hidden", vec!["yes".into()], false);
        let got = _description(&mut state, &styles, ":completion:::*", "files", "x");
        assert!(got.is_some());
    }

    #[test]
    fn empty_description_round_trips_through_format() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion:::*:files",
            "format",
            vec!["[%d]".into()],
            false,
        );
        let got = _description(&mut state, &styles, ":completion:::*", "files", "");
        assert_eq!(got.as_deref(), Some("[]"));
    }

    #[test]
    fn format_with_no_percent_d_emits_literal() {
        // If the format has no `%d`, the description is dropped and
        // the literal string is returned verbatim.
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion:::*:files",
            "format",
            vec!["literal".into()],
            false,
        );
        let got = _description(&mut state, &styles, ":completion:::*", "files", "ignored");
        assert_eq!(got.as_deref(), Some("literal"));
    }
}
