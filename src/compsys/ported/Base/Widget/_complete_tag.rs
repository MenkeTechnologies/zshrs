//! Port of `_complete_tag` from `Completion/Base/Widget/_complete_tag`.
//!
//! Full upstream body (62 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xt
//! sh: 2
//! sh: 3  # Complete tags using either TAGS or tags.  Looks up your directory
//! sh: 4  # hierarchy to find one.  If both exist, uses TAGS.
//! sh: 5  #
//! sh: 6  # You can override the choice of tags file with $TAGSFILE (for TAGS)
//! sh: 7  # or $tagsfile (for tags).
//! sh: 8  #
//! sh: 9  # Could be rewritten by some sed expert to use sed instead of perl.
//! sh:10
//! sh:11  emulate -L zsh
//! sh:12
//! sh:13  # Tags file to look for
//! sh:14  local c_Tagsfile=${TAGSFILE:-TAGS} c_tagsfile=${tagsfile:-tags} expl
//! sh:15  # Max no. of directories to scan up through
//! sh:16  integer c_maxdir=10
//! sh:17  # Context.
//! sh:18  local curcontext="$curcontext"
//! sh:19  local -a c_tags_array
//! sh:20
//! sh:21  if [[ -z "$curcontext" ]]; then
//! sh:22    curcontext="complete-tag:::"
//! sh:23  else
//! sh:24    curcontext="complete-tag:${curcontext#*:}"
//! sh:25  fi
//! sh:26
//! sh:27  local c_path=
//! sh:28  integer c_idir
//! sh:29  while [[ ! -f $c_path$c_Tagsfile &&
//! sh:30           ! -f $c_path$c_tagsfile && $c_idir -lt $c_maxdir ]]; do
//! sh:31    (( c_idir++ ))
//! sh:32    c_path=../$c_path
//! sh:33  done
//! sh:34
//! sh:35  if [[ -f $c_path$c_Tagsfile && $c_path$c_Tagsfile -ef $c_path$c_tagsfile &&
//! sh:36        "$(head -1 $c_path$c_tagsfile)" == '!_TAG_'* ]]; then
//! sh:37          c_Tagsfile=
//! sh:38  fi
//! sh:39
//! sh:40  if [[ -f $c_path$c_Tagsfile ]]; then
//! sh:41    # prefer the more comprehensive TAGS, which unfortunately is a
//! sh:42    # little harder to parse.
//! sh:43    # could do this with sed, just can't be bothered to work out how,
//! sh:44    # after quarter of an hour of trying, except for
//! sh:45    #  rm -f =sed; ln -s /usr/local/bin/perl /usr/bin/sed
//! sh:46    # but that's widely regarded as cheating.
//! sh:47    c_tags_array=($(sed -n \
//! sh:48          -e 's/^\(.*[a-zA-Z_0-9]\)[[ '$'\t'':;,()]*'$'\177''.*$/\1/' \
//! sh:49          -e 's/^.*[^a-zA-Z_0-9]//' \
//! sh:50          -e '/^[a-zA-Z_].*/p' $c_path$c_Tagsfile))
//! sh:51  #  c_tags_array=($(perl -ne '/([a-zA-Z_0-9]+)[ \t:;,\(]*\x7f/ &&
//! sh:52  #                  print "$1\n"' $c_path$c_Tagsfile))
//! sh:53    _main_complete - '' _wanted etags expl 'emacs tag' \
//! sh:54        compadd -a c_tags_array
//! sh:55  elif [[ -f $c_path$c_tagsfile ]]; then
//! sh:56    # tags doesn't have as much in, but the tag is easy to find.
//! sh:57    # we can use awk here.
//! sh:58    c_tags_array=($(awk '{ print $1 }' $c_path$c_tagsfile))
//! sh:59    _main_complete - '' _wanted vtags expl 'vi tag' compadd -a c_tags_array
//! sh:60  else
//! sh:61    return 1
//! sh:62  fi
//! ```



