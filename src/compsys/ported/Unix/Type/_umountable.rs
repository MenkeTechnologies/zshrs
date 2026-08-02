//! Port of `_umountable` from `Completion/Unix/Type/_umountable`.
//!
//! Full upstream body (50 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 5  case "$OSTYPE" in
//! sh: 6  linux*)   tmp=( "${(@f)$(< /proc/self/mounts)}" )
//! sh: 7            dev_tmp=( "${(@)${(@)tmp%% *}:#none}" )
//! sh: 8            mp_tmp=( "${(@)${(@)tmp#* }%% *}" ) ;;
//! sh:16  freebsd*|dragonfly*)  /sbin/mount | while read mline; do
//! sh:17            [[ $mline[(w)1] = map ]] && continue
//! sh:18            dev_tmp+=( $mline[(w)1] ); mp_tmp+=( $mline[(w)3] ) done ;;
//! sh:22  darwin*)  tmp=( "${(@f)$(/sbin/mount)}" )
//! sh:23            dev_tmp=( "${(@)${(@)tmp%% *}:#map}" )
//! sh:24            mp_tmp=( "${(@)${(@)tmp#* on }%% \(*}" ) ;;
//! sh:27  *)        /sbin/mount | while read mline; do
//! sh:28            mp_tmp+=( $mline[(w)1] ); dev_tmp+=( $mline[(w)3] ) done ;;
//! sh:35  # /etc/mtab encodes odd chars as exactly 3 octal digits (\040 = space).
//! sh:42  mp_tmp=("${(@)mp_tmp//(#m)\\[0-7](#c3)/${(#)$(( 8#${MATCH[2,-1]} ))}}")
//! sh:43  dev_tmp=("${(@)dev_tmp//(#m)\\[0-7](#c3)/…}")
//! sh:44  dpath_tmp=( "${(@M)dev_tmp:#/*}" )
//! sh:45  dev_tmp=( "${(@)dev_tmp:#/*}" )
//! sh:47  _alternative \
//! sh:48    'device-labels:device label:compadd -a dev_tmp' \
//! sh:49    'device-paths: device path:_canonical_paths -A dpath_tmp -N -M "r:|/=* r:|=*" device-paths device\ path' \
//! sh:50    'directories:mount point:_canonical_paths -A mp_tmp -N -M "r:|/=* r:|=*" directories mount\ point'
//! ```

use crate::compsys::ported::_alternative::_alternative;
use crate::ported::params::setaparam;

/// sh:42-43 — decode each `\NNN` (backslash + exactly 3 octal digits)
/// escape into its byte, matching `${…//(#m)\\[0-7](#c3)/…}`.
fn decode_octal_escapes(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && b[i + 1..=i + 3].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            let val = ((b[i + 1] - b'0') as u32) * 64
                + ((b[i + 2] - b'0') as u32) * 8
                + (b[i + 3] - b'0') as u32;
            if let Some(c) = char::from_u32(val) {
                out.push(c);
            }
            i += 4;
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

/// Split a whitespace-delimited line into fields (word 1 = index 0).
fn word(line: &str, n: usize) -> Option<String> {
    line.split_whitespace().nth(n).map(String::from)
}

/// (dev_tmp, mp_tmp) — the raw device and mount-point lists per platform.
fn collect_mounts() -> (Vec<String>, Vec<String>) {
    let mut dev_tmp: Vec<String> = Vec::new();
    let mut mp_tmp: Vec<String> = Vec::new();
    match std::env::consts::OS {
        // sh:6-8 — /proc/self/mounts: dev = word 1 (drop "none"), mp = word 2.
        "linux" => {
            if let Ok(txt) = std::fs::read_to_string("/proc/self/mounts") {
                for line in txt.lines().filter(|l| !l.is_empty()) {
                    if let Some(dev) = word(line, 0) {
                        if dev != "none" {
                            dev_tmp.push(dev);
                        }
                    }
                    if let Some(mp) = word(line, 1) {
                        mp_tmp.push(mp);
                    }
                }
            }
        }
        // sh:22-24 — `/sbin/mount`: "dev on /mnt (type…)".
        "macos" => {
            if let Some(out) = run_mount() {
                for line in out.lines().filter(|l| !l.is_empty()) {
                    if let Some(dev) = word(line, 0) {
                        if dev != "map" {
                            dev_tmp.push(dev);
                        }
                    }
                    if let Some((_, after)) = line.split_once(" on ") {
                        let mp = after.split(" (").next().unwrap_or(after).to_string();
                        mp_tmp.push(mp);
                    }
                }
            }
        }
        // sh:16-18 — freebsd/dragonfly `/sbin/mount`: dev = w1 (skip "map"),
        // mp = w3.
        "freebsd" | "dragonfly" => {
            if let Some(out) = run_mount() {
                for line in out.lines().filter(|l| !l.is_empty()) {
                    if word(line, 0).as_deref() == Some("map") {
                        continue;
                    }
                    if let Some(dev) = word(line, 0) {
                        dev_tmp.push(dev);
                    }
                    if let Some(mp) = word(line, 2) {
                        mp_tmp.push(mp);
                    }
                }
            }
        }
        // sh:27-28 — generic `/sbin/mount`: mp = w1, dev = w3.
        _ => {
            if let Some(out) = run_mount() {
                for line in out.lines().filter(|l| !l.is_empty()) {
                    if let Some(mp) = word(line, 0) {
                        mp_tmp.push(mp);
                    }
                    if let Some(dev) = word(line, 2) {
                        dev_tmp.push(dev);
                    }
                }
            }
        }
    }
    (dev_tmp, mp_tmp)
}

fn run_mount() -> Option<String> {
    std::process::Command::new("/sbin/mount")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// `_umountable` — complete unmountable device labels / paths / mount
/// points, split from the platform mount table.
pub fn _umountable(_args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_umountable");
    let (dev_raw, mp_raw) = collect_mounts();

    // sh:42-43 — decode octal escapes.
    let mp_tmp: Vec<String> = mp_raw.iter().map(|s| decode_octal_escapes(s)).collect();
    let dev_decoded: Vec<String> = dev_raw.iter().map(|s| decode_octal_escapes(s)).collect();
    // sh:44-45 — split device entries into absolute paths vs bare labels.
    let dpath_tmp: Vec<String> = dev_decoded
        .iter()
        .filter(|s| s.starts_with('/'))
        .cloned()
        .collect();
    let dev_tmp: Vec<String> = dev_decoded
        .iter()
        .filter(|s| !s.starts_with('/'))
        .cloned()
        .collect();

    setaparam("dev_tmp", dev_tmp);
    setaparam("dpath_tmp", dpath_tmp);
    setaparam("mp_tmp", mp_tmp);

    // sh:47-50
    _alternative(&[
        "device-labels:device label:compadd -a dev_tmp".to_string(),
        "device-paths: device path:_canonical_paths -A dpath_tmp -N -M \"r:|/=* r:|=*\" device-paths device\\ path".to_string(),
        "directories:mount point:_canonical_paths -A mp_tmp -N -M \"r:|/=* r:|=*\" directories mount\\ point".to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octal_escape_decodes_space() {
        // /etc/mtab encodes space as \040.
        assert_eq!(decode_octal_escapes(r"a\040b"), "a b");
        // Non-escape backslash runs are left intact.
        assert_eq!(decode_octal_escapes(r"a\04b"), r"a\04b");
    }

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        // No completion context registered → _alternative yields no match.
        assert_eq!(_umountable(&[]), 1);
    }
}
