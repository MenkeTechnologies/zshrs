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
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use super::compinit::{CompFileDef, CompInitResult};

/// Dump compinit state to `dump_path` using the upstream zsh
/// `.zcompdump` format so a zshrs-generated dump is consumable by
/// real zsh and vice versa.
///
/// Faithful to upstream `Completion/compdump` (sh:1-141):
///   * sh:21-24  write to a `.HOST.PID` temp file then `mv -f` to
///                the final path for crash-safe replacement
///   * sh:37     header: `#files: N\tversion: V`
///   * sh:43-72  emit `_comps`, `_services`, `_patcomps`,
///                `_postpatcomps`, `_compautos` with sorted keys and
///                `${(qq)}` double-quote escaping
///   * sh:108-130 autoload-list dump (one name per line, plus
///                `_compautos` re-emission with their options)
///
/// Returns the path actually written (the final `dump_path`).
pub fn compdump(
    result: &CompInitResult,
    dump_path: &Path,
    zsh_version: &str,
) -> std::io::Result<PathBuf> {
    // sh:21-23  temp file: `${dump_path}.${HOST}.${PID}` for crash-safe
    //   rename. Avoid touching the live dump until we've fully
    //   written + flushed.
    let host = hostname();
    let pid = std::process::id();
    let tmp = dump_path.with_file_name(format!(
        "{}.{}.{}",
        dump_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        host,
        pid
    ));
    {
        let mut file = File::create(&tmp)?;

        // sh:37  header — TAB-separated key:value pairs
        writeln!(
            file,
            "#files: {}\tversion: {}",
            result.files_scanned, zsh_version
        )?;

        // sh:43-72  the five hash dumps, all sorted-by-key and
        //   double-quote-escaped per `${(qq)}` semantics.
        write_assoc_dump(&mut file, "_comps", &result.comps)?;
        write_assoc_dump(&mut file, "_services", &result.services)?;
        write_assoc_dump(&mut file, "_patcomps", &result.patcomps)?;
        write_assoc_dump(&mut file, "_postpatcomps", &result.postpatcomps)?;
        write_assoc_dump(&mut file, "_compautos", &result.compautos)?;

        // sh:108-130  autoload-list: one `_*` fn per line (sorted),
        //   then `_compautos` re-emission with each fn's captured
        //   `autoload` options applied.
        let mut autoload_names: Vec<String> = result
            .files
            .iter()
            .filter_map(|f| match &f.def {
                CompFileDef::CompDef(_) => Some(f.name.clone()),
                _ => None,
            })
            .collect();
        autoload_names.sort();
        autoload_names.dedup();
        if !autoload_names.is_empty() {
            writeln!(file, "autoload -Uz \\")?;
            for (i, name) in autoload_names.iter().enumerate() {
                let cont = if i + 1 < autoload_names.len() {
                    " \\"
                } else {
                    ""
                };
                writeln!(file, "  {}{}", name, cont)?;
            }
        }
        // sh:127-130 — re-emit each `_compautos` entry with its
        //   captured options (e.g. `+X`) so `autoload +X foo` is
        //   restored verbatim. The shell writes one `autoload ${opts}
        //   ${name}` line per entry.
        let mut compautos_sorted: Vec<(&String, &String)> = result.compautos.iter().collect();
        compautos_sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (name, opts) in &compautos_sorted {
            let opt_str = if opts.is_empty() {
                "-Uz".to_string()
            } else {
                opts.to_string()
            };
            writeln!(file, "autoload {} {}", opt_str, name)?;
        }
        file.sync_all()?;
    }

    // sh:138  atomic rename
    fs::rename(&tmp, dump_path)?;
    Ok(dump_path.to_path_buf())
}

/// sh:43-72 — emit one `name=( 'key1' 'val1' 'key2' 'val2' …)`
/// assoc-array block with sorted keys and `${(qq)}` quoting. Calls
/// `typeset -gHA name` before the assignment.
fn write_assoc_dump<W: Write>(
    w: &mut W,
    name: &str,
    entries: &std::collections::HashMap<String, String>,
) -> std::io::Result<()> {
    writeln!(w, "typeset -gHA {}", name)?;
    if entries.is_empty() {
        writeln!(w, "{}=(\n)", name)?;
        return Ok(());
    }
    // sh:44  `${(ok)X}` — ordered keys. Sort by key.
    let mut sorted: Vec<(&String, &String)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    writeln!(w, "{}=(", name)?;
    for (k, v) in &sorted {
        // sh:46  `${(qq)key} ${(qq)val}` — double-quote-escaped form.
        //   zsh's `(qq)` wraps in `'…'` with embedded `'` rewritten
        //   as `'\''`. Match exactly.
        writeln!(w, "  {} {}", qq(k), qq(v))?;
    }
    writeln!(w, ")")?;
    Ok(())
}

/// `${(qq)s}` — wrap `s` in single quotes, escaping embedded `'`
/// as `'\''` (the safest portable form, what upstream emits).
pub(super) fn qq(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    out.push_str(&s.replace('\'', "'\\''"));
    out.push('\'');
    out
}

/// sh:21 — `$HOST` lookup with reasonable fallback.
fn hostname() -> String {
    std::env::var("HOST")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".to_string())
}

