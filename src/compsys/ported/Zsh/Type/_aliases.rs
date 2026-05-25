//! Port of `_aliases` from `Completion/Zsh/Type/_aliases`.
//!
//! Full upstream body (19 lines verbatim):
//! ```text
//! sh: 1  #compdef unalias
//! sh: 2
//! sh: 3  local expl sel args opts
//! sh: 4
//! sh: 5  zparseopts -E -D s:=sel
//! sh: 6
//! sh: 7  [[ -z $sel ]] && sel=rgs
//! sh: 8
//! sh: 9  opts=( "$@" )
//! sh:10
//! sh:11  args=()
//! sh:12  [[ $sel = *r* ]] && args=( $args 'aliases:regular alias:compadd -k aliases' )
//! sh:13  [[ $sel = *g* ]] && args=( $args 'global-aliases:global alias:compadd -k galiases' )
//! sh:14  [[ $sel = *s* ]] && args=( $args 'suffix-aliases:suffix alias:compadd -k saliases' )
//! sh:15  [[ $sel = *R* ]] && args=( $args 'disabled-aliases:disabled regular alias:compadd -k dis_aliases' )
//! sh:16  [[ $sel = *G* ]] && args=( $args 'disabled-global-aliases:disabled global alias:compadd -k dis_galiases' )
//! sh:17  [[ $sel = *S* ]] && args=( $args 'disabled-suffix-aliases:disabled suffix alias:compadd -k dis_saliases' )
//! sh:18
//! sh:19  _alternative -O opts $args
//! ```
//!
//! Strict Rust port: faithful 1:1 — builds the `tag:desc:action`
//! spec strings exactly as upstream does, then dispatches via our
//! ported [`_alternative`]. The action string is the `compadd -k
//! <tablename>` invocation; the action_handler closure resolves
//! `<tablename>` to the right alias slice we've been handed and
//! emits via `add_match`.



use crate::compsys::base::MainCompleteState;
use crate::compsys::completion::Completion;
use crate::compsys::ported::_alternative::_alternative;

/// Six alias tables the leaf crate can't reach without an injection.
pub struct AliasTables<'a> {
    pub regular: &'a [String],
    pub global: &'a [String],
    pub suffix: &'a [String],
    pub disabled_regular: &'a [String],
    pub disabled_global: &'a [String],
    pub disabled_suffix: &'a [String],
}

/// `_aliases` — emit alias names from the requested categories.
///
/// `selector` letters: `r` regular, `g` global, `s` suffix,
/// `R`/`G`/`S` for the disabled variants. Empty defaults to `"rgs"`.
pub fn _aliases(
    state: &mut MainCompleteState,
    tables: &AliasTables<'_>,
    selector: &str,
) -> bool {
    let sel = if selector.is_empty() { "rgs" } else { selector };

    // shell:10-15 — build args list matching upstream verbatim.
    let mut args: Vec<String> = Vec::new();
    if sel.contains('r') {
        args.push("aliases:regular alias:compadd -k aliases".into());
    }
    if sel.contains('g') {
        args.push("global-aliases:global alias:compadd -k galiases".into());
    }
    if sel.contains('s') {
        args.push("suffix-aliases:suffix alias:compadd -k saliases".into());
    }
    if sel.contains('R') {
        args.push("disabled-aliases:disabled regular alias:compadd -k dis_aliases".into());
    }
    if sel.contains('G') {
        args.push("disabled-global-aliases:disabled global alias:compadd -k dis_galiases".into());
    }
    if sel.contains('S') {
        args.push("disabled-suffix-aliases:disabled suffix alias:compadd -k dis_saliases".into());
    }

    // shell:17 — `_alternative -O opts $args`. The action handler
    // dispatches each `compadd -k <tablename>` to the right slice.
    _alternative(state, &args, |s, action| {
        // action is e.g. "compadd -k aliases" — extract the table
        // name (last whitespace-separated token).
        let tablename = action.split_whitespace().last().unwrap_or("");
        let table: &[String] = match tablename {
            "aliases" => tables.regular,
            "galiases" => tables.global,
            "saliases" => tables.suffix,
            "dis_aliases" => tables.disabled_regular,
            "dis_galiases" => tables.disabled_global,
            "dis_saliases" => tables.disabled_suffix,
            _ => return false,
        };
        let prefix = s.comp.params.prefix.clone();
        let mut any = false;
        for name in table {
            if name.starts_with(&prefix) {
                s.comp.add_match(Completion::new(name), None);
                any = true;
            }
        }
        any
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_tables<'a>(
        r: &'a [String],
        g: &'a [String],
        s: &'a [String],
        dr: &'a [String],
        dg: &'a [String],
        ds: &'a [String],
    ) -> AliasTables<'a> {
        AliasTables {
            regular: r,
            global: g,
            suffix: s,
            disabled_regular: dr,
            disabled_global: dg,
            disabled_suffix: ds,
        }
    }

    #[test]
    fn default_selector_rgs_dispatches_three_alternatives() {
        let mut state = MainCompleteState::new("", 0);
        let r = vec!["ll".to_string()];
        let g = vec!["L".to_string()];
        let s = vec!["py".to_string()];
        let t = mk_tables(&r, &g, &s, &[], &[], &[]);
        let _ = _aliases(&mut state, &t, "");
        // _alternative creates groups for each `tag:` in the spec.
        let groups: Vec<&str> = state.comp.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(groups.contains(&"aliases"));
        assert!(groups.contains(&"global-aliases"));
        assert!(groups.contains(&"suffix-aliases"));
    }

    #[test]
    fn selector_r_only_creates_aliases_group_only() {
        let mut state = MainCompleteState::new("", 0);
        let r = vec!["ll".to_string()];
        let g = vec!["G".to_string()];
        let t = mk_tables(&r, &g, &[], &[], &[], &[]);
        let _ = _aliases(&mut state, &t, "r");
        let groups: Vec<&str> = state.comp.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(groups.contains(&"aliases"));
        assert!(!groups.contains(&"global-aliases"));
    }

    #[test]
    fn selector_R_creates_disabled_aliases_group() {
        let mut state = MainCompleteState::new("", 0);
        let dr = vec!["old-alias".to_string()];
        let t = mk_tables(&[], &[], &[], &dr, &[], &[]);
        let _ = _aliases(&mut state, &t, "R");
        let groups: Vec<&str> = state.comp.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(groups.contains(&"disabled-aliases"));
    }

    #[test]
    fn prefix_filters_within_alternative_action() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "ll".into();
        let r = vec!["ll".to_string(), "la".to_string()];
        let t = mk_tables(&r, &[], &[], &[], &[], &[]);
        let _ = _aliases(&mut state, &t, "r");
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["ll"]);
    }

    #[test]
    fn empty_tables_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        let t = mk_tables(&[], &[], &[], &[], &[], &[]);
        assert!(!_aliases(&mut state, &t, "rgsRGS"));
    }

    #[test]
    fn rgsRGS_selector_dispatches_six_alternatives() {
        let mut state = MainCompleteState::new("", 0);
        let one = vec!["x".to_string()];
        let t = mk_tables(&one, &one, &one, &one, &one, &one);
        let _ = _aliases(&mut state, &t, "rgsRGS");
        for tag in [
            "aliases",
            "global-aliases",
            "suffix-aliases",
            "disabled-aliases",
            "disabled-global-aliases",
            "disabled-suffix-aliases",
        ] {
            assert!(
                state.comp.groups.iter().any(|g| g.name == tag),
                "missing group `{tag}`"
            );
        }
    }
}
