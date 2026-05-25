//! Port of `_bash_completions` from `Completion/Base/Widget/_bash_completions`.
//!
//! Full upstream body (46 lines verbatim):
//! ```text
//! sh: 1  #compdef -K _bash_complete-word complete-word \e~ _bash_list-choices list-choices ^X~
//! sh: 2  #
//! sh: 3  # This function is for bash compatibility.  As some of the bash bindings
//! sh: 4  # are already taken up in zsh, only Esc ~ and \C-x ~ are bound, and
//! sh: 5  # you must add the rest by hand.  The bindings expected are:
//! sh: 6  #
//! sh: 7  # Esc ! -> command name
//! sh: 8  # Esc $ -> environment variables
//! sh: 9  # Esc @ -> machine names
//! sh:10  # Esc / -> file name
//! sh:11  # Esc ~ -> a user name
//! sh:12  #
//! sh:13  # C-x instead of Esc with one of the above will list matches and won't
//! sh:14  # attempt any completion.
//! sh:15  #
//! sh:16  # The following will bind the remaining set; simply put it in .zshrc
//! sh:17  # after compinit is run.
//! sh:18  #
//! sh:19  # for key in '!' '$' '@' '/'; do
//! sh:20  #   bindkey "\e$key" _bash_complete-word
//! sh:21  #   bindkey "^X$key" _bash_list-choices
//! sh:22  # done
//! sh:23  #
//! sh:24  # If for some reason \e~ or ^X~ were already bound to something else,
//! sh:25  # that will not have been overridden, so you should add '~' to the
//! sh:26  # list of keys at the top of the for-loop.
//! sh:27
//! sh:28  eval "$_comp_setup"
//! sh:29
//! sh:30  local key=$KEYS[-1] expl
//! sh:31
//! sh:32  case $key in
//! sh:33    '!') _main_complete _command_names
//! sh:34         ;;
//! sh:35    '$') _main_complete - parameters _wanted parameters expl 'exported parameter' \
//! sh:36                                         _parameters -g '*export*'
//! sh:37         ;;
//! sh:38    '@') _main_complete _hosts
//! sh:39         ;;
//! sh:40    '/') _main_complete _files
//! sh:41         ;;
//! sh:42    '~') _main_complete _users
//! sh:43         ;;
//! sh:44    *) _message "Key $key is not understood"
//! sh:45       ;;
//! sh:46  esac
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
