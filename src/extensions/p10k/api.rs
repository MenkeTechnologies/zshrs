//! Port of the `p10k()` shell FUNCTION (internal/p10k.zsh:8983-9156)
//! as a native builtin dispatcher, plus the usage strings
//! (p10k:9169-9330) and the `p10k display` visibility-override state.
//!
//! The zsh theme defines `p10k` as a function whose single argument
//! selects a subcommand: `segment` (emit a user segment mid-render),
//! `display` (show/hide/toggle prompt parts), `reload`, `configure`,
//! `help`, `finalize`, `clear-instant-prompt`. With the theme never
//! executing, calls to `p10k <cmd>` from .zshrc / zpwr land here.
//!
//! `p10k configure` runs the native wizard port (`wizard/`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// p10k:9124 `_p9k__force_must_init=1` — `p10k reload` sets it so the
/// theme re-reads config. The native engine reads the paramtab live on
/// every render, so this is observational only (exposed for tests /
/// future daemon cache-invalidation).
pub static FORCE_REINIT: AtomicBool = AtomicBool::new(false);

/// The prompt layout `p10k display` addresses (p10k:8337-8359
/// `_p9k_init_display`): element names per aligned prompt line, left
/// and right. Registered by `preprompt_render` before every frame.
#[derive(Default)]
struct Layout {
    left: Vec<Vec<String>>,
    right: Vec<Vec<String>>,
}

/// One addressable part: canonical name (positive indices) plus its
/// negative-index alias (`-1/left/dir` ≡ `last line`).
struct Slot {
    name: String,
    alias: Option<String>,
}

static LAYOUT: Mutex<Option<Layout>> = Mutex::new(None);

/// `p10k display` states by canonical part name; absent = the part's
/// default state ([`default_state`]).
static DISPLAY_STATE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Register the aligned element names (`newline`-split, `_joined`
/// stripped) of the frame being rendered.
pub fn set_layout(left: Vec<Vec<String>>, right: Vec<Vec<String>>) {
    *LAYOUT.lock().unwrap() = Some(Layout { left, right });
}

/// Every slot in `_p9k_display_k` order (p10k:8337-8359).
fn slots() -> Vec<Slot> {
    let g = LAYOUT.lock().unwrap();
    let mut out = vec![
        Slot { name: "empty_line".into(), alias: None },
        Slot { name: "ruler".into(), alias: None },
    ];
    let Some(layout) = g.as_ref() else { return out };
    let n = layout.left.len().max(layout.right.len()) as i64;
    for i in 1..=n {
        let j = -n + i - 1;
        let both = |suffix: &str| Slot {
            name: format!("{i}{suffix}"),
            alias: Some(format!("{j}{suffix}")),
        };
        out.push(both(""));
        out.push(both("/left_frame"));
        out.push(both("/right_frame"));
        out.push(both("/left"));
        out.push(both("/right"));
        out.push(both("/gap"));
        let idx = (i - 1) as usize;
        for name in layout.left.get(idx).into_iter().flatten() {
            out.push(both(&format!("/left/{name}")));
        }
        for name in layout.right.get(idx).into_iter().flatten() {
            out.push(both(&format!("/right/{name}")));
        }
    }
    out
}

/// The state a part has before any `p10k display` toggle: `show`,
/// except `empty_line` and `ruler`, which follow p10k:6905-6930 — hidden
/// unless PROMPT_ADD_NEWLINE / SHOW_RULER is on; then hidden on a new
/// tty, `print` without a transient prompt, else `show`.
fn default_state(name: &str) -> String {
    let enabled = match name {
        "empty_line" => crate::extensions::p10k::config::p9k_global("PROMPT_ADD_NEWLINE", "") == "true",
        "ruler" => crate::extensions::p10k::config::p9k_global("SHOW_RULER", "") == "true",
        _ => return "show".to_string(),
    };
    if !enabled || crate::p10k::tty_is_new() {
        "hide".to_string()
    } else if crate::p10k::transient::transient_enabled().is_none() {
        "print".to_string()
    } else {
        "show".to_string()
    }
}

fn state_of(map: &HashMap<String, String>, name: &str) -> String {
    map.get(name).cloned().unwrap_or_else(|| default_state(name))
}

/// Current state of one part (canonical name).
pub fn part_state(name: &str) -> String {
    let g = DISPLAY_STATE.lock().unwrap();
    match g.as_ref() {
        Some(m) => state_of(m, name),
        None => default_state(name),
    }
}

/// Indices into `slots` whose name or alias matches the glob `pattern`
/// (`${(u@)_p9k_display_k[(I)$pattern]}`, p10k:9420), unique per slot.
fn matching(slots: &[Slot], pattern: &str) -> Vec<usize> {
    slots
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            crate::extensions::p10k::shared::glob_name_matches(pattern, &s.name)
                || s.alias
                    .as_deref()
                    .is_some_and(|a| crate::extensions::p10k::shared::glob_name_matches(pattern, a))
        })
        .map(|(i, _)| i)
        .collect()
}

