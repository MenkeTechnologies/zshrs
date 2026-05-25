//! Port of `compdump` from `Completion/compdump`.
//!
//! Full upstream body (141 lines verbatim):
//! ```text
//! sh:  1  # This is a function to dump the definitions for new-style
//! sh:  2  # completion defined by 'compinit' in the same directory.  The output
//! sh:  3  # should be directed into the "compinit.dump" in the same directory as
//! sh:  4  # compinit. If you rename init, just stick .dump onto the end of whatever
//! sh:  5  # you have called it and put it in the same directory.  This is handled
//! sh:  6  # automatically if you invoke compinit with the option -d.
//! sh:  7  #
//! sh:  8  # You will need to update the dump every time you add a new completion.
//! sh:  9  # To do this, simply remove the .dump file, start a new shell, and
//! sh: 10  # create the .dump file as before.  Again, compinit -d handles this
//! sh: 11  # automatically.
//! sh: 12
//! sh: 13  # Print the number of files used for completion. This is used in compinit
//! sh: 14  # to see if auto-dump should re-dump the dump-file.
//! sh: 15
//! sh: 16  emulate -L zsh
//! sh: 17  setopt extendedglob noshglob
//! sh: 18
//! sh: 19  typeset _d_file _d_f _d_fd _d_bks _d_line _d_als _d_files _d_name _d_tmp
//! sh: 20
//! sh: 21  _d_file=${_comp_dumpfile-${0:h}/compinit.dump}.$HOST.$$
//! sh: 22  [[ $_d_file = //* ]] && _d_file=${_d_file[2,-1]}
//! sh: 23
//! sh: 24  [[ -w ${_d_file:h} ]] || return 1
//! sh: 25
//! sh: 26  _d_files=( ${^~fpath:/.}/^([^_]*|*~|*.zwc)(N) )
//! sh: 27
//! sh: 28  if [[ -n "$_comp_secure" ]]; then
//! sh: 29    _d_wdirs=( ${^fpath}(Nf:g+w:,f:o+w:,^u0u${EUID}) )
//! sh: 30    _d_wfiles=( ${^~fpath:/.}/^([^_]*|*~|*.zwc)(N^u0u${EUID}) )
//! sh: 31
//! sh: 32    (( $#_d_wfiles )) && _d_files=( "${(@)_d_files:#(${(j:|:)_d_wfiles})}"  )
//! sh: 33    (( $#_d_wdirs ))  && _d_files=( "${(@)_d_files:#(${(j:|:)_d_wdirs})/*}" )
//! sh: 34  fi
//! sh: 35
//! sh: 36  exec {_d_fd}>$_d_file
//! sh: 37  print "#files: $#_d_files\tversion: $ZSH_VERSION" >& $_d_fd
//! sh: 38
//! sh: 39  # Dump the arrays _comps, _services and _patcomps.  The quoting
//! sh: 40  # hieroglyphics ensure that a single quote inside a variable is itself
//! sh: 41  # correctly quoted.
//! sh: 42
//! sh: 43  print "\n_comps=(" >& $_d_fd
//! sh: 44  for _d_f in ${(ok)_comps}; do
//! sh: 45    print -r - "${(qq)_d_f}" "${(qq)_comps[$_d_f]}"
//! sh: 46  done >& $_d_fd
//! sh: 47  print ")" >& $_d_fd
//! sh: 48
//! sh: 49  print "\n_services=(" >& $_d_fd
//! sh: 50  for _d_f in ${(ok)_services}; do
//! sh: 51    print -r - "${(qq)_d_f}" "${(qq)_services[$_d_f]}"
//! sh: 52  done >& $_d_fd
//! sh: 53  print ")" >& $_d_fd
//! sh: 54
//! sh: 55  print "\n_patcomps=(" >& $_d_fd
//! sh: 56  for _d_f in ${(ok)_patcomps}; do
//! sh: 57    print -r - "${(qq)_d_f}" "${(qq)_patcomps[$_d_f]}"
//! sh: 58  done >& $_d_fd
//! sh: 59  print ")" >& $_d_fd
//! sh: 60
//! sh: 61  _d_tmp="_postpatcomps"
//! sh: 62  print "\n_postpatcomps=(" >& $_d_fd
//! sh: 63  for _d_f in ${(ok)_postpatcomps}; do
//! sh: 64    print -r - "${(qq)_d_f}" "${(qq)_postpatcomps[$_d_f]}"
//! sh: 65  done >& $_d_fd
//! sh: 66  print ")" >& $_d_fd
//! sh: 67
//! sh: 68  print "\n_compautos=(" >& $_d_fd
//! sh: 69  for _d_f in "${(ok@)_compautos}"; do
//! sh: 70    print -r - "${(qq)_d_f}" "${(qq)_compautos[$_d_f]}"
//! sh: 71  done >& $_d_fd
//! sh: 72  print ")" >& $_d_fd
//! sh: 73
//! sh: 74  print >& $_d_fd
//! sh: 75
//! sh: 76  # Now dump the key bindings. We dump all bindings for zle widgets
//! sh: 77  # whose names start with a underscore.
//! sh: 78  # We need both the zle -C's and the bindkey's to recreate.
//! sh: 79  # We can ignore any zle -C which rebinds a standard widget (second
//! sh: 80  # argument to zle does not begin with a `_').
//! sh: 81
//! sh: 82  _d_bks=()
//! sh: 83  typeset _d_complist=
//! sh: 84  zle -lL |
//! sh: 85    while read -rA _d_line; do
//! sh: 86      if [[ ${_d_line[3]} = _* && ${_d_line[5]} = _* ]]; then
//! sh: 87        if [[ -z "$_d_complist" && ${_d_line[4]} = .menu-select ]]; then
//! sh: 88          print 'zmodload -i zsh/complist'
//! sh: 89  	_d_complist=yes
//! sh: 90        fi
//! sh: 91        print -r - ${_d_line}
//! sh: 92        _d_bks+=(${_d_line[3]})
//! sh: 93      fi
//! sh: 94    done >& $_d_fd
//! sh: 95  bindkey |
//! sh: 96    while read -rA _d_line; do
//! sh: 97      if [[ ${_d_line[2]} = (${(j.|.)~_d_bks}) ]]; then
//! sh: 98        print -r "bindkey '${_d_line[1][2,-2]}' ${_d_line[2]}"
//! sh: 99      fi
//! sh:100    done >& $_d_fd
//! sh:101
//! sh:102  print >& $_d_fd
//! sh:103
//! sh:104
//! sh:105  # Autoloads: look for all defined functions beginning with `_' (that also
//! sh:106  # exists in fpath: see workers/38547).
//! sh:107
//! sh:108  _d_als=($^fpath/(${(o~j.|.)$(typeset +fm '_*')})(N:t))
//! sh:109
//! sh:110  # print them out:  about five to a line looks neat
//! sh:111
//! sh:112  integer _i=5
//! sh:113  print -n autoload -Uz >& $_d_fd
//! sh:114  while (( $#_d_als )); do
//! sh:115    if (( ! $+_compautos[$_d_als[1]] )); then
//! sh:116      print -n " $_d_als[1]"
//! sh:117      if (( ! --_i && $#_d_als > 1 )); then
//! sh:118        _i=5
//! sh:119        print -n ' \\\n           '
//! sh:120      fi
//! sh:121    fi
//! sh:122    shift _d_als
//! sh:123  done >& $_d_fd
//! sh:124
//! sh:125  print >& $_d_fd
//! sh:126
//! sh:127  local _c
//! sh:128  for _c in "${(ok@)_compautos}"; do
//! sh:129    print "autoload -Uz $_compautos[$_c] $_c" >& $_d_fd
//! sh:130  done
//! sh:131
//! sh:132  print >& $_d_fd
//! sh:133
//! sh:134  print "typeset -gUa _comp_assocs" >& $_d_fd
//! sh:135  print "_comp_assocs=( ${(qq)_comp_assocs} )" >& $_d_fd
//! sh:136  exec {_d_fd}>&-
//! sh:137
//! sh:138  mv -f $_d_file ${_d_file%.$HOST.$$}
//! sh:139
//! sh:140  unfunction compdump
//! sh:141  autoload -Uz compdump
//! ```



