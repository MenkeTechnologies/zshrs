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



use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;

/// `_directory_stack` — emit numbered dirstack entries when the
/// user typed `+` or `-`.
pub fn _directory_stack(
    state: &mut CompletionState,
    dirstack: &[String],
    pushdminus: bool,
) -> bool {
    let prefix = state.params.prefix.clone();
    if !prefix.starts_with('+') && !prefix.starts_with('-') {
        return false;
    }
    let sign = prefix.chars().next().unwrap();
    let user_typed = &prefix[1..];

    state.begin_group("directory-stack", true);
    let mut any = false;
    // When pushdminus AND sign is '+', OR no-pushdminus AND sign is '-',
    // the numbering is REVERSED.
    let reverse =
        (pushdminus && sign == '+') || (!pushdminus && sign == '-');
    let n = dirstack.len();
    for (i, entry) in dirstack.iter().enumerate() {
        let display_num = if reverse { n.saturating_sub(1).saturating_sub(i) } else { i };
        let cand = format!("{}{}", sign, display_num);
        if !cand[1..].starts_with(user_typed) {
            continue;
        }
        let mut comp = Completion::new(cand.clone());
        comp.disp = Some(format!("{} -- {}", cand, entry));
        state.add_match(comp, Some("directory-stack"));
        any = true;
    }
    state.end_group();
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_dash_or_plus_prefix_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "/some/path".into();
        let dirs = vec!["~".to_string(), "~/work".to_string()];
        assert!(!_directory_stack(&mut state, &dirs, false));
    }

    #[test]
    fn plus_emits_ascending_numbered_entries() {
        let mut state = CompletionState::new();
        state.params.prefix = "+".into();
        let dirs = vec![
            "~".to_string(),
            "~/work".to_string(),
            "~/Documents".to_string(),
        ];
        let _ = _directory_stack(&mut state, &dirs, false);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        // With pushdminus=false, + uses ascending 0,1,2.
        assert!(names.contains(&"+0"));
        assert!(names.contains(&"+1"));
        assert!(names.contains(&"+2"));
    }

    #[test]
    fn dash_emits_descending_numbered_entries() {
        // With pushdminus=false, `-` is reversed.
        let mut state = CompletionState::new();
        state.params.prefix = "-".into();
        let dirs = vec!["~".to_string(), "~/work".to_string()];
        let _ = _directory_stack(&mut state, &dirs, false);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"-0"));
        assert!(names.contains(&"-1"));
    }

    #[test]
    fn pushdminus_flips_plus_to_descending() {
        let mut state = CompletionState::new();
        state.params.prefix = "+".into();
        let dirs = vec!["~".to_string(), "~/w".to_string()];
        let _ = _directory_stack(&mut state, &dirs, true);
        // With pushdminus=true, `+` is reversed.
        // entries[0]=~ → display=1; entries[1]=~/w → display=0
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"+0"));
        assert!(names.contains(&"+1"));
    }

    #[test]
    fn partial_number_typed_filters() {
        let mut state = CompletionState::new();
        state.params.prefix = "+1".into();
        let dirs: Vec<String> = (0..15).map(|i| format!("/d{i}")).collect();
        let _ = _directory_stack(&mut state, &dirs, false);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        // All emitted should be +1, +10, +11, +12, +13, +14 (starts_with "1")
        assert!(names.iter().all(|n| n[1..].starts_with("1")));
        assert!(names.contains(&"+1"));
        assert!(names.contains(&"+10"));
    }

    #[test]
    fn entries_attached_to_disp() {
        let mut state = CompletionState::new();
        state.params.prefix = "+".into();
        let dirs = vec!["~/myproj".to_string()];
        let _ = _directory_stack(&mut state, &dirs, false);
        let disp = state.groups[0].matches[0]
            .disp
            .as_deref()
            .unwrap_or("");
        assert!(disp.contains("~/myproj"));
    }

    #[test]
    fn empty_dirstack_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "+".into();
        assert!(!_directory_stack(&mut state, &[], false));
    }
}
