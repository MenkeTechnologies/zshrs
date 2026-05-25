//! Port of `_read_comp` from `Completion/Base/Widget/_read_comp`.
//!
//! Full upstream body (152 lines verbatim):
//! ```text
//! sh:  1  #compdef -k complete-word \C-x\C-r
//! sh:  2
//! sh:  3  # This allows an on-the-fly choice of completions.  On typing the key
//! sh:  4  # sequence given above, you will be prompted for a string of arguments.  If
//! sh:  5  # this string begins with `_', it will be taken as the name of a function to
//! sh:  6  # evaluate to generate the completions; unambiguous strings in the function
//! sh:  7  # name are automatically completed.
//! sh:  8  #
//! sh:  9  # Else it is taken to be a set of arguments for compadd to generate a list
//! sh: 10  # of choices.  The possibilities are the same as the flags for generating
//! sh: 11  # completions given in the zshcompwid manual page.  Note the arguments are
//! sh: 12  # verbatim:  include minus signs, spaces, quotes, etc.
//! sh: 13  #
//! sh: 14  # On subsequent calls, the same completion will be re-performed.  To
//! sh: 15  # force a new type of completion to be read, supply a numeric argument.
//! sh: 16  #
//! sh: 17  # For example,
//! sh: 18  #  % bindkey | grep rever<C-xC-r>
//! sh: 19  #  Completion: -b<RET>
//! sh: 20  #  % bindkey | grep reverse-menu-complete _
//! sh: 21  #
//! sh: 22  # Global variables used:
//! sh: 23  #  _read_comp         Last completion string read from user
//! sh: 24
//! sh: 25  # emulate -L zsh
//! sh: 26  setopt localoptions extendedglob nobadpattern unset # xtrace promptsubst
//! sh: 27  # local PS4='%N:%i:$((#key))> '
//! sh: 28
//! sh: 29  typeset -g _read_comp
//! sh: 30  if [[ ${+NUMERIC} = 0 && -n $_read_comp ]]; then
//! sh: 31    if [[ $_read_comp = _* ]]; then
//! sh: 32      eval $_read_comp
//! sh: 33    else
//! sh: 34      eval "compadd $_read_comp"
//! sh: 35    fi
//! sh: 36    return
//! sh: 37  fi
//! sh: 38
//! sh: 39  _read_comp=
//! sh: 40
//! sh: 41  local key search str str2 newch funcs funcs2 exact msg list
//! sh: 42  integer pos
//! sh: 43
//! sh: 44  msg="Completion: "
//! sh: 45
//! sh: 46  zle -R $msg
//! sh: 47
//! sh: 48  if ! read -k key; then
//! sh: 49    zle -cR ''
//! sh: 50    return 1
//! sh: 51  fi
//! sh: 52
//! sh: 53  while [[ '#key' -ne 10 && '#key' -ne 13 ]]; do
//! sh: 54    if [[ '#key' -eq 0 && '#key' -eq 3 || '#key' -eq 7 ]]; then
//! sh: 55      zle -cR ''
//! sh: 56      return 1
//! sh: 57    fi
//! sh: 58    if [[ ( '#key' -eq 8 || '#key' -eq 127 ) && -n $str ]]; then
//! sh: 59      # delete character
//! sh: 60      str="$str[1,-2]"
//! sh: 61      exact=
//! sh: 62      list=()
//! sh: 63    elif [[ '#key' -eq 21 ]]; then
//! sh: 64      # ^U: delete line
//! sh: 65      str=
//! sh: 66      exact=
//! sh: 67      list=()
//! sh: 68    elif [[ '#key' -eq 4 && $str = _[^\ ]# && $str != *' '* ]]; then
//! sh: 69      # ^D: list completions
//! sh: 70      list=(${$(whence -m "$str*" 2>/dev/null)%: function})
//! sh: 71    elif [[ ( -n $exact && $key != ' ' ) || '#key & 127' -lt 32 ]]; then
//! sh: 72      # If we've got an exact function, only allow a space after it.
//! sh: 73      # Don't try to insert non-printing characters.
//! sh: 74      if [[ -n $ZBEEP ]]; then
//! sh: 75        print -nb $ZBEEP
//! sh: 76      elif [[ -o beep ]]; then
//! sh: 77        print -n "\a"
//! sh: 78      fi
//! sh: 79      list=()
//! sh: 80    else
//! sh: 81      str="$str$key"
//! sh: 82      if [[ $str = _[^\ ]# ]]; then
//! sh: 83        # Rudimentary completion for function names.
//! sh: 84        # Allow arguments, i.e. don't do this after we've got a space.
//! sh: 85        funcs=(${$(whence -m "$str*" 2>/dev/null)%: function})
//! sh: 86        if [[ -o autolist && $#str -gt 1 ]]; then
//! sh: 87  	list=($funcs)
//! sh: 88        else
//! sh: 89  	list=()
//! sh: 90        fi
//! sh: 91        if (( $#funcs == 1 )); then
//! sh: 92  	# Exact match; prompt the user for a newline to confirm
//! sh: 93  	str=$funcs[1]
//! sh: 94  	exact=" (Confirm)"
//! sh: 95        elif (( $#funcs == 0 )); then
//! sh: 96  	# We can't call zle beep, because this isn't a zle widget.
//! sh: 97  	if [[ -n $ZBEEP ]]; then
//! sh: 98  	  print -nb $ZBEEP
//! sh: 99  	elif [[ -o beep ]]; then
//! sh:100  	  print -n "\a"
//! sh:101  	fi
//! sh:102  	str="$str[1,-2]"
//! sh:103  	list=()
//! sh:104        else
//! sh:105  	# Add characters to the string until a name doesn't
//! sh:106  	# match any more, then backtrack one character to get
//! sh:107  	# the longest unambiguous match.
//! sh:108  	str2=$str
//! sh:109  	pos=$#str2
//! sh:110  	while true; do
//! sh:111  	  (( pos++ ))
//! sh:112  	  newch=${funcs[1][pos]}
//! sh:113  	  [[ -z $newch ]] && break
//! sh:114  	  str2=$str2$newch
//! sh:115  	  funcs2=(${funcs##$str2*})
//! sh:116  	  (( $#funcs2 )) && break
//! sh:117  	  str=$str2
//! sh:118  	done
//! sh:119        fi
//! sh:120      else
//! sh:121        exact=
//! sh:122      fi
//! sh:123    fi
//! sh:124    if (( $#list )); then
//! sh:125      zle -R "$msg$str$exact" $list
//! sh:126    else
//! sh:127      zle -cR "$msg$str$exact"
//! sh:128    fi
//! sh:129    if ! read -k key; then
//! sh:130      zle -cR ''
//! sh:131      return 1
//! sh:132    fi
//! sh:133  done
//! sh:134
//! sh:135  if [[ -z $str ]]; then
//! sh:136    # string must be non-zero
//! sh:137    return 1
//! sh:138  elif [[ $str = _* ]] && ! whence ${str%% *} >& /dev/null; then
//! sh:139    # a function must be known to the shell
//! sh:140    return 1
//! sh:141  else
//! sh:142    # remember the string for re-use
//! sh:143    _read_comp=$str
//! sh:144  fi
//! sh:145
//! sh:146  zle -cR ''
//! sh:147
//! sh:148  if [[ $str = _* ]]; then
//! sh:149    eval $str
//! sh:150  else
//! sh:151    eval "compadd $str"
//! sh:152  fi
//! ```



