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
use std::io::{BufRead, BufReader, BufWriter, Write};
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
///
/// # This function does not run in either shipped mode
///
/// Its only caller is `compinit_compat` (`src/extensions/compinit_bg.rs:140`),
/// whose only caller is `builtin_compinit` behind `if self.zsh_compat`
/// (`src/extensions/ext_builtins.rs:2663`). Those two conditions are mutually
/// exclusive:
///
/// * `zsh_compat` is `is_zsh_mode()` (`bins/zshrs.rs:2517`), and
/// * `compinit` is only compiled to `BUILTIN_COMPINIT` when
///   `!IS_ZSH_MODE` — under `--zsh` the opcode arm returns `None` on purpose
///   so the upstream shell function wins (`src/extensions/compile_zsh.rs:3251-3272`),
///   and `IS_ZSH_MODE` is the same predicate (`bins/zshrs.rs:1626`).
///
/// So under `--zsh` the shell `compdump` writes the dump. Native mode used
/// to write none at all; it now writes an explicit `-d FILE` through
/// [`compdump_live`]. Measured with
/// `fpath=(/opt/homebrew/Cellar/zsh/5.9.2/share/zsh/functions)` and
/// `autoload -Uz compinit; compinit -u -d FILE`:
///
/// ```text
///   zsh          FILE written, `#files: 998   version: 5.9.2`, 52410 bytes
///   zshrs --zsh  FILE written, `#files: 1026  version: 5.9.2`, 53257 bytes
///   zshrs (was)  FILE NOT written ($_comp_dumpfile was set correctly)
///   zshrs (now)  FILE written, byte-identical to the `--zsh` one
/// ```
///
/// `version:` is the discriminator — this function stamps whatever
/// `zsh_version` it is handed, and `compinit_compat` hands it
/// `"zshrs-0.1.0"`. Neither measured mode produced a dump carrying that
/// string; every dump either mode produced carries `$ZSH_VERSION`, which is
/// what the shell `compdump` writes (sh:37).
///
/// Consequence for the sh:108 autoload list below: it is built from
/// `#compdef`-tagged files, where upstream uses `typeset +fm '_*'`
/// intersected with `$fpath` basenames (so upstream also lists headerless
/// helpers such as `_describe`). That divergence is currently unobservable —
/// the list zshrs actually writes comes from the shell `compdump` and is a
/// superset of zsh's (1029 names vs 1001, the extra 28 being bundle-only
/// `_z*`/`_ai`/`_shadow`; nothing zsh lists is missing). Fixing the list here
/// would also need the call ORDER fixed: `compinit_compat` calls this BEFORE
/// `register_autoload_stubs` (`compinit_bg.rs:140` vs `:146`), whereas
/// upstream `compinit` calls `compdump` last, after `compdef -na` has
/// autoloaded every completer — so a faithful `typeset +fm '_*'` read at the
/// current call site would dump an EMPTY list.
///
/// [`compdump_live`] below is the entry point native mode actually uses for
/// an explicit `compinit -d FILE`. It reads the live shell tables rather than
/// a `CompInitResult`, and `builtin_compinit` calls it from the END of the
/// scan branch, where upstream sh:549-551 sits — so sh:108's
/// `typeset +fm '_*'` read is correct there and the list it writes is the
/// upstream one, not the header-driven subset described above.
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
        // The dump is ~5,000 short lines (five assoc blocks plus one
        // `autoload` continuation line per completer). Writing them to a raw
        // `File` cost one `write(2)` per line; a `BufWriter` collapses that
        // into a handful of syscalls. `compinit` over zsh's own Completion
        // tree spent ~24s of its ~37s here. docs/BUGS.md #1122.
        let mut file = BufWriter::new(File::create(&tmp)?);

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
        // Flush the buffer, then fsync the FILE underneath it — the dump is
        // renamed into place next, so it has to be on disk first.
        file.flush()?;
        file.into_inner()
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .sync_all()?;
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