/// Check if dump file is valid and can be used.
///
/// Reads the upstream header `#files: N\tversion: V` and compares
/// `N` against the count of `_*` files in `fpath`, and `V` against
/// the supplied zsh version string.
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
    let line = first_line.trim_end_matches('\n');

    // sh:37 — `#files: N\tversion: V`
    let stripped = match line.strip_prefix("#files:") {
        Some(s) => s.trim_start(),
        None => return false,
    };
    let mut parts = stripped.splitn(2, '\t');
    let n_str = parts.next().unwrap_or("").trim();
    let version_part = parts.next().unwrap_or("");
    let stored_version = match version_part.strip_prefix("version:") {
        Some(s) => s.trim(),
        None => return false,
    };
    let stored_count: usize = match n_str.parse() {
        Ok(n) => n,
        Err(_) => return false,
    };

    // sh:38 — count `_*` files in fpath
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

/// Escape a string for zsh single quotes (legacy public helper).
pub(super) fn escape_zsh_string(s: &str) -> String {
    s.replace('\'', "'\\''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_escape_zsh_string() {
        assert_eq!(escape_zsh_string("hello"), "hello");
        assert_eq!(escape_zsh_string("it's"), "it'\\''s");
    }

    #[test]
    fn qq_wraps_in_single_quotes_and_escapes() {
        assert_eq!(qq("plain"), "'plain'");
        assert_eq!(qq("it's"), "'it'\\''s'");
        assert_eq!(qq(""), "''");
    }

    fn empty_result() -> CompInitResult {
        CompInitResult {
            files_scanned: 3,
            dirs_scanned: 0,
            scan_time_ms: 0,
            files: Vec::new(),
            comps: HashMap::new(),
            services: HashMap::new(),
            patcomps: HashMap::new(),
            postpatcomps: HashMap::new(),
            compautos: HashMap::new(),
            keybindings: Vec::new(),
            widgetkeys: Vec::new(),
        }
    }

    #[test]
    fn header_matches_upstream_format() {
        // sh:37 — `#files: N\tversion: V`. Critical for interop with
        //   a real zsh-generated .zcompdump.
        let mut r = empty_result();
        r.files_scanned = 42;
        let tmp = std::env::temp_dir().join("zshrs_compdump_header_test");
        let _ = std::fs::remove_file(&tmp);
        let _ = compdump(&r, &tmp, "5.9").unwrap();
        let content = std::fs::read_to_string(&tmp).unwrap();
        let first_line = content.lines().next().unwrap();
        assert_eq!(first_line, "#files: 42\tversion: 5.9");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn check_dump_accepts_upstream_header() {
        // Synthesize the exact line format an upstream `compdump` writes
        let tmp = std::env::temp_dir().join("zshrs_check_dump_upstream");
        std::fs::write(
            &tmp,
            "#files: 0\tversion: 5.9\ntypeset -gHA _comps\n_comps=(\n)\n",
        )
        .unwrap();
        // No `_*` files in an empty fpath → current_count = 0 → match.
        assert!(check_dump(&tmp, &[], "5.9"));
        // Wrong version → mismatch
        assert!(!check_dump(&tmp, &[], "5.10"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn check_dump_rejects_old_zshrs_format() {
        // The pre-fix format `#compdump N . V` must NOT be accepted by
        //   the new reader — that was the interop-breaking bug.
        let tmp = std::env::temp_dir().join("zshrs_check_dump_old_format");
        std::fs::write(&tmp, "#compdump 0 . 5.9\n").unwrap();
        assert!(!check_dump(&tmp, &[], "5.9"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn assoc_dump_sorts_keys_deterministically() {
        // sh:44 — `${(ok)X}` sorted-key emission. HashMap iteration
        //   order in Rust is nondeterministic; we must sort.
        let mut r = empty_result();
        r.comps.insert("zfoo".to_string(), "_z".to_string());
        r.comps.insert("alpha".to_string(), "_a".to_string());
        r.comps.insert("mike".to_string(), "_m".to_string());
        let tmp = std::env::temp_dir().join("zshrs_compdump_sort_test");
        let _ = std::fs::remove_file(&tmp);
        let _ = compdump(&r, &tmp, "5.9").unwrap();
        let content = std::fs::read_to_string(&tmp).unwrap();
        let comps_pos = content.find("_comps=(").unwrap();
        let after = &content[comps_pos..];
        let alpha_pos = after.find("'alpha'").unwrap();
        let mike_pos = after.find("'mike'").unwrap();
        let zfoo_pos = after.find("'zfoo'").unwrap();
        assert!(alpha_pos < mike_pos, "alpha must precede mike");
        assert!(mike_pos < zfoo_pos, "mike must precede zfoo");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn rename_replaces_existing_dump_atomically() {
        // sh:21-23 + sh:138 — write to `.HOST.PID` then `mv -f`.
        //   After compdump returns, the temp file must not linger.
        let tmp = std::env::temp_dir().join("zshrs_compdump_atomic_test");
        let _ = std::fs::remove_file(&tmp);
        let _ = compdump(&empty_result(), &tmp, "5.9").unwrap();
        assert!(tmp.exists());
        // Leftover temp files would be named like
        //   `<tmp>.<host>.<pid>` in the same parent dir.
        let parent = tmp.parent().unwrap();
        let stray: Vec<_> = std::fs::read_dir(parent)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with("zshrs_compdump_atomic_test.") && n != "zshrs_compdump_atomic_test"
            })
            .collect();
        assert!(stray.is_empty(), "temp file leaked: {:?}", stray);
        let _ = std::fs::remove_file(&tmp);
    }
}