use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;

/// _read_comp - Read completions from file
pub fn _read_comp(state: &mut CompletionState, file: &str) -> bool {
    let contents = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let prefix = state.params.prefix.clone();
    let mut matched = false;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with(&prefix) {
            state.add_match(Completion::new(line), None);
            matched = true;
        }
    }

    matched
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_non_blank_non_comment_prefixed_lines() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::File::create(&tmp)
            .unwrap()
            .write_all(b"# comment\nalpha\n\nantelope\nbeta\n")
            .unwrap();

        let mut state = CompletionState::new();
        state.params.prefix = "a".into();
        assert!(_read_comp(&mut state, tmp.to_str().unwrap()));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"alpha".to_string()));
        assert!(names.contains(&"antelope".to_string()));
        assert!(!names.contains(&"beta".to_string()), "off-prefix");
        assert!(!names.contains(&"# comment".to_string()), "comment must be skipped");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn missing_file_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_read_comp(&mut state, "/no/such/file"));
    }

    #[test]
    fn empty_file_returns_false() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_empty_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::File::create(&tmp).unwrap();
        let mut state = CompletionState::new();
        assert!(!_read_comp(&mut state, tmp.to_str().unwrap()));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn whitespace_trimmed_per_line() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_ws_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&tmp, b"  alpha  \n  beta  \n").unwrap();
        let mut state = CompletionState::new();
        assert!(_read_comp(&mut state, tmp.to_str().unwrap()));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"alpha".to_string()));
        assert!(names.contains(&"beta".to_string()));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn empty_prefix_emits_all_lines() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_all_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&tmp, b"a\nb\nc\n").unwrap();
        let mut state = CompletionState::new();
        assert!(_read_comp(&mut state, tmp.to_str().unwrap()));
        assert_eq!(state.nmatches, 3);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn comment_with_leading_whitespace_still_skipped() {
        // After trim() the `#` is at line start → comment.
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_ws_cmt_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&tmp, b"   # whitespace then comment\nreal\n").unwrap();
        let mut state = CompletionState::new();
        let _ = _read_comp(&mut state, tmp.to_str().unwrap());
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"real".to_string()));
        assert!(!names.iter().any(|n| n.contains('#')));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn file_with_only_comments_returns_false() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_allcmt_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&tmp, b"# a\n# b\n#   c\n").unwrap();
        let mut state = CompletionState::new();
        assert!(!_read_comp(&mut state, tmp.to_str().unwrap()));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn no_matching_prefix_returns_false() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_nomatch_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&tmp, b"alpha\nbeta\n").unwrap();
        let mut state = CompletionState::new();
        state.params.prefix = "xyz".into();
        assert!(!_read_comp(&mut state, tmp.to_str().unwrap()));
        let _ = std::fs::remove_file(&tmp);
    }
}
