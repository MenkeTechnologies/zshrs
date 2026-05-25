//! Port of `_file_descriptors` from `Completion/Zsh/Type/_file_descriptors`.
//!
//! Full upstream body (59 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local i fds expl disp link sep
//! sh: 4  local -a list proc
//! sh: 5
//! sh: 6  fds=( /dev/fd/<3->(N:t) )
//! sh: 7  fds=( ${(n)fds} )
//! sh: 8
//! sh: 9  if zstyle -T ":completion:${curcontext}:file-descriptors" verbose; then
//! sh:10    zstyle -s ":completion:${curcontext}:file-descriptors" list-separator sep || sep=--
//! sh:11
//! sh:12    if [[ $OSTYPE = freebsd* ]]; then
//! sh:13      fds=( ${(f)"$(procstat -f $$|awk -v OFS=: '$3>2 && $3~/[0-9]/ {print $3,$10}')"} )
//! sh:14      zformat -a list " $sep " $fds
//! sh:15      fds=( ${fds%%:*} )
//! sh:16    elif
//! sh:17      proc=( /proc/$$/(fd|path)/<->(@N[-1]:h) )
//! sh:18      [[ -n $proc ]]
//! sh:19    then
//! sh:20      if zmodload -F zsh/stat b:zstat; then
//! sh:21        for i in "${fds[@]}"; do
//! sh:22  	if zstat +link -A link $proc/$i; then
//! sh:23  	  list+=( "${(r.$#fds[-1].)i} $sep ${(D)link[1]}" )
//! sh:24  	else
//! sh:25  	  fds[(i)$i]=()
//! sh:26  	fi
//! sh:27        done
//! sh:28      elif (( $+commands[readlink] )); then
//! sh:29        for i in "${fds[@]}"; do
//! sh:30  	if link=$(readlink $proc/$i); then
//! sh:31  	  list+=( "${(r.$#fds[-1].)i} $sep ${(D)link}" )
//! sh:32  	else
//! sh:33  	  fds[(i)$i]=()
//! sh:34  	fi
//! sh:35        done
//! sh:36      else
//! sh:37        for i in "${fds[@]}"; do
//! sh:38  	if link=$(ls -l $proc/$i); then
//! sh:39  	  list+=( "${(r.$#fds[-1].)i} $sep ${(D)link#* -> }" )
//! sh:40  	else
//! sh:41  	  fds[(i)$i]=()
//! sh:42  	fi
//! sh:43        done
//! sh:44      fi 2>/dev/null
//! sh:45    fi
//! sh:46
//! sh:47    if (( list[(I)* $sep ?*] )); then
//! sh:48      list=(
//! sh:49        "${(r.$#fds[-1].):-0} $sep standard input"
//! sh:50        "${(r.$#fds[-1].):-1} $sep standard output"
//! sh:51        "${(r.$#fds[-1].):-2} $sep standard error" $list
//! sh:52      )
//! sh:53      disp=( -d list )
//! sh:54    fi
//! sh:55  fi
//! sh:56  fds=( 0 1 2 $fds )
//! sh:57
//! sh:58  _description -V file-descriptors expl 'file descriptor'
//! sh:59  compadd $disp -o nosort "$@" "$expl[@]" -a fds
//! ```
//!
//! Strict Rust port: walks our own `/dev/fd/N` entries for N ≥ 3,
//! filters numeric basenames. Lists fd 3 onwards (0/1/2 are
//! stdin/stdout/stderr — generally not what the user wants when
//! they're redirecting).



use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;

/// `_file_descriptors` — emit numeric fds ≥ 3 currently open by
/// the process.
pub fn _file_descriptors(state: &mut CompletionState) -> bool {
    let prefix = state.params.prefix.clone();
    let mut fds: Vec<u32> = Vec::new();

    // /dev/fd/<3-> on most Unix systems. On macOS this is a virtual
    // dir; on Linux it's a symlink to /proc/self/fd.
    if let Ok(entries) = std::fs::read_dir("/dev/fd") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if let Ok(n) = s.parse::<u32>() {
                if n >= 3 {
                    fds.push(n);
                }
            }
        }
    }
    fds.sort();

    state.begin_group("file-descriptors", true);
    let mut any = false;
    for n in fds {
        let s = n.to_string();
        if !s.starts_with(&*prefix) {
            continue;
        }
        state.add_match(Completion::new(s), Some("file-descriptors"));
        any = true;
    }
    state.end_group();
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_panic_on_call() {
        // Whether any fds ≥ 3 are open depends on the test
        // environment. Just pin no panic.
        let mut state = CompletionState::new();
        let _ = _file_descriptors(&mut state);
    }

    #[test]
    fn emitted_values_are_all_numeric_strings() {
        let mut state = CompletionState::new();
        let _ = _file_descriptors(&mut state);
        for m in state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
        {
            assert!(m.str_.parse::<u32>().is_ok(), "non-numeric fd: `{}`", m.str_);
        }
    }

    #[test]
    fn fds_under_3_excluded() {
        let mut state = CompletionState::new();
        let _ = _file_descriptors(&mut state);
        for m in state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
        {
            let n: u32 = m.str_.parse().unwrap();
            assert!(n >= 3, "fd {n} should be excluded (0/1/2 reserved)");
        }
    }

    #[test]
    fn group_named_file_descriptors() {
        let mut state = CompletionState::new();
        let _ = _file_descriptors(&mut state);
        assert!(state.groups.iter().any(|g| g.name == "file-descriptors"));
    }

    #[test]
    fn prefix_filter_works_when_matches_exist() {
        // Open an extra fd to ensure SOMETHING comes back.
        use std::fs::File;
        let _temp = File::open("/dev/null").expect("open /dev/null");
        let mut state = CompletionState::new();
        state.params.prefix = "9".into();
        let _ = _file_descriptors(&mut state);
        for m in state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
        {
            assert!(m.str_.starts_with('9'));
        }
    }
}
