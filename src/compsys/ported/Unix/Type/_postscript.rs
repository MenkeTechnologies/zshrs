//! Port of `_postscript` from `Completion/Unix/Type/_postscript`.
//!
//! Full upstream body (14 lines, abridged):
//! ```text
//! sh: 1  #compdef ps2epsi ps2pdf psmulti pswrap ps2pdf12 ps2pdf13 …
//! sh: 3  local expl ext=''
//! sh: 8  if [[ "$1" == '-z' ]]; then
//! sh: 9    ext='(|.bz2|.gz|.Z)'
//! sh:10    shift
//! sh:11  fi
//! sh:13  _description files expl 'PostScript file'
//! sh:14  _files "$@" "$expl[@]" -g "*.(#i)(ps|eps)$ext(-.)"
//! ```

use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_files::_files;
use crate::ported::params::getaparam;

/// `_postscript` — complete PostScript files (optionally compressed).
pub fn _postscript(args: &[String]) -> i32 {
    // sh:8-11 — leading `-z` allows a trailing compression suffix.
    let (ext, rest) = if args.first().map(|s| s.as_str()) == Some("-z") {
        ("(|.bz2|.gz|.Z)", &args[1..])
    } else {
        ("", args)
    };
    // sh:13
    let _ = _description(&[
        "files".to_string(),
        "expl".to_string(),
        "PostScript file".to_string(),
    ]);
    // sh:14  _files "$@" "$expl[@]" -g "*.(#i)(ps|eps)$ext(-.)"
    let mut a: Vec<String> = rest.to_vec();
    a.extend(getaparam("expl").unwrap_or_default());
    a.push("-g".to_string());
    a.push(format!("*.(#i)(ps|eps){}(-.)", ext));
    _files(&a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_postscript(&[]), 1);
    }
}
