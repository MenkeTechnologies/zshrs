//! Port of `_fbsd_device_types` from `Completion/BSD/Type/_fbsd_device_types`.
//!
//! Full upstream body (32 lines):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # device types on FreeBSD/DragonFly
//! sh: 4  # (for commands using devstat_buildmatch(), such as iostat and vmstat)
//! sh: 6  local -a d i types
//! sh: 8  d=( da sa printer proc worm cd scanner optical changer
//! sh: 9      comm array enclosure floppy)
//! sh:10  i=( IDE SCSI other )
//! sh:11  types=(
//! sh:12    "($d)da[direct access devices]"
//! sh:13    "($d)sa[sequential access devices]"
//! sh:14    "($d)printer[printers]"
//! sh:15    "($d)proc[processor devices]"
//! sh:16    "($d)worm[write once read multiple devices]"
//! sh:17    "($d)cd[CD devices]"
//! sh:18    "($d)scanner[scanner devices]"
//! sh:19    "($d)optical[optical memory devices]"
//! sh:20    "($d)changer[medium changer devices]"
//! sh:21    "($d)comm[communication devices]"
//! sh:22    "($d)array[storage array devices]"
//! sh:23    "($d)enclosure[enclosure services devices]"
//! sh:24    "($d)floppy[floppy devices]"
//! sh:25    "($i)IDE[Integrated Drive Electronics devices]"
//! sh:26    "($i)SCSI[Small Computer System Interface devices]"
//! sh:27    "($i)other[any other device interface]"
//! sh:28    'pass[passthrough devices]'
//! sh:29  )
//! sh:31  _values -s , 'device type' $types
//! ```

use crate::compsys::ported::_values::_values;

/// sh:8-9 — mutual-exclusion group `d`: the "disk-like" device types.
const D_GROUP: &str =
    "da sa printer proc worm cd scanner optical changer comm array enclosure floppy";

/// sh:10 — mutual-exclusion group `i`: the interface types.
const I_GROUP: &str = "IDE SCSI other";

/// sh:11-29 — `_values` value-spec strings. Built as a `fn` (rather than a
/// `const [&str]`) so the `($d)`/`($i)` exclusion groups are assembled from
/// `D_GROUP`/`I_GROUP` exactly once, mirroring how zsh interpolates `$d`/`$i`
/// into each spec string at array-construction time.
fn types() -> Vec<String> {
    vec![
        format!("({D_GROUP})da[direct access devices]"),
        format!("({D_GROUP})sa[sequential access devices]"),
        format!("({D_GROUP})printer[printers]"),
        format!("({D_GROUP})proc[processor devices]"),
        format!("({D_GROUP})worm[write once read multiple devices]"),
        format!("({D_GROUP})cd[CD devices]"),
        format!("({D_GROUP})scanner[scanner devices]"),
        format!("({D_GROUP})optical[optical memory devices]"),
        format!("({D_GROUP})changer[medium changer devices]"),
        format!("({D_GROUP})comm[communication devices]"),
        format!("({D_GROUP})array[storage array devices]"),
        format!("({D_GROUP})enclosure[enclosure services devices]"),
        format!("({D_GROUP})floppy[floppy devices]"),
        format!("({I_GROUP})IDE[Integrated Drive Electronics devices]"),
        format!("({I_GROUP})SCSI[Small Computer System Interface devices]"),
        format!("({I_GROUP})other[any other device interface]"),
        "pass[passthrough devices]".to_string(),
    ]
}

/// `_fbsd_device_types` — device types on FreeBSD/DragonFly (for commands
/// using `devstat_buildmatch()`, such as `iostat` and `vmstat`).
///
/// sh: the shell function never references `"$@"`/`$argv` — all arguments
/// to `_values` are the literal `-s ,`, `'device type'`, and the `$types`
/// array. `args` is accepted (per the port ABI) but unused, matching the
/// original's ignoring of its own positional parameters.
pub fn _fbsd_device_types(_args: &[String]) -> i32 {
    // sh:31  _values -s , 'device type' $types
    let mut call: Vec<String> = vec!["-s".to_string(), ",".to_string(), "device type".to_string()];
    call.extend(types());
    _values(&call)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_builds_expected_count_and_exclusion_groups() {
        let v = types();
        // 13 disk-like + 3 interface + 1 passthrough == 17 (sh:12-28).
        assert_eq!(v.len(), 17);
        assert!(v[0].starts_with("(da sa printer"));
        assert_eq!(v[0], format!("({D_GROUP})da[direct access devices]"));
        assert!(v[13].starts_with(&format!("({I_GROUP})")));
        assert_eq!(v[16], "pass[passthrough devices]");
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_fbsd_device_types(&[]), 1);
    }
}