/// sh:26 — `_d_files=( ${^~fpath:/.}/^([^_]*|*~|*.zwc)(N) )`.
///
/// The count the header's `#files:` field carries (sh:37) and the one
/// `compaudit:62` computes from the identical expression, which is what
/// `compinit` sh:494 compares the header against. The negated group means
/// the basename starts with `_`, does not end in `~`, and does not end in
/// `.zwc`; `${…:/.}` drops a literal `.` element. Nothing is deduplicated —
/// the expression is one glob per `$fpath` directory, so a name present in
/// two directories counts twice.
pub fn dump_file_count(fpath: &[PathBuf]) -> usize {
    fpath
        .iter()
        .filter(|d| d.as_os_str() != ".") // sh:26 `${^~fpath:/.}`
        .map(|dir| {
            fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| {
                            let n = e.file_name().to_string_lossy().into_owned();
                            n.starts_with('_') && !n.ends_with('~') && !n.ends_with(".zwc")
                        })
                        .count()
                })
                .unwrap_or(0)
        })
        .sum()
}

/// sh:84-94 — the `zle -lL | while read` half of the widget dump.
///
/// Reads the live thingy table instead of re-parsing `zle -lL`'s stdout;
/// `scanlistwidgets` (zle_thingy.rs:949) formats the same three fields from
/// the same table. sh:86's test is on the `zle -lL` output's word 3 (the
/// widget name) and word 5 (the completion function) — a `zle -C` line is
/// `zle -C <name> <.wid> <func>` — so only `WIDGET_NCOMP` widgets can match
/// at all, and both names must start with `_`.
///
/// Returns the emitted lines plus sh:92's `_d_bks` (the widget names), which
/// sh:97 filters the `bindkey` listing by.
fn widget_dump_lines() -> (Vec<String>, Vec<String>) {
    use crate::ported::utils::quotedzputs;
    use crate::ported::zle::zle_h::{WidgetImpl, WIDGET_INT};

    let mut triples: Vec<(String, String, String)> = Vec::new();
    if let Ok(tab) = crate::ported::zle::zle_thingy::thingytab().lock() {
        for (name, t) in tab.iter() {
            let Some(w) = t.widget.as_ref() else { continue };
            // zle_thingy.c:514-515 — `zle -l` skips internal widgets.
            if (w.flags & WIDGET_INT) != 0 {
                continue;
            }
            if let WidgetImpl::Comp { wid, func, .. } = &w.u {
                // sh:86 `[[ ${_d_line[3]} = _* && ${_d_line[5]} = _* ]]`
                if name.starts_with('_') && func.starts_with('_') {
                    triples.push((name.clone(), wid.clone(), func.clone()));
                }
            }
        }
    }
    // `zle -lL` emits in sorted order (zle_thingy.rs:1000 sorts what C's
    // hash walk leaves in addnode order), and the `while read` loop below it
    // preserves that order.
    triples.sort();

    let mut lines = Vec::with_capacity(triples.len() + 1);
    let mut bks = Vec::with_capacity(triples.len());
    let mut complist = false; // sh:83 `typeset _d_complist=`
    for (name, wid, func) in triples {
        // sh:87-90 — the first `.menu-select` binding needs the module that
        // defines that widget loaded before the dump's `zle -C` line runs.
        if !complist && wid == ".menu-select" {
            lines.push("zmodload -i zsh/complist".to_string()); // sh:88
            complist = true; // sh:89
        }
        // sh:91 `print -r - ${_d_line}` — the `zle -lL` line, verbatim.
        lines.push(format!(
            "zle -C {} {} {}",
            quotedzputs(&name),
            quotedzputs(&wid),
            quotedzputs(&func)
        ));
        bks.push(name); // sh:92
    }
    (lines, bks)
}

/// sh:95-100 — the `bindkey | while read` half.
///
/// `bindkey` with no arguments lists the `main` keymap (zle_keymap.c:1094,
/// `kmname` defaulting to the current keymap); `scankeymap` with `sort=1`
/// walks it in the same order `bin_bindkey_list` does. sh:98 re-quotes the
/// listing's `"…"` form as `'…'` by slicing `[2,-2]` off it, which is what
/// `bindztrdup`'s surrounding double quotes are.
fn bindkey_dump_lines(bks: &[String]) -> Vec<String> {
    let Some(km) = crate::ported::zle::zle_keymap::openkeymap("main") else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    crate::ported::zle::zle_keymap::scankeymap(&km, 1, &mut |seq, bind, _str| {
        let Some(t) = bind else { return };
        // sh:97 `if [[ ${_d_line[2]} = (${(j.|.)~_d_bks}) ]]`
        if !bks.iter().any(|b| b == &t.nam) {
            return;
        }
        let quoted = crate::ported::zle::zle_utils::bindztrdup(seq);
        // sh:98 `${_d_line[1][2,-2]}` — drop `bindztrdup`'s `"` wrapper.
        let inner = quoted
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(&quoted);
        lines.push(format!("bindkey '{}' {}", inner, t.nam));
    });
    lines
}