use rayon::prelude::*;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::compinit::{CompFileDef, CompInitResult};

/// Dump the compinit state to a cache file
pub fn compdump(
    result: &CompInitResult,
    dump_path: &Path,
    zsh_version: &str,
) -> std::io::Result<()> {
    use std::io::Write;

    let mut file = File::create(dump_path)?;

    // Header line: #compdump <num_files> . <zsh_version>
    writeln!(file, "#compdump {} . {}", result.files_scanned, zsh_version)?;

    // Dump _comps
    writeln!(
        file,
        "typeset -gHA _comps _services _patcomps _postpatcomps _compautos"
    )?;
    writeln!(file, "_comps=(")?;
    for (cmd, func) in &result.comps {
        writeln!(
            file,
            "  '{}' '{}'",
            escape_zsh_string(cmd),
            escape_zsh_string(func)
        )?;
    }
    writeln!(file, ")")?;

    // Dump _services
    writeln!(file, "_services=(")?;
    for (cmd, svc) in &result.services {
        writeln!(
            file,
            "  '{}' '{}'",
            escape_zsh_string(cmd),
            escape_zsh_string(svc)
        )?;
    }
    writeln!(file, ")")?;

    // Dump _patcomps
    writeln!(file, "_patcomps=(")?;
    for (pat, func) in &result.patcomps {
        writeln!(
            file,
            "  '{}' '{}'",
            escape_zsh_string(pat),
            escape_zsh_string(func)
        )?;
    }
    writeln!(file, ")")?;

    // Dump _postpatcomps
    writeln!(file, "_postpatcomps=(")?;
    for (pat, func) in &result.postpatcomps {
        writeln!(
            file,
            "  '{}' '{}'",
            escape_zsh_string(pat),
            escape_zsh_string(func)
        )?;
    }
    writeln!(file, ")")?;

    // Dump _compautos
    writeln!(file, "_compautos=(")?;
    for (name, opts) in &result.compautos {
        writeln!(
            file,
            "  '{}' '{}'",
            escape_zsh_string(name),
            escape_zsh_string(opts)
        )?;
    }
    writeln!(file, ")")?;

    // Autoload all completion functions
    writeln!(file, "autoload -Uz \\")?;
    for file_info in &result.files {
        if matches!(file_info.def, CompFileDef::CompDef(_)) {
            writeln!(file, "  {} \\", file_info.name)?;
        }
    }
    writeln!(file)?;

    Ok(())
}

/// Check if dump file is valid and can be used
pub fn check_dump(dump_path: &Path, fpath: &[PathBuf], zsh_version: &str) -> bool {
    let file = match File::open(dump_path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    if reader.read_line(&mut first_line).is_err() {
        return false;
    }

    // Parse header: #compdump <num_files> . <version>
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 4 || parts[0] != "#compdump" {
        return false;
    }

    let stored_count: usize = match parts[1].parse() {
        Ok(n) => n,
        Err(_) => return false,
    };

    let stored_version = parts[3];

    // Quick count of files in fpath
    let current_count: usize = fpath
        .par_iter()
        .filter(|dir| dir.as_os_str() != "." && dir.exists())
        .map(|dir| {
            fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_name().to_string_lossy().starts_with('_'))
                        .count()
                })
                .unwrap_or(0)
        })
        .sum();

    stored_count == current_count && stored_version == zsh_version
}

/// Escape a string for zsh single quotes
pub(super) fn escape_zsh_string(s: &str) -> String {
    s.replace('\'', "'\\''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_zsh_string() {
        assert_eq!(escape_zsh_string("hello"), "hello");
        assert_eq!(escape_zsh_string("it's"), "it'\\''s");
    }
}
