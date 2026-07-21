//! Port of `_login_classes` from `Completion/BSD/Type/_login_classes`.
//!
//! Full upstream body (11 lines):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl login_classes
//! sh: 5  login_classes=(${${(M)${(f)"$(</etc/login.conf)"}:#[^#[:blank:]]*}%%[:|]*})
//! sh: 6  if [[ $OSTYPE = openbsd* ]]; then
//! sh: 7    login_classes+=(/etc/login.conf.d/*(N:t))
//! sh: 8  fi
//! sh:10  _description login-classes expl 'login class'
//! sh:11  compadd "$@" "$expl[@]" - $login_classes
//! ```

use crate::compsys::ported::_description::_description;
use crate::ported::params::{getaparam, getsparam};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:5 — `${(M)${(f)"$(</etc/login.conf)"}:#[^#[:blank:]]*}` (per line):
/// keep only entry-header lines, i.e. lines whose first character is
/// neither `#` (comment) nor blank/tab (an indented continuation line of
/// the previous entry). Then `%%[:|]*` strips everything from the first
/// `:` or `|` onward, leaving the (first) class name.
fn class_name_from_line(line: &str) -> Option<String> {
    let first = line.chars().next()?;
    if first == '#' || first == ' ' || first == '\t' {
        return None;
    }
    let end = line.find([':', '|']).unwrap_or(line.len());
    Some(line[..end].to_string())
}

/// sh:5 — `${(f)"$(</etc/login.conf)"}` (split file contents into lines)
/// composed with `class_name_from_line` for each line.
fn parse_login_classes(contents: &str) -> Vec<String> {
    contents.lines().filter_map(class_name_from_line).collect()
}

/// sh:7 — `/etc/login.conf.d/*(N:t)`: basenames of directory entries in
/// `dir`, `N` (NULL_GLOB) meaning an absent/empty directory yields no
/// error and no elements; dotfiles are excluded (default glob behavior);
/// results are name-sorted, matching zsh's default glob order.
fn login_conf_d_basenames(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            if let Some(name) = ent.file_name().to_str() {
                if !name.starts_with('.') {
                    out.push(name.to_string());
                }
            }
        }
    }
    out.sort();
    out
}

/// `_login_classes` — complete BSD `login.conf` login class names, plus
/// (on OpenBSD) the extra classes defined under `/etc/login.conf.d/`.
pub fn _login_classes(args: &[String]) -> i32 {
    // sh:5
    let mut login_classes = std::fs::read_to_string("/etc/login.conf")
        .map(|s| parse_login_classes(&s))
        .unwrap_or_default();

    // sh:6-8
    let ostype = getsparam("OSTYPE").unwrap_or_default();
    if ostype.starts_with("openbsd") {
        login_classes.extend(login_conf_d_basenames("/etc/login.conf.d"));
    }

    // sh:10  _description login-classes expl 'login class'
    let _ = _description(&[
        "login-classes".to_string(),
        "expl".to_string(),
        "login class".to_string(),
    ]);
    // sh:11  compadd "$@" "$expl[@]" - $login_classes
    let mut cadd: Vec<String> = args.to_vec();
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.push("-".to_string());
    cadd.extend(login_classes);
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_name_from_line_skips_comments_and_continuations() {
        assert_eq!(class_name_from_line("# a comment"), None);
        assert_eq!(class_name_from_line("\t:path=/usr/bin:"), None);
        assert_eq!(class_name_from_line("    :path=/usr/bin:"), None);
        assert_eq!(class_name_from_line(""), None);
    }

    #[test]
    fn class_name_from_line_strips_from_first_colon_or_pipe() {
        assert_eq!(
            class_name_from_line("default:\\"),
            Some("default".to_string())
        );
        assert_eq!(
            class_name_from_line("staff|Staff Members:\\"),
            Some("staff".to_string())
        );
        assert_eq!(class_name_from_line("bareword"), Some("bareword".to_string()));
    }

    #[test]
    fn parse_login_classes_extracts_only_header_lines() {
        let contents = "\
# /etc/login.conf
#
default:\\
\t:path=/usr/bin:\\
\t:tc=auth-defaults:
staff|Staff Members:\\
\t:tc=default:
";
        assert_eq!(
            parse_login_classes(contents),
            vec!["default".to_string(), "staff".to_string()]
        );
    }

    #[test]
    fn login_conf_d_basenames_empty_dir_is_not_an_error() {
        // sh:7 `(N)` — NULL_GLOB: a missing directory yields no elements,
        // not a failure.
        assert_eq!(
            login_conf_d_basenames("/nonexistent/login.conf.d/for/tests"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn login_conf_d_basenames_lists_sorted_visible_entries() {
        let dir = std::env::temp_dir().join(format!(
            "zshrs_login_classes_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("zzz.conf"), b"").unwrap();
        std::fs::write(dir.join("aaa.conf"), b"").unwrap();
        std::fs::write(dir.join(".hidden.conf"), b"").unwrap();

        let names = login_conf_d_basenames(dir.to_str().unwrap());
        assert_eq!(names, vec!["aaa.conf".to_string(), "zzz.conf".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("OSTYPE", "linux-gnu");
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_login_classes(&[]), 1);
    }
}