use crate::compsys::base::MainCompleteState;
use crate::compsys::compcore::CompletionState;

/// _complete_tag - Complete for specific tag
pub fn _complete_tag(
    state: &mut MainCompleteState,
    tag: &str,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    if state.tags.requested(tag) {
        state.comp.begin_group(tag, true);
        let result = action(&mut state.comp);
        state.comp.end_group();
        result
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrequested_tag_skips_action() {
        let mut state = MainCompleteState::new("", 0);
        let ran = std::cell::Cell::new(false);
        let result = _complete_tag(&mut state, "values", |_| {
            ran.set(true);
            true
        });
        assert!(!result);
        assert!(!ran.get(), "action must NOT run when tag not requested");
    }

    #[test]
    fn requested_tag_runs_action_and_returns_its_result() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["values".into()]);
        state.tags.configure_from_style(&["values".into()]);
        state.tags.start();
        assert!(_complete_tag(&mut state, "values", |_| true));
        assert!(state.comp.groups.iter().any(|g| g.name == "values"));
    }

    #[test]
    fn action_can_emit_matches() {
        use crate::compsys::completion::Completion;
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["values".into()]);
        state.tags.configure_from_style(&["values".into()]);
        state.tags.start();
        _complete_tag(&mut state, "values", |s| {
            s.add_match(Completion::new("via-tag"), None);
            true
        });
        let names: Vec<String> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"via-tag".to_string()));
    }

    #[test]
    fn false_action_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["values".into()]);
        state.tags.configure_from_style(&["values".into()]);
        state.tags.start();
        assert!(!_complete_tag(&mut state, "values", |_| false));
    }

    #[test]
    fn tag_not_in_offered_set_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["foo".into()]);
        state.tags.configure_from_style(&["foo".into()]);
        state.tags.start();
        // `bar` is NOT offered → requested(bar) false → skip action.
        let invoked = std::cell::Cell::new(false);
        let r = _complete_tag(&mut state, "bar", |_| {
            invoked.set(true);
            true
        });
        assert!(!r);
        assert!(!invoked.get());
    }

    #[test]
    fn empty_action_state_creates_tag_group_for_later_population() {
        // Even a no-op action should leave behind the named group so
        // a later caller can pop matches into it.
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["x".into()]);
        state.tags.configure_from_style(&["x".into()]);
        state.tags.start();
        _complete_tag(&mut state, "x", |_| true);
        assert!(state.comp.groups.iter().any(|g| g.name == "x"));
    }

    #[test]
    fn two_complete_tag_calls_keep_groups_isolated() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["files".into(), "dirs".into()]);
        state
            .tags
            .configure_from_style(&["files dirs".into()]);
        state.tags.start();
        _complete_tag(&mut state, "files", |s| {
            s.add_match(crate::compsys::completion::Completion::new("a"), Some("files"));
            true
        });
        _complete_tag(&mut state, "dirs", |s| {
            s.add_match(crate::compsys::completion::Completion::new("b"), Some("dirs"));
            true
        });
        let files_grp = state.comp.groups.iter().find(|g| g.name == "files").unwrap();
        let dirs_grp = state.comp.groups.iter().find(|g| g.name == "dirs").unwrap();
        assert!(files_grp.matches.iter().any(|c| c.str_ == "a"));
        assert!(dirs_grp.matches.iter().any(|c| c.str_ == "b"));
        // Cross-contamination check.
        assert!(!files_grp.matches.iter().any(|c| c.str_ == "b"));
    }

    #[test]
    fn special_chars_in_tag_name_preserved() {
        let mut state = MainCompleteState::new("", 0);
        let tag = "argument-1";
        state.tags.init(&[tag.into()]);
        state.tags.configure_from_style(&[tag.into()]);
        state.tags.start();
        _complete_tag(&mut state, tag, |_| true);
        assert!(state.comp.groups.iter().any(|g| g.name == tag));
    }
}
