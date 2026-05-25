//! Port of `_condition` from `Completion/Zsh/Context/_condition`.
//!
//! Full upstream body (60 lines verbatim):
//! ```text
//! sh: 1  #compdef -condition-
//! sh: 2
//! sh: 3  local prev="$words[CURRENT-1]" ret=1
//! sh: 4
//! sh: 5  if [[ "$prev" = -o ]]; then
//! sh: 6    _tags -C -o options && _options
//! sh: 7  elif [[ "$prev" = -([a-hkprsuwxLOGSN]|[no]t|ef) ]]; then
//! sh: 8    _tags -C "$prev" files && _files
//! sh: 9  elif [[ "$prev" = -t ]]; then
//! sh:10    _file_descriptors
//! sh:11  elif [[ "$prev" = -v ]]; then
//! sh:12    _parameters -r "\= \t\n\[\-"
//! sh:13  else
//! sh:14    if [[ "$PREFIX" = -* ]] ||
//! sh:15       ! zstyle -T ":completion:${curcontext}:options" prefix-needed; then
//! sh:16
//! sh:17      if [[ "$prev" = (\[\[|\|\||\&\&|\!|\() ]]; then
//! sh:18        _describe -o 'condition code' \
//! sh:19                  '( -a:existing\ file
//! sh:20  	           -b:block\ special\ file
//! sh:21  	           -c:character\ special\ file
//! sh:22  	           -d:directory
//! sh:23  	           -e:existing\ file
//! sh:24  	           -f:regular\ file
//! sh:25  	           -g:setgid\ bit
//! sh:26  	           -h:symbolic\ link
//! sh:27  	           -k:sticky\ bit
//! sh:28  	           -n:non-empty\ string
//! sh:29  	           -o:option
//! sh:30  	           -p:named\ pipe
//! sh:31  	           -r:readable\ file
//! sh:32  	           -s:non-empty\ file
//! sh:33  	           -t:terminal\ file\ descriptor
//! sh:34  	           -u:setuid\ bit
//! sh:35  		   -v:set\ variable
//! sh:36  	           -w:writable\ file
//! sh:37  	           -x:executable\ file
//! sh:38  	           -z:empty\ string
//! sh:39  	           -L:symbolic\ link
//! sh:40  	           -O:own\ file
//! sh:41  	           -G:group-owned\ file
//! sh:42  	           -S:socket
//! sh:43  	           -N:unread\ file)' && ret=0
//! sh:44      else
//! sh:45        _describe -o 'condition code' \
//! sh:46  	        '( -nt:newer\ than
//! sh:47  	           -ot:older\ than
//! sh:48  	           -ef:same\ file
//! sh:49  	           -eq:numerically\ equal
//! sh:50  	           -ne:numerically\ not\ equal
//! sh:51  	           -lt:numerically\ less\ than
//! sh:52  	           -le:numerically\ less\ than\ or\ equal
//! sh:53  	           -gt:numerically\ greater\ than
//! sh:54  	           -ge:numerically\ greater\ than\ or\ equal)' && ret=0
//! sh:55      fi
//! sh:56    fi
//! sh:57    _alternative 'files:: _files' 'parameters:: _parameters' && ret=0
//! sh:58
//! sh:59    return ret
//! sh:60  fi
//! ```
//!
//! Strict Rust port: faithful dispatch based on `prev` (the
//! previous word on the line). Operators get their right-hand
//! side completed per the upstream branch table. Caller supplies
//! file/options/parameter handlers since `_options` /
//! `_file_descriptors` / `_parameters` need data injection.



use std::collections::HashMap;

use crate::compsys::base::MainCompleteState;
use crate::compsys::ported::_file_descriptors::_file_descriptors;
use crate::compsys::ported::_options::_options;
use crate::compsys::ported::_parameters::_parameters;

/// `_condition` — `-condition-` context dispatcher.
pub fn _condition(
    state: &mut MainCompleteState,
    shell_options: &[(&str, bool)],
    params: &HashMap<String, String>,
    files_completer: impl FnOnce(&mut MainCompleteState) -> bool,
) -> bool {
    let prev = if state.comp.params.current >= 1 {
        let idx = (state.comp.params.current - 1) as usize;
        state.comp.params.words.get(idx).cloned().unwrap_or_default()
    } else {
        String::new()
    };

    match prev.as_str() {
        // shell:5 — `-o` → options
        "-o" => _options(&mut state.comp, shell_options),
        // shell:9 — `-t` → file descriptors
        "-t" => _file_descriptors(&mut state.comp),
        // shell:11 — `-v` → parameters
        "-v" => _parameters(&mut state.comp, params),
        // shell:7 — file-test operators → files
        p if matches!(
            p,
            "-a" | "-b"
                | "-c"
                | "-d"
                | "-e"
                | "-f"
                | "-g"
                | "-h"
                | "-k"
                | "-p"
                | "-r"
                | "-s"
                | "-u"
                | "-w"
                | "-x"
                | "-L"
                | "-O"
                | "-G"
                | "-S"
                | "-N"
                | "-nt"
                | "-ot"
                | "-ef"
        ) =>
        {
            files_completer(state)
        }
        // Default — operator suggestions are caller-handled.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nofiles(_: &mut MainCompleteState) -> bool {
        panic!("files_completer must not run for this test");
    }

    #[test]
    fn dash_o_dispatches_to_options() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["[[".into(), "-o".into()]; // prev = "-o"
        let opts: Vec<(&str, bool)> = vec![("EXTENDED_GLOB", true)];
        let _ = _condition(&mut state, &opts, &HashMap::new(), nofiles);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"EXTENDED_GLOB"));
    }

    #[test]
    fn dash_v_dispatches_to_parameters() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["[[".into(), "-v".into()];
        let mut p = HashMap::new();
        p.insert("X".into(), "scalar".into());
        let _ = _condition(&mut state, &[], &p, nofiles);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"X"));
    }

    #[test]
    fn dash_f_dispatches_to_files_completer() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["[[".into(), "-f".into()];
        let fired = std::cell::Cell::new(false);
        let _ = _condition(&mut state, &[], &HashMap::new(), |_| {
            fired.set(true);
            true
        });
        assert!(fired.get());
    }

    #[test]
    fn dash_t_dispatches_to_file_descriptors() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["[[".into(), "-t".into()];
        // _file_descriptors may emit nothing in sandboxed env; pin
        // no panic + correct group label.
        let _ = _condition(&mut state, &[], &HashMap::new(), nofiles);
        // Group is created even if no fds open.
        assert!(state
            .comp
            .groups
            .iter()
            .any(|g| g.name == "file-descriptors"));
    }

    #[test]
    fn unknown_prev_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["[[".into(), "??".into()];
        assert!(!_condition(&mut state, &[], &HashMap::new(), nofiles));
    }
}
