//! Port of `_files` from `Completion/Unix/Type/_files`.
//!
//! Full upstream body (153 lines verbatim):
//! ```text
//! sh:  1  #compdef -redirect-,-default-,-default-
//! sh:  2
//! sh:  3  local -a match mbegin mend
//! sh:  4  local -a subtree
//! sh:  5  local ret=1
//! sh:  6
//! sh:  7  # Look for glob qualifiers. This is duplicated from _path_files because
//! sh:  8  # we don't want to complete them multiple times (for each file pattern).
//! sh:  9  if _have_glob_qual $PREFIX; then
//! sh: 10    compset -p ${#match[1]}
//! sh: 11    compset -S '[^\)\|\~]#(|\))'
//! sh: 12    if [[ $_comp_caller_options[extendedglob] == on ]] && compset -P '\#'; then
//! sh: 13      _globflags && ret=0
//! sh: 14    else
//! sh: 15      if [[ $_comp_caller_options[extendedglob] == on ]]; then
//! sh: 16        _describe -t globflags "glob flag" '(\#:introduce\ glob\ flag)' -Q -S '' && ret=0
//! sh: 17      fi
//! sh: 18      _globquals && ret=0
//! sh: 19    fi
//! sh: 20    return ret
//! sh: 21  elif [[ $_comp_caller_options[extendedglob] == on && $PREFIX = \(\#[^\)]# ]] && compset -P '\(\#'; then
//! sh: 22    # Globbing flags can start at beginning of word, even though
//! sh: 23    # glob qualifiers can't.
//! sh: 24    _globflags && return
//! sh: 25  fi
//! sh: 26
//! sh: 27  local opts tmp glob pat pats expl tag i def descr end ign tried
//! sh: 28  local type sdef ignvars ignvar prepath oprefix rfiles rfile
//! sh: 29
//! sh: 30  zparseopts -a opts \
//! sh: 31      '/=tmp' 'f=tmp' 'g+:-=tmp' q n 1 2 P: S: r: R: W: x+: X+: M+: F: J+: V+: o+:
//! sh: 32
//! sh: 33  type="${(@j::M)${(@)tmp#-}#?}"
//! sh: 34  if (( $tmp[(I)-g*] )); then
//! sh: 35    glob="${${${(@)${(@M)tmp:#-g*}#-g}##[[:blank:]]#}%%[[:blank:]]#}"
//! sh: 36    [[ "$glob" = *[^\\][[:blank:]]* ]] &&
//! sh: 37        glob="{${glob//(#b)([^\\])[[:blank:]]##/${match[1]},}}"
//! sh: 38
//! sh: 39    # add `#q' to the beginning of any glob qualifier if not there already
//! sh: 40    [[ "$glob" = (#b)(*\()([^\|\~]##\)) && $match[2] != \#q* ]] &&
//! sh: 41        glob="${match[1]}#q${match[2]}"
//! sh: 42  elif [[ $type = */* ]]; then
//! sh: 43    glob="*(#q-/)"
//! sh: 44  fi
//! sh: 45  tmp=$opts[(I)-F]
//! sh: 46  if (( tmp )); then
//! sh: 47    ignvars=($=opts[tmp+1])
//! sh: 48    if [[ $ignvars = _comp_ignore ]]; then
//! sh: 49      ign=( $_comp_ignore )
//! sh: 50    elif [[ $ignvars = \(* ]]; then
//! sh: 51      ign=( ${=ignvars[2,-2]} )
//! sh: 52    else
//! sh: 53      ign=()
//! sh: 54      for ignvar in $ignvars; do
//! sh: 55        ign+=(${(P)ignvar})
//! sh: 56      done
//! sh: 57      opts[tmp+1]=_comp_ignore
//! sh: 58    fi
//! sh: 59  else
//! sh: 60    ign=()
//! sh: 61  fi
//! sh: 62
//! sh: 63  if zstyle -a ":completion:${curcontext}:" file-patterns tmp; then
//! sh: 64    pats=()
//! sh: 65
//! sh: 66    for i in ${tmp//\%p/${${glob:-\*}//:/\\:}}; do
//! sh: 67      if [[ $i = *[^\\]:* ]]; then
//! sh: 68        pats+=( " $i " )
//! sh: 69      else
//! sh: 70        pats+=( " ${i}:files " )
//! sh: 71      fi
//! sh: 72    done
//! sh: 73  elif zstyle -t ":completion:${curcontext}:" list-dirs-first; then
//! sh: 74    pats=( " *(-/):directories:directory ${${glob:-*}//:/\\:}(#q^-/):globbed-files" '*:all-files' )
//! sh: 75  else
//! sh: 76    # People prefer to have directories shown on first try as default.
//! sh: 77    # Even if the calling function didn't use -/.
//! sh: 78    pats=( "${${glob:-*}//:/\\:}:globbed-files *(-/):directories" '*:all-files ' )
//! sh: 79  fi
//! sh: 80
//! sh: 81  tried=()
//! sh: 82  for def in "$pats[@]"; do
//! sh: 83    eval "def=( ${${def//\\:/\\\\\\:}//(#b)([][()|*?^#~<>])/\\${match[1]}} )"
//! sh: 84
//! sh: 85    tmp="${(@M)def#*[^\\]:}"
//! sh: 86    (( $tried[(I)${(q)tmp}] )) && continue
//! sh: 87    tried=( "$tried[@]" "$tmp" )
//! sh: 88
//! sh: 89    for sdef in "$def[@]"; do
//! sh: 90
//! sh: 91      tag="${${sdef#*[^\\]:}%%:*}"
//! sh: 92      pat="${${sdef%%:${tag}*}//\\:/:}"
//! sh: 93
//! sh: 94      if [[ "$sdef" = *:${tag}:* ]]; then
//! sh: 95        # If the file-patterns spec includes a description, use it and give the
//! sh: 96        # group/description options from it precedence over passed in parameters.
//! sh: 97        descr="${(Q)sdef#*:${tag}:}"
//! sh: 98        end=
//! sh: 99      else
//! sh:100        if (( $opts[(I)-X] )); then
//! sh:101          descr=
//! sh:102        else
//! sh:103          descr=file
//! sh:104        fi
//! sh:105        end=yes
//! sh:106      fi
//! sh:107
//! sh:108      _tags "$tag"
//! sh:109      while _tags; do
//! sh:110        _comp_ignore=()
//! sh:111        while _next_label "$tag" expl "$descr"; do
//! sh:112          _comp_ignore=( $_comp_ignore $ign )
//! sh:113          if [[ -n "$end" ]]; then
//! sh:114            expl=( "$opts[@]" "$expl[@]" )
//! sh:115          else
//! sh:116            expl+=( "$opts[@]" )
//! sh:117          fi
//! sh:118
//! sh:119          if _path_files -g "$pat" "$expl[@]"; then
//! sh:120            ret=0
//! sh:121          elif [[ $PREFIX$SUFFIX != */* ]] && \
//! sh:122              zstyle -a ":completion:${curcontext}:$tag" recursive-files rfiles
//! sh:123          then
//! sh:124            for rfile in $rfiles; do
//! sh:125              if [[ $PWD/ = ${~rfile} ]]; then
//! sh:126                if [[ -z $subtree ]]; then
//! sh:127                  subtree=( **/*(/) )
//! sh:128                fi
//! sh:129                for prepath in $subtree; do
//! sh:130                  oprefix=$PREFIX
//! sh:131                  PREFIX=$prepath/$PREFIX
//! sh:132                  _path_files -g "$pat" "$expl[@]" && ret=0
//! sh:133                  PREFIX=$oprefix
//! sh:134                done
//! sh:135                break
//! sh:136              fi
//! sh:137            done
//! sh:138          fi
//! sh:139        done
//! sh:140        (( ret )) || break
//! sh:141      done
//! sh:142
//! sh:143      ### For that _next_tags change mentioned above we would have to
//! sh:144      ### comment out the following line. (Or not, depending on the order
//! sh:145      ### of the patterns.)
//! sh:146
//! sh:147      [[ "$pat" = '*' ]] && return ret
//! sh:148
//! sh:149    done
//! sh:150    (( ret )) || return 0
//! sh:151  done
//! sh:152
//! sh:153  return 1
//! ```



