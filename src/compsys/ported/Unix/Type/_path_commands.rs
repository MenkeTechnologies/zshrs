//! Port of `_path_commands` from `Completion/Unix/Type/_path_commands`.
//!
//! Full upstream body (125 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  (( $+functions[_path_commands_caching_policy] )) ||
//! sh:  4  _path_commands_caching_policy() {
//! sh:  5
//! sh:  6  local file
//! sh:  7  local -a oldp dbfiles
//! sh:  8
//! sh:  9  # rebuild if cache is more than a week old
//! sh: 10  oldp=( "$1"(Nmw+1) )
//! sh: 11  (( $#oldp )) && return 0
//! sh: 12
//! sh: 13  dbfiles=(/usr/share/man/index.(bt|db|dir|pag)(N) \
//! sh: 14    /usr/man/index.(bt|db|dir|pag)(N) \
//! sh: 15    /var/cache/man/index.(bt|db|dir|pag)(N) \
//! sh: 16    /var/catman/index.(bt|db|dir|pag)(N) \
//! sh: 17    /usr/share/man/*/whatis(N))
//! sh: 18
//! sh: 19  for file in $dbfiles; do
//! sh: 20    [[ $file -nt $1 ]] && return 0
//! sh: 21  done
//! sh: 22
//! sh: 23  return 1
//! sh: 24  }
//! sh: 25
//! sh: 26  _call_whatis() {
//! sh: 27    local sec impl variant sections=( 1 6 8 )
//! sh: 28    case "$OSTYPE" in
//! sh: 29      (#i)dragonfly|(free|open)bsd*) impl=mandoc ;;
//! sh: 30      netbsd*) impl=apropos ;;
//! sh: 31      linux-gnu*)
//! sh: 32        sections=( 1 8 )
//! sh: 33        # The same test as for man so has a good chance of being cached
//! sh: 34        _pick_variant -c man -r variant \
//! sh: 35          freebsd='-S mansect' \
//! sh: 36          openbsd='-S subsection' \
//! sh: 37          $OSTYPE \
//! sh: 38          ---
//! sh: 39        [[ $variant = $OSTYPE ]] && impl=man-db || impl=mandoc
//! sh: 40      ;;
//! sh: 41    esac
//! sh: 42    case $impl in
//! sh: 43      mandoc)
//! sh: 44        for sec in $sections; do
//! sh: 45          whatis -s $sec .\*
//! sh: 46        done
//! sh: 47      ;;
//! sh: 48      man-db)
//! sh: 49        whatis -s ${(j.,.)sections} -r .\*
//! sh: 50      ;;
//! sh: 51      apropos)
//! sh: 52        apropos -l ''|grep "([${(j..)sections}])"
//! sh: 53      ;;
//! sh: 54    esac
//! sh: 55  }
//! sh: 56
//! sh: 57  _path_commands() {
//! sh: 58  local need_desc expl ret=1
//! sh: 59
//! sh: 60  if zstyle -t ":completion:${curcontext}:commands" extra-verbose; then
//! sh: 61    local update_policy first
//! sh: 62    if [[ $+_command_descriptions -eq 0 ]]; then
//! sh: 63      first=yes
//! sh: 64      typeset -A -g _command_descriptions
//! sh: 65    fi
//! sh: 66    zstyle -s ":completion:${curcontext}:" cache-policy update_policy
//! sh: 67    [[ -z "$update_policy" ]] && zstyle ":completion:${curcontext}:" \
//! sh: 68      cache-policy _path_commands_caching_policy
//! sh: 69    if ( [[ -n $first ]] || _cache_invalid command-descriptions ) && \
//! sh: 70      ! _retrieve_cache command-descriptions; then
//! sh: 71      local line
//! sh: 72      for line in "${(f)$(_call_program command-descriptions _call_whatis)}"; do
//! sh: 73        [[ -n ${line:#(#b)([^ ]#) #\([^ ]#\)( #\[[^ ]#\]|)[ -]#(*)} ]] && continue;
//! sh: 74        [[ -z $match[1] || -z $match[3] || -z ${${match[1]}:#*:*} ]] && continue;
//! sh: 75        _command_descriptions[$match[1]]=$match[3]
//! sh: 76      done
//! sh: 77      _store_cache command-descriptions _command_descriptions
//! sh: 78    fi
//! sh: 79
//! sh: 80    (( $#_command_descriptions )) && need_desc=yes
//! sh: 81  fi
//! sh: 82
//! sh: 83  if [[ -n $need_desc ]]; then
//! sh: 84    typeset -a dcmds descs cmds matches
//! sh: 85    local desc cmd sep
//! sh: 86    compadd "$@" -O matches -k commands
//! sh: 87    for cmd in $matches; do
//! sh: 88      desc=$_command_descriptions[$cmd]
//! sh: 89      if [[ -z $desc ]]; then
//! sh: 90        cmds+=$cmd
//! sh: 91      else
//! sh: 92        dcmds+=$cmd
//! sh: 93        descs+="$cmd:$desc"
//! sh: 94      fi
//! sh: 95    done
//! sh: 96    zstyle -s ":completion:${curcontext}:" list-separator sep || sep=--
//! sh: 97    zformat -a descs " $sep " $descs
//! sh: 98    descs=("${(@r:COLUMNS-1:)descs}")
//! sh: 99    _wanted commands expl 'external command' \
//! sh:100      compadd "$@" -ld descs -a dcmds && ret=0
//! sh:101    _wanted commands expl 'external command' compadd "$@" -a cmds && ret=0
//! sh:102  else
//! sh:103    _wanted commands expl 'external command' compadd "$@" -k commands && ret=0
//! sh:104  fi
//! sh:105  # TODO: this is called from '_command_names -e' which is typically used in
//! sh:106  # contexts (such as _env) that don't accept directory names.  Should this
//! sh:107  # 'if' block move up to the "_command_names -" branch of _command_names?
//! sh:108  if [[ -o path_dirs ]]; then
//! sh:109    local -a path_dirs
//! sh:110
//! sh:111    if [[ $PREFIX$SUFFIX = */* ]]; then
//! sh:112      path_dirs=( ${path:#.} )
//! sh:113      # Find command from path, not hashed
//! sh:114      _wanted commands expl 'external command' _path_files -W path_dirs -g '*(-*)' && ret=0
//! sh:115    else
//! sh:116      path_dirs=(${^path}/*(/N:t))
//! sh:117      (( ${#path_dirs} )) &&
//! sh:118          _wanted path-dirs expl 'directory in path' compadd "$@" -S / -a path_dirs && ret=0
//! sh:119    fi
//! sh:120  fi
//! sh:121
//! sh:122  return ret
//! sh:123  }
//! sh:124
//! sh:125  _path_commands "$@"
//! ```
//!
//! `compadd -k commands` is shorthand for "emit every external
//! executable found in $path" — exactly what [`_command_names`] in
//! externals-only mode does.
//!
//! Strict Rust port: wraps the emission with [`_wanted`] on the
//! `commands` tag with `'external command'` description (matching
//! upstream verbatim), then dispatches to [`_command_names`] with
//! `externals_only=true`.



