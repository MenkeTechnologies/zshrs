//! Port of `_pdf` from `Completion/Unix/Type/_pdf`.
//!
//! Full upstream body (20 lines, abridged):
//! ```text
//! sh: 1  #compdef pdf2dsc pdf2ps pdfimages pdfinfo pdftopbm pdftops …
//! sh: 3  local expl ext=''
//! sh:14  if [[ "$1" == '-z' ]]; then
//! sh:15    ext='(|.bz2|.gz|.Z)'
//! sh:16    shift
//! sh:17  fi
//! sh:19  _description files expl 'PDF file'
//! sh:20  _files "$@" "$expl[@]" -g "*.(#i)pdf$ext(-.)"
//! ```

use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_files::_files;
use crate::ported::params::getaparam;

/// `_pdf` — complete PDF files (optionally compressed, with `-z`).
pub fn _pdf(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_pdf");
    // sh:14-17 — a leading `-z` allows a trailing compression suffix.
    let (ext, rest) = if args.first().map(|s| s.as_str()) == Some("-z") {
        ("(|.bz2|.gz|.Z)", &args[1..])
    } else {
        ("", args)
    };
    // sh:19
    let _ = _description(&[
        "files".to_string(),
        "expl".to_string(),
        "PDF file".to_string(),
    ]);
    // sh:20  _files "$@" "$expl[@]" -g "*.(#i)pdf$ext(-.)"
    let mut a: Vec<String> = rest.to_vec();
    a.extend(getaparam("expl").unwrap_or_default());
    a.push("-g".to_string());
    a.push(format!("*.(#i)pdf{}(-.)", ext));
    crate::compsys::ported::shared::call_compfn("_files", &a, || _files(&a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_pdf(&[]), 1);
    }

    #[test]
    fn dash_z_is_consumed() {
        let _g = crate::test_util::global_state_lock();
        // -z must not leak into _files' argv; still returns 1 (no executor).
        assert_eq!(_pdf(&["-z".to_string()]), 1);
    }
}