/// True when the part `name` (canonical form, e.g. `2/left/dir`,
/// `1/left_frame`) is hidden by a `p10k display` toggle.
pub fn is_hidden(name: &str) -> bool {
    let g = DISPLAY_STATE.lock().unwrap();
    g.as_ref().is_some_and(|m| m.get(name).is_some_and(|s| s == "hide"))
}

/// p10k:9422-9456 — apply one `part-pattern=state-list` toggle. A
/// single state sets it; several cycle from the current state to the
/// next (wrapping to the first). Returns whether anything changed
/// (drives the reset-prompt at p10k:9459).
pub fn display_set(pattern: &str, states: &[&str]) -> bool {
    if states.is_empty() {
        return false;
    }
    let all = slots();
    let mut g = DISPLAY_STATE.lock().unwrap();
    let map = g.get_or_insert_with(HashMap::new);
    let mut changed = false;
    for k in matching(&all, pattern) {
        let name = &all[k].name;
        let cur = state_of(map, name);
        let new = if states.len() == 1 {
            states[0].to_string()
        } else {
            // `${list[list[(I)$cur]+1]:-$list[1]}`
            match states.iter().rposition(|s| *s == cur) {
                Some(i) => states.get(i + 1).unwrap_or(&states[0]).to_string(),
                None => states[0].to_string(),
            }
        };
        if new == cur {
            continue;
        }
        map.insert(name.clone(), new);
        changed = true;
    }
    changed
}

/// p10k:9396-9411 `p10k display -a` — `(name state)` pairs for every
/// part matching each pattern (default `*`).
pub fn display_dump(patterns: &[&str]) -> Vec<String> {
    let all = slots();
    let g = DISPLAY_STATE.lock().unwrap();
    let empty = HashMap::new();
    let map = g.as_ref().unwrap_or(&empty);
    let mut out = Vec::new();
    for pat in patterns {
        for k in matching(&all, pat) {
            out.push(all[k].name.clone());
            out.push(state_of(map, &all[k].name));
        }
    }
    out
}

/// p10k:9046-9051 `p10k display -r` — drop every toggle so the next
/// render shows all parts.
pub fn display_reset() {
    if let Some(map) = DISPLAY_STATE.lock().unwrap().as_mut() {
        map.clear();
    }
}

// --------------------------------------------------------------------
// Usage strings — verbatim from p10k:9169-9330 (leading `%2F`/`%B`
// prompt escapes intact; the dispatcher prints them through
// `promptexpand`, exactly as the theme's `print -rP` does).
// --------------------------------------------------------------------

/// p10k:9169 `__p9k_p10k_usage`.
pub const USAGE: &str = "Usage: %2Fp10k%f %Bcommand%b [options]

Commands:

  %Bconfigure%b  run interactive configuration wizard
  %Breload%b     reload configuration
  %Bsegment%b    print a user-defined prompt segment
  %Bdisplay%b    show, hide or toggle prompt parts
  %Bhelp%b       print this help message

Print help for a specific command:

  %2Fp10k%f %Bhelp%b command";

/// p10k:9183 `__p9k_p10k_segment_usage`.
pub const SEGMENT_USAGE: &str = r##"Usage: %2Fp10k%f %Bsegment%b [-h] [{+|-}re] [-s state] [-b bg] [-f fg] [-i icon] [-c cond] [-t text]

Print a user-defined prompt segment. Can be called only during prompt rendering.

