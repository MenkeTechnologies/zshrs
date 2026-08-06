//! Port of `_locales` from `Completion/Unix/Type/_locales`.
//!
//! Full upstream body (12 lines verbatim):
//! ```text
//! sh: 1  #compdef -value-,LANG,-default- -value-,LANGUAGE,-default- -P …
//! sh: 3  local expl locales
//! sh: 5  if (( $+commands[locale] )); then
//! sh: 6    locales=( $(_call_program locales locale -a) )
//! sh: 7    [[ $OSTYPE = *-gnu ]] && locales=( ${locales/utf8/UTF-8} )
//! sh: 8  else
//! sh: 9    locales=( /usr/lib/locale/*(:t) )
//! sh:10  fi
//! sh:12  _wanted locales expl locale compadd -a "$@" - locales
//! ```
//!
//! sh:5 approx — rather than testing `$+commands[locale]`, run `locale -a`
//! via `_call_program` and fall back to the `/usr/lib/locale` listing when it
//! produces nothing (locale absent / spawn failure).

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getsparam, setaparam};

/// `_locales` — complete installed locale names.
pub fn _locales(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_locales");
    // sh:5-6  $(_call_program locales locale -a) — REPLY carries stdout.
    let _ = _call_program(&[
        "locales".to_string(),
        "locale".to_string(),
        "-a".to_string(),
    ]);
    let reply = getsparam("REPLY").unwrap_or_default();
    let mut locales: Vec<String> = reply.split_whitespace().map(String::from).collect();

    if locales.is_empty() {
        // sh:9  fall back to /usr/lib/locale/*(:t) (basenames).
        if let Ok(rd) = std::fs::read_dir("/usr/lib/locale") {
            locales = rd
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| !n.starts_with('.')) // glob hides dotfiles by default
                .collect();
        }
    } else {
        // sh:7  [[ $OSTYPE = *-gnu ]] && locales=( ${locales/utf8/UTF-8} )
        let ostype = getsparam("OSTYPE").unwrap_or_default();
        if ostype.ends_with("-gnu") {
            locales = locales
                .iter()
                .map(|l| l.replacen("utf8", "UTF-8", 1))
                .collect();
        }
    }

    // sh:12  _wanted locales expl locale compadd -a "$@" - locales
    // `compadd -a - locales` reads the ARRAY PARAM named `locales`.
    setaparam("locales", locales);
    let mut w = vec![
        "locales".to_string(),
        "expl".to_string(),
        "locale".to_string(),
        "compadd".to_string(),
        "-a".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.push("locales".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_locales(&[]), 1);
    }
}