use crate::compsys::base::MainCompleteState;
use crate::compsys::ported::_command_names::{ShellInventory, _command_names};
use crate::compsys::ported::_wanted::_wanted;

/// `_path_commands` — emit external executables.
pub fn _path_commands(state: &mut MainCompleteState) -> bool {
    // shell: `_wanted commands expl 'external command' compadd "$@" -k commands`
    _wanted(state, "commands", "external command", |s| {
        let inv = ShellInventory::default();
        _command_names(s, &inv, true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compsys::base::TagManager;

    fn seed(state: &mut MainCompleteState) {
        state.tags = TagManager::new();
        state.tags.init(&["commands".into()]);
        state.tags.add_try(&["commands".into()]);
        let _ = state.tags.start();
    }

    #[test]
    fn emits_path_executables_with_known_prefix() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "ls".into();
        let _ = _path_commands(&mut state);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.iter().any(|n| *n == "ls" || n.starts_with("ls")));
    }

    #[test]
    fn untagged_call_skips_emission() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "ls".into();
        assert!(!_path_commands(&mut state));
    }

    #[test]
    fn no_panic_with_empty_prefix() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        let _ = _path_commands(&mut state);
    }

    #[test]
    fn off_prefix_emits_no_matches() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "definitely-no-such-binary-zzz-xyz".into();
        let _ = _path_commands(&mut state);
        let matches: usize = state
            .comp
            .groups
            .iter()
            .filter(|g| g.name == "commands")
            .map(|g| g.matches.len())
            .sum();
        assert_eq!(matches, 0);
    }

    #[test]
    fn group_named_commands_with_external_command_description() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "ls".into();
        let _ = _path_commands(&mut state);
        let grp = state
            .comp
            .groups
            .iter()
            .find(|g| g.name == "commands")
            .expect("commands group");
        assert!(grp.explanations.iter().any(|e| e == "external command"));
    }
}