Options:
  -t text   segment's main content; will undergo prompt expansion: '%%F{blue}%%*%%f' will
            show as %F{blue}%*%f; default is empty
  -i icon   segment's icon; default is empty
  -r        icon is a symbolic reference that needs to be resolved; for example, 'LOCK_ICON'
  +r        icon is already resolved and should be printed literally; for example, '⭐';
            this is the default; you can also use $'\u2B50' if you don't want to have
            non-ascii characters in source code
  -b bg     background color; for example, 'blue', '4', or '#0000ff'; empty value means
            transparent background, as in '%%k'; default is black
  -f fg     foreground color; for example, 'blue', '4', or '#0000ff'; empty value means
            default foreground color, as in '%%f'; default is empty
  -s state  segment's state for the purpose of applying styling options; if you want to
            to be able to use POWERLEVEL9K parameters to specify different colors or icons
            depending on some property, use different states for different values of that
            property
  -c        condition; if empty after parameter expansion and process substitution, the
            segment is hidden; this is an advanced feature, use with caution; default is '1'
  -e        segment's main content will undergo parameter expansion and process
            substitution; the content will be surrounded with double quotes and thus
            should quote its own double quotes; this is an advanced feature, use with
            caution
  +e        segment's main content should not undergo parameter expansion and process
            substitution; this is the default
  -h        print this help message

Example: 'core' segment tells you if there is a file name 'core' in the current directory.

- Segment's icon is '⭐'.
- Segment's text is the file's size in bytes.
- If you have permissions to delete the file, state is DELETABLE. If not, it's PROTECTED.

  zmodload -F zsh/stat b:zstat

  function prompt_core() {
    local size=()
    if ! zstat -A size +size core 2>/dev/null; then
      # No 'core' file in the current directory.
      return
    fi
    if [[ -w . ]]; then
      local state=DELETABLE
    else
      local state=PROTECTED
    fi
    p10k segment -s $state -i '⭐' -f blue -t ${size[1]}b
  }

To enable this segment, add 'core' to POWERLEVEL9K_LEFT_PROMPT_ELEMENTS or
POWERLEVEL9K_RIGHT_PROMPT_ELEMENTS.

Example customizations:

  # Override default foreground.
  POWERLEVEL9K_CORE_FOREGROUND=red

  # Override foreground when DELETABLE.
  POWERLEVEL9K_CORE_DELETABLE_BACKGROUND=green

  # Override icon when PROTECTED.
  POWERLEVEL9K_CORE_PROTECTED_VISUAL_IDENTIFIER_EXPANSION='❎'

  # Don't show file size when PROTECTED.
  POWERLEVEL9K_CORE_PROTECTED_CONTENT_EXPANSION=''"##;

/// p10k:8908 `__p9k_p10k_configure_usage`.
pub const CONFIGURE_USAGE: &str = "Usage: %2Fp10k%f %Bconfigure%b

Run interactive configuration wizard.";

/// p10k:8911 `__p9k_p10k_reload_usage`.
pub const RELOAD_USAGE: &str = "Usage: %2Fp10k%f %Breload%b

Reload configuration.";

/// p10k:8915 `__p9k_p10k_finalize_usage`.
pub const FINALIZE_USAGE: &str = "Usage: %2Fp10k%f %Bfinalize%b

Perform the final stage of initialization. Must be called at the very end of zshrc.";

/// p10k:9264 `__p9k_p10k_display_usage`.
pub const DISPLAY_USAGE: &str = r##"Usage: %2Fp10k%f %Bdisplay%b part-pattern=state-list...

  Show, hide or toggle prompt parts. If called from zle, the current
  prompt is refreshed.

Usage: %2Fp10k%f %Bdisplay%b -a [part-pattern]...

  Populate array `reply` with states of prompt parts matching the patterns.
  If no patterns are supplied, assume `*`.

Usage: %2Fp10k%f %Bdisplay%b -r

  Redisplay prompt.

Parts:
  empty_line    empty line (duh)
  ruler         ruler; if POWERLEVEL9K_RULER_CHAR=' ', it's essentially another
                new_line
  N             prompt line number N, 1-based; counting from the top if positive,
                from the bottom if negative
  N/left_frame  left frame on the Nth line
  N/left        left prompt on the Nth line
  N/gap         gap between left and right prompts on the Nth line
  N/right       right prompt on the Nth line
  N/right_frame right frame on the Nth line
  N/left/S      segment S within N/left (dir, time, etc.)
  N/right/S     segment S within N/right (dir, time, etc.)

