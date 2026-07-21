//! Port of `_bsd_disks` from `Completion/BSD/Type/_bsd_disks`.
//!
//! Full upstream body (27 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  # disk device names on BSDs
//! sh: 3  local -a disks
//! sh: 5  case $OSTYPE in
//! sh: 6    freebsd*)
//! sh: 7      disks=( ${${(M)${(f)"$(geom disk list)"}\:#Geom name\:*}#*\: } )
//! sh: 9    dragonfly*)
//! sh:10      disks=( $(sysctl -n kern.disks) )
//! sh:12    openbsd*)
//! sh:13      disks=( ${${(s.,.)"$(sysctl -n hw.disknames)"}%\:*} )
//! sh:15    netbsd*)
//! sh:16      disks=( $(sysctl -n hw.disknames) )
//! sh:18  esac
//! sh:20  if (( $#disks )); then
//! sh:21    local expl
//! sh:22    _wanted disk-devices expl 'disk device' compadd "$@" $disks
//! sh:23    return
//! sh:24  fi
//! sh:26  return 1
//! ```

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::getsparam;
use std::process::Command;

/// sh:7 — parse `geom disk list` output: keep only lines matching the
/// glob `Geom name:*`, then strip the shortest prefix ending in `": "`
/// (e.g. `"Geom name: da0"` -> `"da0"`).
fn parse_geom_disk_list(raw: &str) -> Vec<String> {
    raw.lines()
        .filter(|l| l.starts_with("Geom name:"))
        .map(|l| match l.find(": ") {
            Some(idx) => l[idx + 2..].to_string(),
            None => l.to_string(),
        })
        .collect()
}

/// sh:13 — parse `sysctl -n hw.disknames` on OpenBSD: comma-separated
/// `name:duid` pairs (e.g. `"wd0:abcd,sd0:ef01"`), split on `,` then
/// strip the shortest `:*` suffix off each element.
fn parse_openbsd_disknames(raw: &str) -> Vec<String> {
    raw.trim_end_matches('\n')
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|part| part.split(':').next().unwrap_or(part).to_string())
        .collect()
}

/// sh:10, sh:16 — plain word-split (`$IFS`) of the DragonFlyBSD
/// `kern.disks` / NetBSD `hw.disknames` sysctl output (space-separated
/// disk names, e.g. `"da0 da1 cd0"`).
fn parse_word_split(raw: &str) -> Vec<String> {
    raw.split_whitespace().map(String::from).collect()
}

/// Spawn `cmd args...` and return its captured stdout (empty string on
/// spawn failure — zsh `$(...)` command substitution likewise degrades
/// to empty output rather than aborting the script).
fn run_capture(cmd: &str, args: &[&str]) -> String {
    match Command::new(cmd).args(args).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => String::new(),
    }
}

/// `_bsd_disks` — offer BSD-flavor-specific disk device names.
pub fn _bsd_disks(args: &[String]) -> i32 {
    // sh:5-18  case $OSTYPE in ... esac
    let ostype = getsparam("OSTYPE").unwrap_or_default();
    let disks: Vec<String> = if ostype.starts_with("freebsd") {
        // sh:6-7
        parse_geom_disk_list(&run_capture("geom", &["disk", "list"]))
    } else if ostype.starts_with("dragonfly") {
        // sh:9-10
        parse_word_split(&run_capture("sysctl", &["-n", "kern.disks"]))
    } else if ostype.starts_with("openbsd") {
        // sh:12-13
        parse_openbsd_disknames(&run_capture("sysctl", &["-n", "hw.disknames"]))
    } else if ostype.starts_with("netbsd") {
        // sh:15-16
        parse_word_split(&run_capture("sysctl", &["-n", "hw.disknames"]))
    } else {
        Vec::new()
    };

    // sh:20-24  if (( $#disks )); then ... _wanted ... ; return; fi
    if !disks.is_empty() {
        let mut wanted_argv: Vec<String> = vec![
            "disk-devices".to_string(),
            "expl".to_string(),
            "disk device".to_string(),
            "compadd".to_string(),
        ];
        wanted_argv.extend(args.iter().cloned());
        wanted_argv.extend(disks);
        return _wanted(&wanted_argv);
    }

    // sh:26  return 1
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_geom_disk_list_strips_prefix_and_ignores_other_lines() {
        let raw = "Geom name: da0\n1. Name: da0\n   Providers: 1\nGeom name: da1\n";
        assert_eq!(
            parse_geom_disk_list(raw),
            vec!["da0".to_string(), "da1".to_string()]
        );
    }

    #[test]
    fn parse_geom_disk_list_empty_on_no_match() {
        assert_eq!(parse_geom_disk_list("nothing here\n"), Vec::<String>::new());
    }

    #[test]
    fn parse_openbsd_disknames_splits_comma_then_strips_colon_suffix() {
        let raw = "wd0:e5254f66b5e7ce1c,sd0:e5254f66b5e7ce1c,cd0:\n";
        assert_eq!(
            parse_openbsd_disknames(raw),
            vec!["wd0".to_string(), "sd0".to_string(), "cd0".to_string()]
        );
    }

    #[test]
    fn parse_openbsd_disknames_empty_input() {
        assert_eq!(parse_openbsd_disknames(""), Vec::<String>::new());
    }

    #[test]
    fn parse_word_split_splits_on_whitespace() {
        assert_eq!(
            parse_word_split("da0 da1  cd0\n"),
            vec!["da0".to_string(), "da1".to_string(), "cd0".to_string()]
        );
    }

    #[test]
    fn parse_word_split_empty_input() {
        assert_eq!(parse_word_split(""), Vec::<String>::new());
    }

    #[test]
    fn returns_one_for_unrecognized_ostype_without_spawning_a_command() {
        // sh:5-18 — no case arm matches, so `disks` stays empty and we
        // fall straight to sh:26 `return 1` without ever running geom/sysctl.
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("OSTYPE", "linux-gnu");
        assert_eq!(_bsd_disks(&[]), 1);
    }
}