/// sh:108 — `_d_als=($^fpath/(${(o~j.|.)$(typeset +fm '_*')})(N:t))`.
///
/// Every currently-DEFINED function whose name starts with `_` that also has
/// a file somewhere in `$fpath`, basenamed. No `#compdef` / `#autoload`
/// header is required, which is why the dump's autoload list is a superset
/// of what a header-driven `$fpath` scan can see (workers/38547, quoted at
/// sh:105-106). `$^fpath/(a|b|c)` distributes over `$fpath` in order and
/// each directory's glob comes back sorted, so a name present in two
/// directories is emitted twice — reproduced here rather than deduplicated.
///
/// Note the absence of `:/.` here: unlike sh:26, this expression does not
/// drop a literal `.` element from `$fpath`.
fn autoload_dump_names(fpath: &[PathBuf]) -> Vec<String> {
    let mut defined: Vec<String> = match crate::ported::hashtable::shfunctab_lock().read() {
        // `typeset +fm '_*'` lists every entry of the function table,
        // autoload stubs included (bin_typeset's `+f` prints names only).
        Ok(tab) => tab
            .iter()
            .map(|(k, _)| k)
            .filter(|k| k.starts_with('_'))
            .cloned()
            .collect(),
        Err(_) => return Vec::new(),
    };
    defined.sort(); // sh:108 `${(o…)…}`
    let mut out = Vec::new();
    for dir in fpath {
        for name in &defined {
            if dir.join(name).is_file() {
                out.push(name.clone()); // sh:108 `(N:t)`
            }
        }
    }
    out
}