Part States:
  show          the part is displayed
  hide          the part is not displayed
  print         the part is printed in precmd; only applicable to empty_line and
                ruler; unlike show, the effects of print cannot be undone with hide;
                print used to look better after `clear` but this is no longer the
                case; it's best to avoid it unless you know what you are doing

part-pattern is a glob pattern for parts. Examples:

  */kubecontext         all kubecontext prompt segments, regardless of where
                        they are
  1/(right|right_frame) all prompt segments and frame from the right side of
                        the first line

state-list is a comma-separated list of states. Must have at least one element.
If more than one, states will rotate.

Example: Bind Ctrl+P to toggle right prompt.

  function toggle-right-prompt() { p10k display '*/right'=hide,show; }
  zle -N toggle-right-prompt
  bindkey '^P' toggle-right-prompt

Example: Print the current state of all prompt parts:

  typeset -A reply
  p10k display -a '*'
  printf '%%-32s = %%q\n' ${(@kv)reply} | sort
"##;

/// p10k:9128 — `local var=__p9k_p10k_$2_usage; print -rP ${(P)var}`:
/// the usage string for `p10k help <sub>`, or the top usage.
pub fn help_usage(sub: Option<&str>) -> &'static str {
    match sub {
        None => USAGE,
        Some("segment") => SEGMENT_USAGE,
        Some("configure") => CONFIGURE_USAGE,
        Some("reload") => RELOAD_USAGE,
        Some("finalize") => FINALIZE_USAGE,
        Some("display") => DISPLAY_USAGE,
        Some("help") => USAGE,
        _ => USAGE,
    }
}

/// Print a usage string the way `print -rP` does — prompt-expanded to
/// the caller's fd. `to_stderr` mirrors the theme's `>&2` on the error
/// paths (p10k:8989/9013/…).
pub fn print_usage(s: &str, to_stderr: bool) {
    let (expanded, _, _) = crate::ported::prompt::promptexpand(s, 0, None);
    if to_stderr {
        eprintln!("{expanded}");
    } else {
        println!("{expanded}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() {
        display_reset();
        set_layout(
            vec![vec!["dir".into(), "vcs".into()], vec!["prompt_char".into()]],
            vec![vec!["time".into()], vec![]],
        );
    }

    #[test]
    fn display_hide_show_toggle() {
        let _g = crate::test_util::global_state_lock();
        layout();
        assert!(!is_hidden("1/left/dir"));
        assert!(display_set("1/left/dir", &["hide"]));
        assert!(is_hidden("1/left/dir"));
        // idempotent set → no change.
        assert!(!display_set("1/left/dir", &["hide"]));
        assert!(display_set("1/left/dir", &["show"]));
        assert!(!is_hidden("1/left/dir"));
        // toggle list cycles show→hide.
        assert!(display_set("1/left/dir", &["hide", "show"]));
        assert!(is_hidden("1/left/dir"));
        display_reset();
        assert!(!is_hidden("1/left/dir"));
    }

    #[test]
    fn negative_index_aliases_the_same_slot() {
        let _g = crate::test_util::global_state_lock();
        layout();
        // line -1 is the last line (2): its prompt_char is `2/left/prompt_char`.
        assert!(display_set("-1/left/prompt_char", &["hide"]));
        assert!(is_hidden("2/left/prompt_char"));
        assert!(!is_hidden("1/left/dir"));
        display_reset();
    }

    #[test]
    fn dump_reports_canonical_name_state_pairs() {
        let _g = crate::test_util::global_state_lock();
        layout();
        display_set("1/right/time", &["hide"]);
        assert_eq!(
            display_dump(&["1/right/time"]),
            vec!["1/right/time".to_string(), "hide".to_string()]
        );
        // a glob matches every left part of line 1.
        assert_eq!(
            display_dump(&["1/left/*"]),
            vec!["1/left/dir", "show", "1/left/vcs", "show"]
        );
        display_reset();
    }
}