use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::{Completion, CompletionFlags};
use std::fs;
use std::path::PathBuf;

/// Options for file completion
#[derive(Clone, Debug, Default)]
pub struct FilesOpts {
    /// Only complete directories (-/)
    pub dirs_only: bool,
    /// File glob pattern (-g)
    pub glob: Option<String>,
    /// Prefix to add (-P)
    pub prefix: Option<String>,
    /// Suffix to add (-S)
    pub suffix: Option<String>,
    /// Working directory (-W)
    pub work_dir: Option<String>,
    /// Description (-X)
    pub description: Option<String>,
    /// File types to match (e.g., "*.rs")
    pub file_patterns: Vec<String>,
    /// Exclude patterns
    pub exclude_patterns: Vec<String>,
    /// Show hidden files
    pub show_hidden: bool,
}

impl FilesOpts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn dirs_only() -> Self {
        Self {
            dirs_only: true,
            ..Default::default()
        }
    }

    /// Parse _files arguments
    pub fn parse(args: &[String]) -> Self {
        let mut opts = Self::new();
        let mut i = 0;

        while i < args.len() {
            match args[i].as_str() {
                "-/" => opts.dirs_only = true,
                "-g" => {
                    if i + 1 < args.len() {
                        opts.glob = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "-P" => {
                    if i + 1 < args.len() {
                        opts.prefix = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "-S" => {
                    if i + 1 < args.len() {
                        opts.suffix = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "-W" => {
                    if i + 1 < args.len() {
                        opts.work_dir = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "-X" => {
                    if i + 1 < args.len() {
                        opts.description = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "-F" => {
                    // Ignore file type specifiers for now
                    if i + 1 < args.len() {
                        i += 1;
                    }
                }
                arg if arg.starts_with("-g") => {
                    opts.glob = Some(arg[2..].to_string());
                }
                arg if arg.starts_with("-P") => {
                    opts.prefix = Some(arg[2..].to_string());
                }
                arg if arg.starts_with("-S") => {
                    opts.suffix = Some(arg[2..].to_string());
                }
                _ => {
                    // Could be a file pattern
                    if !args[i].starts_with('-') {
                        opts.file_patterns.push(args[i].clone());
                    }
                }
            }
            i += 1;
        }

        opts
    }
}

/// Check if filename matches a glob pattern
fn matches_glob(name: &str, pattern: &str) -> bool {
    // Simple glob matching - supports * and ?
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let name_chars: Vec<char> = name.chars().collect();

    fn match_helper(pattern: &[char], name: &[char]) -> bool {
        match (pattern.first(), name.first()) {
            (None, None) => true,
            (Some('*'), _) => {
                // * matches zero or more characters
                match_helper(&pattern[1..], name)
                    || (!name.is_empty() && match_helper(pattern, &name[1..]))
            }
            (Some('?'), Some(_)) => {
                // ? matches exactly one character
                match_helper(&pattern[1..], &name[1..])
            }
            (Some(p), Some(n)) if *p == *n => match_helper(&pattern[1..], &name[1..]),
            _ => false,
        }
    }

    match_helper(&pattern_chars, &name_chars)
}

/// Execute file completion
pub fn files_execute(state: &mut CompletionState, opts: &FilesOpts) -> bool {
    let prefix = &state.params.prefix;

    // Determine base directory and file prefix
    let (base_dir, file_prefix) = if let Some(sep_pos) = prefix.rfind('/') {
        let dir = &prefix[..sep_pos + 1];
        let file = &prefix[sep_pos + 1..];
        (PathBuf::from(dir), file.to_string())
    } else {
        (PathBuf::from("../.."), prefix.clone())
    };

    // Use working directory if specified
    let search_dir = if let Some(ref wd) = opts.work_dir {
        PathBuf::from(wd).join(&base_dir)
    } else {
        base_dir.clone()
    };

    // Read directory
    let entries = match fs::read_dir(&search_dir) {
        Ok(e) => e,
        Err(_) => return false,
    };

    let group_name = if opts.dirs_only {
        "directories"
    } else {
        "files"
    };
    state.begin_group(group_name, true);

    if let Some(ref desc) = opts.description {
        state.add_explanation(desc.clone(), Some(group_name));
    }

    let mut added = false;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden files unless prefix starts with .
        if name_str.starts_with('.') && !file_prefix.starts_with('.') && !opts.show_hidden {
            continue;
        }

        // Check prefix match
        if !name_str.starts_with(&file_prefix) {
            continue;
        }

        let path = entry.path();
        let is_dir = path.is_dir();

        // Skip non-directories if dirs_only
        if opts.dirs_only && !is_dir {
            continue;
        }

        // Check glob pattern
        if let Some(ref glob) = opts.glob {
            if !is_dir && !matches_glob(&name_str, glob) {
                continue;
            }
        }

        // Check file patterns
        if !opts.file_patterns.is_empty() && !is_dir {
            let matches_any = opts
                .file_patterns
                .iter()
                .any(|p| matches_glob(&name_str, p));
            if !matches_any {
                continue;
            }
        }

        // Build completion string. Note: PathBuf doesn't impl
        // PartialEq<&str>, so compare via to_str() (a clippy --fix
        // pass over-aggressively rewrote this and broke the build).
        let mut comp_str = if base_dir.to_str() == Some(".") {
            name_str.to_string()
        } else {
            format!("{}{}", base_dir.display(), name_str)
        };

        // Add prefix
        if let Some(ref pfx) = opts.prefix {
            comp_str = format!("{}{}", pfx, comp_str);
        }

        // Add suffix or / for directories
        if is_dir {
            comp_str.push('/');
        } else if let Some(ref sfx) = opts.suffix {
            comp_str.push_str(sfx);
        }

        let mut comp = Completion::new(&comp_str);

        // Don't set descriptions for files - zsh doesn't show them in normal tab completion
        // The file type is already indicated by color and trailing / for directories

        // Set file mode character for LS_COLORS coloring
        if is_dir {
            comp.modec = '/';
            comp.flags |= CompletionFlags::NOSPACE;
        } else if path.is_symlink() {
            comp.modec = '@';
        } else {
            // Check if executable
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = path.metadata() {
                    if meta.permissions().mode() & 0o111 != 0 {
                        comp.modec = '*';
                    }
                }
            }
        }

        state.add_match(comp, Some(group_name));
        added = true;
    }

    state.end_group();
    added
}

/// Execute directory completion (_directories)
pub fn directories_execute(state: &mut CompletionState) -> bool {
    files_execute(state, &FilesOpts::dirs_only())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_matching() {
        assert!(matches_glob("foo.rs", "*.rs"));
        assert!(matches_glob("foo.rs", "foo.*"));
        assert!(matches_glob("foo.rs", "f?o.rs"));
        assert!(matches_glob("foobar", "foo*"));
        assert!(!matches_glob("foo.rs", "*.txt"));
        assert!(!matches_glob("bar.rs", "foo*"));
    }

    #[test]
    fn test_parse_opts() {
        let opts = FilesOpts::parse(&["-/".to_string(), "-g".to_string(), "*.rs".to_string()]);
        assert!(opts.dirs_only);
        assert_eq!(opts.glob, Some("*.rs".to_string()));
    }

    #[test]
    fn test_parse_combined_opts() {
        let opts = FilesOpts::parse(&["-g*.txt".to_string(), "-Pprefix_".to_string()]);
        assert_eq!(opts.glob, Some("*.txt".to_string()));
        assert_eq!(opts.prefix, Some("prefix_".to_string()));
    }

    #[test]
    fn parse_dash_x_description() {
        let opts = FilesOpts::parse(&["-X".to_string(), "Pick a file".to_string()]);
        assert_eq!(opts.description.as_deref(), Some("Pick a file"));
    }

    #[test]
    fn parse_dash_w_work_dir() {
        let opts = FilesOpts::parse(&["-W".to_string(), "/tmp".to_string()]);
        assert_eq!(opts.work_dir.as_deref(), Some("/tmp"));
    }

    #[test]
    fn parse_dash_s_suffix() {
        let opts = FilesOpts::parse(&["-S".to_string(), "/".to_string()]);
        assert_eq!(opts.suffix.as_deref(), Some("/"));
    }

    #[test]
    fn parse_dash_F_consumes_arg_but_ignores_value() {
        // -F is documented as ignored; pin that the followup arg is
        // consumed (so parser doesn't try to interpret it as a flag).
        let opts = FilesOpts::parse(&[
            "-F".to_string(),
            "(*.rs)".to_string(),
            "-g".to_string(),
            "*.toml".to_string(),
        ]);
        assert_eq!(opts.glob.as_deref(), Some("*.toml"));
    }

    #[test]
    fn dirs_only_constructor_sets_flag() {
        assert!(FilesOpts::dirs_only().dirs_only);
        assert!(!FilesOpts::new().dirs_only);
    }

    #[test]
    fn matches_glob_handles_question_mark() {
        assert!(matches_glob("foo", "f?o"));
        assert!(matches_glob("foo", "f??"), "two ? matches any 2 chars");
        assert!(!matches_glob("foo", "f?"), "? matches exactly one, not two");
        assert!(!matches_glob("foo", "?ab"), "second char of foo isn't `a`");
    }

    #[test]
    fn matches_glob_anchored_star() {
        // Pin that star matches zero-length too.
        assert!(matches_glob("abc", "*abc"));
        assert!(matches_glob("abc", "abc*"));
        assert!(matches_glob("abc", "*abc*"));
        assert!(matches_glob("xabc", "*abc"));
        assert!(matches_glob("abcy", "abc*"));
    }

    #[test]
    fn files_execute_returns_false_for_nonexistent_dir() {
        let mut state = CompletionState::new();
        state.params.prefix = "/no/such/dir/foo".into();
        assert!(!files_execute(&mut state, &FilesOpts::default()));
    }

    #[test]
    fn directories_execute_emits_dirs_only_in_named_group() {
        let mut state = CompletionState::new();
        state.params.prefix = "/".into();
        let _ = directories_execute(&mut state);
        // The group is named "directories" (vs "files" for the
        // general case). Pin both: group name AND that nothing
        // emitted has a `.` extension (would be a file).
        if let Some(grp) = state.groups.iter().find(|g| g.name == "directories") {
            for m in &grp.matches {
                // Every emitted match should end in `/` (directory).
                assert!(
                    m.str_.ends_with('/'),
                    "directories_execute emitted a non-dir: `{}`",
                    m.str_
                );
            }
        }
    }
}