/// Port of `Completion/compdump` (sh:1-141) reading the LIVE shell state,
/// which is the state upstream reads: sh:44's `${(ok)_comps}`, sh:84's
/// `zle -lL`, sh:108's `typeset +fm '_*'` and sh:135's `$_comp_assocs` are
/// all parameters and tables of the shell that is running `compinit`, not
/// anything a scan carries.
///
/// This is the difference from [`compdump`] above, which formats a
/// `CompInitResult` and therefore has to be called with one in hand. It also
/// fixes that function's call-ORDER constraint: upstream calls `compdump`
/// LAST (sh:549-551, after sh:521-545's `compdef -na` has autoloaded every
/// completer), so reading the live tables is only correct at the very end of
/// `compinit` — which is where `builtin_compinit` calls this.
///
/// `tables` carries the five association arrays (sh:43-72); the caller reads
/// them from the executor rather than this function reaching back into it.
///
/// Returns the path written. `Err` for sh:24's unwritable-directory bail-out
/// (`[[ -w ${_d_file:h} ]] || return 1`) and for any I/O failure.
pub fn compdump_live(
    tables: &super::compinit::DumpTables,
    dump_path: &Path,
    zsh_version: &str,
    fpath: &[PathBuf],
) -> std::io::Result<PathBuf> {
    // sh:21-22 — `${_comp_dumpfile}.$HOST.$$`, written aside and renamed
    // (sh:138) so a concurrent reader never sees a partial dump.
    let tmp = dump_path.with_file_name(format!(
        "{}.{}.{}",
        dump_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        hostname(),
        std::process::id()
    ));
    // sh:24 — `[[ -w ${_d_file:h} ]] || return 1`.
    let parent = dump_path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(p) = parent {
        if !p.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{}: no such directory", p.display()),
            ));
        }
    }

    {
        let mut file = BufWriter::new(File::create(&tmp)?);

        // sh:37 — `print "#files: $#_d_files\tversion: $ZSH_VERSION"`.
        writeln!(
            file,
            "#files: {}\tversion: {}",
            dump_file_count(fpath),
            zsh_version
        )?;

        // sh:43-72 — the five tables, each preceded by a blank line
        // (`print "\n_comps=("`), keys in `${(ok)}` order, both key and
        // value through `${(qq)}`.
        for (name, entries) in [
            ("_comps", &tables.comps),
            ("_services", &tables.services),
            ("_patcomps", &tables.patcomps),
            ("_postpatcomps", &tables.postpatcomps),
            ("_compautos", &tables.compautos),
        ] {
            writeln!(file)?;
            writeln!(file, "{}=(", name)?;
            let mut sorted: Vec<(&String, &String)> = entries.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(b.0));
            for (k, v) in sorted {
                writeln!(file, "{} {}", qq(k), qq(v))?; // sh:45
            }
            writeln!(file, ")")?;
        }

        // sh:74 — `print >& $_d_fd`.
        writeln!(file)?;

        // sh:82-100 — widget definitions then the key sequences bound to
        // them.
        let (widget_lines, bks) = widget_dump_lines();
        for line in &widget_lines {
            writeln!(file, "{}", line)?;
        }
        for line in bindkey_dump_lines(&bks) {
            writeln!(file, "{}", line)?;
        }

        // sh:102 — `print >& $_d_fd`.
        writeln!(file)?;

        // sh:112-125 — `autoload -Uz` + the names, five to a line. The
        // counter only decrements on a name that is actually printed
        // (sh:115's `_compautos` entries are skipped here and re-emitted
        // with their own options at sh:128-130), and sh:117's
        // `$#_d_als > 1` is the length of what is LEFT including the name
        // just printed, so the last line never gets a continuation.
        let als = autoload_dump_names(fpath);
        write!(file, "autoload -Uz")?; // sh:113
        let mut i = 5; // sh:112 `integer _i=5`
        for (idx, name) in als.iter().enumerate() {
            if tables.compautos.contains_key(name) {
                continue; // sh:115
            }
            write!(file, " {}", name)?; // sh:116
            i -= 1;
            if i == 0 && als.len() - idx > 1 {
                i = 5; // sh:118
                write!(file, " \\\n           ")?; // sh:119
            }
        }
        writeln!(file)?; // sh:125

        // sh:127-130 — one `autoload -Uz <opts> <name>` per `_compautos`
        // entry, in `${(ok@)}` order.
        let mut compautos_sorted: Vec<(&String, &String)> = tables.compautos.iter().collect();
        compautos_sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (name, opts) in compautos_sorted {
            writeln!(file, "autoload -Uz {} {}", opts, name)?; // sh:129
        }

        // sh:132 — `print >& $_d_fd`.
        writeln!(file)?;

        // sh:134-135. `${(qq)_comp_assocs}` inside double quotes joins the
        // array with spaces, each element quoted; an empty/unset array
        // yields a single `''`.
        writeln!(file, "typeset -gUa _comp_assocs")?;
        let assocs = crate::ported::params::getaparam("_comp_assocs").unwrap_or_default();
        let joined = if assocs.is_empty() {
            qq("")
        } else {
            assocs
                .iter()
                .map(|s| qq(s))
                .collect::<Vec<_>>()
                .join(" ")
        };
        writeln!(file, "_comp_assocs=( {} )", joined)?;

        file.flush()?;
        file.into_inner()
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .sync_all()?;
    }

    // sh:138 — `mv -f $_d_file ${_d_file%.$HOST.$$}`.
    fs::rename(&tmp, dump_path)?;
    Ok(dump_path.to_path_buf())
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

    /// sh:26 `_d_files=( ${^~fpath:/.}/^([^_]*|*~|*.zwc)(N) )` — the count
    /// the `#files:` header carries and the one `compinit` sh:494 compares
    /// against. The negated glob group is three alternatives, so a name is
    /// counted only when it starts with `_`, does not end in `~`, and does
    /// not end in `.zwc`. `${…:/.}` drops a literal `.` element.
    #[test]
    fn dump_file_count_matches_the_sh26_glob() {
        let dir = std::env::temp_dir().join("zshrs_compdump_filecount");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (name, counted) in [
            ("_counted", true),
            ("_also_counted", true),
            ("notunderscore", false), // `[^_]*`
            ("_backup~", false),      // `*~`
            ("_compiled.zwc", false), // `*.zwc`
        ] {
            fs::write(dir.join(name), "").unwrap();
            let _ = counted;
        }
        assert_eq!(dump_file_count(&[dir.clone()]), 2);
        // `.` is dropped before the glob, so it contributes nothing even
        // when the process cwd is full of `_*` files.
        assert_eq!(dump_file_count(&[PathBuf::from(".")]), 0);
        // No dedup: one glob per directory, so the same basename in two
        // directories counts twice.
        assert_eq!(dump_file_count(&[dir.clone(), dir.clone()]), 4);
        let _ = fs::remove_dir_all(&dir);
    }

    /// `compdump_live` must write what `compinit -C` reads back: the whole
    /// point of honouring `-d FILE` is that a later shell loads it. Round
    /// trip through the two readers `builtin_compinit`'s `-C` branch uses.
    ///
    /// Also pins sh:115/sh:129's split, which the `_compautos` bug made
    /// visible: a name with autoload options must leave the main
    /// `autoload -Uz …` list and reappear as its own
    /// `autoload -Uz <opts> <name>` line.
    #[test]
    fn compdump_live_round_trips_through_the_dump_readers() {
        use crate::compsys::ported::compinit::{
            dump_assoc_tables, dump_autoload_names, DumpTables,
        };
        let dir = std::env::temp_dir().join("zshrs_compdump_live_roundtrip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let dump = dir.join("dumpfile");

        let mut tables = DumpTables::default();
        tables.comps.insert("zzcmd".to_string(), "_zzcmd".to_string());
        // `${(qq)}` has to survive an embedded quote (sh:40-41's "quoting
        // hieroglyphics") — a raw `'` would end the value's quoting and
        // shift every following word.
        tables
            .comps
            .insert("it's".to_string(), "_apostrophe".to_string());
        tables
            .services
            .insert("-redirect-,<,zzz".to_string(), "zzz".to_string());
        tables
            .patcomps
            .insert("zz*".to_string(), "_zzpat".to_string());
        tables
            .postpatcomps
            .insert("*zz".to_string(), "_zzpost".to_string());
        tables
            .compautos
            .insert("_zzauto".to_string(), "+X".to_string());

        compdump_live(&tables, &dump, "5.9.2", &[dir.clone()]).unwrap();
        let text = fs::read_to_string(&dump).unwrap();

        assert!(
            text.starts_with("#files: 0\tversion: 5.9.2\n"),
            "sh:37 header, got {:?}",
            text.lines().next()
        );
        let read = dump_assoc_tables(&dump).expect("dump must parse");
        assert_eq!(read.comps.get("zzcmd").map(String::as_str), Some("_zzcmd"));
        assert_eq!(
            read.comps.get("it's").map(String::as_str),
            Some("_apostrophe"),
            "an embedded `'` must round-trip through ${{(qq)}}"
        );
        assert_eq!(
            read.services.get("-redirect-,<,zzz").map(String::as_str),
            Some("zzz")
        );
        assert_eq!(read.patcomps.get("zz*").map(String::as_str), Some("_zzpat"));
        assert_eq!(
            read.postpatcomps.get("*zz").map(String::as_str),
            Some("_zzpost")
        );
        assert_eq!(read.compautos.get("_zzauto").map(String::as_str), Some("+X"));

        // sh:129 — the `_compautos` entry gets its own line, options first.
        assert!(
            text.contains("\nautoload -Uz +X _zzauto\n"),
            "sh:129 line missing from:\n{}",
            text
        );
        // …and sh:115 keeps it out of the bulk list, so the reader sees the
        // name exactly once.
        assert_eq!(
            dump_autoload_names(&dump)
                .iter()
                .filter(|n| *n == "_zzauto")
                .count(),
            1,
            "sh:115 must not also list _zzauto in the bulk `autoload -Uz` line"
        );
        // sh:134-135 — `compinit -C` reaches these two lines by sourcing the
        // dump; without them `$_comp_assocs` is undeclared for the session.
        assert!(text.contains("\ntypeset -gUa _comp_assocs\n"));
        assert!(text.contains("\n_comp_assocs=( "));
        // sh:138's rename must leave no `.HOST.PID` sibling behind.
        let stray: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("dumpfile."))
            .collect();
        assert!(stray.is_empty(), "temp file leaked: {:?}", stray);
        let _ = fs::remove_dir_all(&dir);
    }
}
