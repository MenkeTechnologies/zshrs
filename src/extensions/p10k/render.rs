//! p10k prompt ASSEMBLY — port of `_p9k_left_prompt_segment`
//! (p10k.zsh:614-850), `_p9k_right_prompt_segment` (p10k.zsh:853-1099)
//! and the line assembly of `_p9k_set_prompt` (p10k.zsh:5815-5964) plus
//! the line prefix/suffix construction from `_p9k_init_lines`
//! (p10k.zsh:7946-8049) and prompt prefix bits (p10k.zsh:8097-8146).
//!
//! The zsh original does not render eagerly: each segment call compiles
//! a deferred `${...}`-expansion TEMPLATE (cached in `_p9k_cache`) that
//! zsh's prompt expander evaluates with a runtime state machine
//! (`_p9k__bg`, `_p9k__i`, `_p9k__s`, `_p9k__ss`, `_p9k__sss`,
//! `_p9k__w`, `_p9k_t` slot table). This Rust port renders EAGERLY once
//! per prompt: the state machine collapses into a linear fold over the
//! segment list, with the same case analysis and the same emitted
//! prompt escapes. The `_p9k_cache_get`/`_p9k_cache_set` template cache
//! (p10k:615/834) is therefore unnecessary and omitted.
//!
//! Phase-1 simplifications (each marked TODO at the code site):
//!   - the `p10k display` toggle layer (`${_p9k__<i>l-...}` wrappers)
//!     and instant prompt are not ported;
//!   - CONTENT_EXPANSION / VISUAL_IDENTIFIER_EXPANSION are honored only
//!     in their default passthrough and empty (hide) forms;
//!   - joined segments (`is_joined_name`, separator "case 2") join only
//!     within their own prompt LINE (p10k's `_p9k_left_join`,
//!     p10k:8414-8433, is built over the concatenated multi-line list,
//!     but a line always opens with `bg=NONE` so cross-line joins can
//!     only matter through skipped-anchor chains); SELF_JOINED
//!     (p10k:697-704) is not honored — each Segment renders once so the
//!     multi-sub-segment case it exists for cannot arise;
//!   - non-last-line right prompts are gap-aligned per
//!     `_p9k_build_gap_post` (gap char, gap fg/bg styling,
//!     ZLE_RPROMPT_INDENT, drop-right-on-overflow); only
//!     MULTILINE_*_PROMPT_GAP_EXPANSION (default passthrough), the
//!     `p10k display */gap` toggles and the `_p9k_prompt_overflow_bug`
//!     terminator variant (p10k:8055) are unported.

use crate::extensions::p10k::config::{p9k_global, p9k_param};
use crate::extensions::p10k::icons;
use crate::ported::params::{getiparam, getsparam};
use crate::ported::utils::getkeystring;
use crate::ported::zsh_h::WCWIDTH;

/// One rendered prompt segment, produced by segments_core/segments_env.
/// `content` is already prompt-escaped; `icon` is the resolved glyph;
/// `fg`/`bg` are the segment's DEFAULT color specs (the `$2`/`$3`
/// arguments of `_p9k_left_prompt_segment`, p10k:607-608) which the
/// user's `POWERLEVEL9K_*_FOREGROUND/BACKGROUND` params override.
#[derive(Debug, Clone, Default)]
pub struct Segment {
    pub name: String,
    pub state: Option<String>,
    pub content: String,
    pub icon: Option<String>,
    pub fg: String,
    pub bg: String,
}

/// p10k:5834/5849/5880/5895 —
/// `${${(0)_p9k_line_segments_left[i]}%_joined}`: a prompt ELEMENT name
/// may carry a `_joined` suffix. The suffix is stripped for segment
/// dispatch (the segment function is `prompt_<base>`) and marks the
/// element as joining the PREVIOUS element's group
/// (`_p9k_left_join`/`_p9k_right_join`, p10k:8414-8433).
///
/// Contract with mod.rs: pass the element name to this fn, dispatch the
/// segment builders on the returned base name, and put the ORIGINAL
/// (suffixed) element name back into each built `Segment.name` — render
/// strips the suffix again for all `POWERLEVEL9K_*` param lookups and
/// uses it to select separator "case 2" (p10k:673/683/923).
pub fn is_joined_name(name: &str) -> (&str, bool) {
    match name.strip_suffix("_joined") {
        Some(base) => (base, true),
        None => (name, false),
    }
}

// ---------------------------------------------------------------------------
// Color helpers
// ---------------------------------------------------------------------------

/// p10k:532-541 — `_p9k_translate_color`: decimal codes are left-padded
/// to 3 digits, `#hex` is lowercased, names go through the
/// `__p9k_colors` table (p10k:59-110) after stripping `bg-`/`fg-`/`br`
/// prefixes. Unknown names translate to "" (zsh empty-subscript miss).
fn translate_color(c: &str) -> String {
    if !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()) {
        // p10k:534 — ${(l.3..0.)1}: pad to width 3 with zeros.
        format!("{c:0>3}")
    } else if c.len() > 1 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        // p10k:536 — hexadecimal color code: lowercased.
        c.to_ascii_lowercase()
    } else {
        // p10k:539 — ${${${1#bg-}#fg-}#br}: strip prefixes, then table.
        let key = c.strip_prefix("bg-").unwrap_or(c);
        let key = key.strip_prefix("fg-").unwrap_or(key);
        let key = key.strip_prefix("br").unwrap_or(key);
        named_color(key).unwrap_or("").to_string()
    }
}

/// p10k:59-110 — `typeset -grA __p9k_colors`: the xterm-256 name table.
#[rustfmt::skip]
fn named_color(name: &str) -> Option<&'static str> {
    let code = match name {
        "black" => "000", "red" => "001", "green" => "002", "yellow" => "003",
        "blue" => "004", "magenta" => "005", "cyan" => "006", "white" => "007",
        "grey" => "008", "maroon" => "009", "lime" => "010", "olive" => "011",
        "navy" => "012", "fuchsia" => "013", "aqua" => "014", "teal" => "014",
        "silver" => "015", "grey0" => "016", "navyblue" => "017", "darkblue" => "018",
        "blue3" => "020", "blue1" => "021", "darkgreen" => "022", "deepskyblue4" => "025",
        "dodgerblue3" => "026", "dodgerblue2" => "027", "green4" => "028", "springgreen4" => "029",
        "turquoise4" => "030", "deepskyblue3" => "032", "dodgerblue1" => "033", "darkcyan" => "036",
        "lightseagreen" => "037", "deepskyblue2" => "038", "deepskyblue1" => "039", "green3" => "040",
        "springgreen3" => "041", "cyan3" => "043", "darkturquoise" => "044", "turquoise2" => "045",
        "green1" => "046", "springgreen2" => "047", "springgreen1" => "048", "mediumspringgreen" => "049",
        "cyan2" => "050", "cyan1" => "051", "purple4" => "055", "purple3" => "056",
        "blueviolet" => "057", "grey37" => "059", "mediumpurple4" => "060", "slateblue3" => "062",
        "royalblue1" => "063", "chartreuse4" => "064", "paleturquoise4" => "066", "steelblue" => "067",
        "steelblue3" => "068", "cornflowerblue" => "069", "darkseagreen4" => "071", "cadetblue" => "073",
        "skyblue3" => "074", "chartreuse3" => "076", "seagreen3" => "078", "aquamarine3" => "079",
        "mediumturquoise" => "080", "steelblue1" => "081", "seagreen2" => "083", "seagreen1" => "085",
        "darkslategray2" => "087", "darkred" => "088", "darkmagenta" => "091", "orange4" => "094",
        "lightpink4" => "095", "plum4" => "096", "mediumpurple3" => "098", "slateblue1" => "099",
        "wheat4" => "101", "grey53" => "102", "lightslategrey" => "103", "mediumpurple" => "104",
        "lightslateblue" => "105", "yellow4" => "106", "darkseagreen" => "108", "lightskyblue3" => "110",
        "skyblue2" => "111", "chartreuse2" => "112", "palegreen3" => "114", "darkslategray3" => "116",
        "skyblue1" => "117", "chartreuse1" => "118", "lightgreen" => "120", "aquamarine1" => "122",
        "darkslategray1" => "123", "deeppink4" => "125", "mediumvioletred" => "126", "darkviolet" => "128",
        "purple" => "129", "mediumorchid3" => "133", "mediumorchid" => "134", "darkgoldenrod" => "136",
        "rosybrown" => "138", "grey63" => "139", "mediumpurple2" => "140", "mediumpurple1" => "141",
        "darkkhaki" => "143", "navajowhite3" => "144", "grey69" => "145", "lightsteelblue3" => "146",
        "lightsteelblue" => "147", "darkolivegreen3" => "149", "darkseagreen3" => "150", "lightcyan3" => "152",
        "lightskyblue1" => "153", "greenyellow" => "154", "darkolivegreen2" => "155", "palegreen1" => "156",
        "darkseagreen2" => "157", "paleturquoise1" => "159", "red3" => "160", "deeppink3" => "162",
        "magenta3" => "164", "darkorange3" => "166", "indianred" => "167", "hotpink3" => "168",
        "hotpink2" => "169", "orchid" => "170", "orange3" => "172", "lightsalmon3" => "173",
        "lightpink3" => "174", "pink3" => "175", "plum3" => "176", "violet" => "177",
        "gold3" => "178", "lightgoldenrod3" => "179", "tan" => "180", "mistyrose3" => "181",
        "thistle3" => "182", "plum2" => "183", "yellow3" => "184", "khaki3" => "185",
        "lightyellow3" => "187", "grey84" => "188", "lightsteelblue1" => "189", "yellow2" => "190",
        "darkolivegreen1" => "192", "darkseagreen1" => "193", "honeydew2" => "194", "lightcyan1" => "195",
        "red1" => "196", "deeppink2" => "197", "deeppink1" => "199", "magenta2" => "200",
        "magenta1" => "201", "orangered1" => "202", "indianred1" => "204", "hotpink" => "206",
        "mediumorchid1" => "207", "darkorange" => "208", "salmon1" => "209", "lightcoral" => "210",
        "palevioletred1" => "211", "orchid2" => "212", "orchid1" => "213", "orange1" => "214",
        "sandybrown" => "215", "lightsalmon1" => "216", "lightpink1" => "217", "pink1" => "218",
        "plum1" => "219", "gold1" => "220", "lightgoldenrod2" => "222", "navajowhite1" => "223",
        "mistyrose1" => "224", "thistle1" => "225", "yellow1" => "226", "lightgoldenrod1" => "227",
        "khaki1" => "228", "wheat1" => "229", "cornsilk1" => "230", "grey100" => "231",
        "grey3" => "232", "grey7" => "233", "grey11" => "234", "grey15" => "235",
        "grey19" => "236", "grey23" => "237", "grey27" => "238", "grey30" => "239",
        "grey35" => "240", "grey39" => "241", "grey42" => "242", "grey46" => "243",
        "grey50" => "244", "grey54" => "245", "grey58" => "246", "grey62" => "247",
        "grey66" => "248", "grey70" => "249", "grey74" => "250", "grey78" => "251",
        "grey82" => "252", "grey85" => "253", "grey89" => "254", "grey93" => "255",
        _ => return None,
    };
    Some(code)
}

/// p10k:586-588 — `_p9k_background`: `%K{c}` or `%k` for default.
fn bg_seq(color: &str) -> String {
    if color.is_empty() {
        "%k".to_string()
    } else {
        format!("%K{{{color}}}")
    }
}

/// p10k:590-595 — `_p9k_foreground`: `%F{c}` or `%f` for default.
/// (p10k's own comment: `%1F` would be faster but triggers a zsh
/// percent-expansion bug, hence always the braced form.)
fn fg_seq(color: &str) -> String {
    if color.is_empty() {
        "%f".to_string()
    } else {
        format!("%F{{{color}}}")
    }
}

/// p10k:544-554 — `_p9k_color`: param probe chain, then translate.
fn resolve_color(seg: &Segment, which: &str, default: &str) -> String {
    translate_color(&p9k_param(&seg.name, seg.state.as_deref(), which, default))
}

/// p10k:8390-8396 — `_p9k_color1`/`_p9k_color2` from COLOR_SCHEME:
/// light → (7, 0), anything else → (0, 7). Used for the fg==bg
/// subseparator conflict fallback (p10k:684-689, 1057-1062).
fn scheme_colors() -> (&'static str, &'static str) {
    if p9k_global("COLOR_SCHEME", "dark") == "light" {
        ("7", "0")
    } else {
        ("0", "7")
    }
}

// ---------------------------------------------------------------------------
// Icon lookup
// ---------------------------------------------------------------------------

/// Raw-default sentinel: p10k marks non-user defaults with a leading
/// `$'\1'` so they skip `${(g::)}` processing (p10k:517-522). The same
/// trick discriminates "user param set" from "fall to default" here.
const UNSET: &str = "\u{1}p9k-unset\u{1}";

/// p10k:511-530 — `_p9k_get_icon $1 KEY default`: probe the
/// `POWERLEVEL9K_*` param chain for KEY (segment-scoped when a segment
/// style is given, bare global otherwise); user values get `${(g::)}`
/// echo-escape processing (p10k:524) and the `\b?` zero-width wrap
/// (p10k:525); misses fall to the mode icon table, then to `default`.
fn get_icon(seg: Option<&Segment>, key: &str, default: &str) -> String {
    let user = match seg {
        // p10k:520 — _p9k_param "$1" "$2" …: segment-scoped chain.
        Some(s) => {
            let v = p9k_param(&s.name, s.state.as_deref(), key, UNSET);
            if v == UNSET { None } else { Some(v) }
        }
        // p10k:499-505 — non-`prompt_*` arm: bare POWERLEVEL9K_<KEY>.
        None => getsparam(&format!("POWERLEVEL9K_{key}")),
    };
    match user {
        Some(v) => {
            // p10k:524 — _p9k__ret=${(g::)_p9k__ret}
            let decoded = getkeystring(&v).0;
            // p10k:525 — [[ $_p9k__ret != $'\b'? ]] || _p9k__ret="%{$_p9k__ret%}"
            // — "penance for past sins": backspace + exactly one char
            // becomes a zero-width %{...%} group.
            let mut it = decoded.chars();
            if it.next() == Some('\u{8}') && it.next().is_some() && it.next().is_none() {
                return format!("%{{{decoded}%}}");
            }
            decoded
        }
        None => {
            // p10k:520 — ${icons[$2]-$'\1'$3}: table value if the key
            // exists, else the caller default. icons::icon() returns ""
            // for unknown keys; every table-backed key this module uses
            // (LEFT/RIGHT_(SUB)SEGMENT_SEPARATOR) has a non-empty
            // glyph, so empty == miss is a safe discriminator here.
            let table = icons::icon(key);
            if table.is_empty() {
                default.to_string()
            } else {
                table.to_string()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Small shared helpers
// ---------------------------------------------------------------------------

/// Segment-scoped param with the segment's state in the probe chain.
fn seg_param(seg: &Segment, param: &str, default: &str) -> String {
    p9k_param(&seg.name, seg.state.as_deref(), param, default)
}

/// p10k:752 — `${${(%):-$_p9k__c%1(l.1.0)}[-1]}`: "does this string
/// have prompt-expanded width >= 1?". Eager approximation: skip the
/// zero-width prompt escapes (`%F{..} %K{..} %f %k %b %B %u %U %s %S
/// %E %{...%}`) and report whether anything visible remains. `%%` is a
/// literal percent and counts as visible.
fn visibly_nonempty(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            return true; // any plain char (including space) has width
        }
        match chars.get(i + 1).copied() {
            Some('F') | Some('K') if chars.get(i + 2) == Some(&'{') => {
                // %F{..} / %K{..}
                let mut j = i + 3;
                while j < chars.len() && chars[j] != '}' {
                    j += 1;
                }
                i = j + 1;
            }
            Some('f') | Some('k') | Some('b') | Some('B') | Some('u') | Some('U')
            | Some('s') | Some('S') | Some('E') => i += 2,
            Some('{') => {
                // %{...%} zero-width group
                let mut j = i + 2;
                while j + 1 < chars.len() && !(chars[j] == '%' && chars[j + 1] == '}') {
                    j += 1;
                }
                i = j + 2;
            }
            // %% literal percent, or an escape we don't model
            // (%1(l.…), %n, %~, …) — conservatively visible.
            _ => return true,
        }
    }
    false
}

/// p10k:720-722 / 951-953 — VISUAL_IDENTIFIER_EXPANSION, default
/// `${P9K_VISUAL_IDENTIFIER}` (the resolved icon). Empty hides the
/// icon; any other expansion string is unsupported phase-1.
fn resolved_icon(seg: &Segment) -> String {
    let exp = seg_param(seg, "VISUAL_IDENTIFIER_EXPANSION", "${P9K_VISUAL_IDENTIFIER}");
    if exp.is_empty() {
        return String::new(); // p10k:738/746 — empty expansion → no icon
    }
    if exp != "${P9K_VISUAL_IDENTIFIER}" {
        // TODO(phase-1): arbitrary VISUAL_IDENTIFIER_EXPANSION strings
        // (p10k:735-736) need the zsh expander; fall back to the glyph.
        tracing::debug!(target: "p10k", segment = %seg.name,
            "VISUAL_IDENTIFIER_EXPANSION override not supported; using resolved icon");
    }
    seg.icon.clone().unwrap_or_default()
}

/// p10k:724-726 / 955-957 — CONTENT_EXPANSION, default `${P9K_CONTENT}`.
fn resolved_content(seg: &Segment) -> String {
    let exp = seg_param(seg, "CONTENT_EXPANSION", "${P9K_CONTENT}");
    if exp.is_empty() {
        return String::new();
    }
    if exp != "${P9K_CONTENT}" {
        // TODO(phase-1): arbitrary CONTENT_EXPANSION (p10k:749) needs
        // the zsh expander; pass the segment content through.
        tracing::debug!(target: "p10k", segment = %seg.name,
            "CONTENT_EXPANSION override not supported; using raw content");
    }
    // p10k:749 — ${_p9k__c//$'\r'}: strip carriage returns.
    seg.content.replace('\r', "")
}

/// p10k:5869/5915 —
/// `_p9k__prompt=${${_p9k__prompt//$' %{\b'/'%{%G'}//$' \b'}`:
/// the nerd-font double-width hack; ` %{\b` becomes `%{%G` and any
/// remaining ` \b` pair is deleted.
fn fix_backspace_hack(s: &str) -> String {
    s.replace(" %{\u{8}", "%{%G").replace(" \u{8}", "")
}

// ---------------------------------------------------------------------------
// Left prompt side — port of _p9k_left_prompt_segment (p10k:614-850)
// ---------------------------------------------------------------------------

/// Fold state replacing p10k's `_p9k__bg` / `_p9k__s` / `_p9k__ss` /
/// `_p9k__sss` runtime globals (assigned per segment at p10k:826-829,
/// consumed by the next segment's `_p9k_t` slot, p10k:681-694).
struct LeftState {
    /// `_p9k__bg` — background of the previous segment; None = "NONE"
    /// (line start, p10k:7958 `${_p9k__bg::=NONE}`).
    bg: Option<String>,
    /// `_p9k__s` — `%F{prev_bg}<LEFT_SEGMENT_SEPARATOR>` (p10k:827).
    sep: String,
    /// `_p9k__ss` — previous segment's LEFT_SUBSEGMENT_SEPARATOR.
    subsep: String,
    /// `_p9k__sss` — end-of-line closer `%F{bg}<end symbol>`
    /// (p10k:827), consumed by the line suffix (p10k:7959).
    sss: String,
}

/// Render one left segment, appending to `out`. Mirrors the emission
/// order of the template built at p10k:706-831. `joined` is the
/// resolved case-2 predicate (p10k:696/708-710 — the previously
/// RENDERED segment lies inside this segment's join group). Returns
/// whether the segment rendered (drove `_p9k__i`, p10k:828).
fn render_left_segment(seg: &Segment, joined: bool, st: &mut LeftState, out: &mut String) -> bool {
    // p10k:5895 — `${..%_joined}`: param/icon lookups use the BASE name.
    let stripped;
    let seg = match is_joined_name(&seg.name) {
        (base, true) => {
            stripped = Segment { name: base.to_string(), ..seg.clone() };
            &stripped
        }
        _ => seg,
    };
    let content = resolved_content(seg);
    let icon = resolved_icon(seg);
    let has_content = visibly_nonempty(&content);
    let has_icon = !icon.is_empty();
    // p10k:750-759 — `${${_p9k__e:#00}:+…}`: a segment whose content
    // AND icon are both empty renders nothing (no separators, no state
    // update). This is p10k's guarantee that an empty segment never
    // paints background padding blocks.
    if !has_content && !has_icon {
        return false;
    }

    // p10k:616-624 — resolve bg/fg through the param chain with the
    // segment's own defaults.
    let bg_color = resolve_color(seg, "BACKGROUND", &seg.bg);
    let fg_color = resolve_color(seg, "FOREGROUND", &seg.fg);
    let bg = bg_seq(&bg_color);
    let fg = fg_seq(&fg_color);
    // p10k:626 — style=%b$bg$fg
    let style = format!("%b{bg}{fg}");

    // p10k:629-636 — this segment's separators (published for the NEXT
    // segment via _p9k__s/_p9k__ss, p10k:827).
    let sep = get_icon(Some(seg), "LEFT_SEGMENT_SEPARATOR", "");
    let subsep = get_icon(Some(seg), "LEFT_SUBSEGMENT_SEPARATOR", "");

    // p10k:653-663 — inter/intra-segment whitespace.
    let space = get_icon(Some(seg), "WHITESPACE_BETWEEN_LEFT_SEGMENTS", " ");
    let mut left_space = get_icon(Some(seg), "LEFT_LEFT_WHITESPACE", &space);
    if left_space.contains('%') {
        left_space.push_str(&style); // p10k:658
    }
    let mut right_space = get_icon(Some(seg), "LEFT_RIGHT_WHITESPACE", &space);
    if right_space.contains('%') {
        right_space.push_str(&style); // p10k:663
    }

    // Segment separator logic (p10k:669-694):
    //   if [[ $_p9k__bg == NONE ]]; then 1
    //   elif (( joined )); then          2
    //   elif same-bg; then               3
    //   else                             4
    match &st.bg {
        None => {
            // p10k:645-647 + 682 — first segment: optional start
            // symbol drawn %b%k%F{bg}, then style + leading space.
            // Empty bg_color → %f (fg_seq), never the `%F{}` form
            // (zshrs's prompt expander paints `%F{}` as palette 0).
            let start = get_icon(Some(seg), "LEFT_PROMPT_FIRST_SEGMENT_START_SYMBOL", "");
            if !start.is_empty() {
                out.push_str(&format!("%b%k{}{start}", fg_seq(&bg_color)));
            }
            out.push_str(&style);
            out.push_str(&left_space);
        }
        Some(_) if joined => {
            // Case 2 (p10k:683): joined segment — style only, no
            // separator, no leading whitespace.
            out.push_str(&style);
        }
        Some(prev_bg) => {
            // p10k:675 — $bg_color == (${_p9k__bg}|${_p9k__bg:-0})
            let same_bg = bg_color == *prev_bg || (prev_bg.is_empty() && bg_color == "0");
            if same_bg {
                // Case 3 (p10k:684-693): thin subseparator on the shared
                // background; if fg would vanish into bg, use the scheme
                // contrast color for the subseparator only.
                if !fg_color.is_empty() && fg_color == bg_color {
                    let (c1, c2) = scheme_colors();
                    let alt = if fg_color == c1 { c2 } else { c1 };
                    // p10k:690 — %b$bg%F{alt}$ss$style$left_space
                    out.push_str(&format!("%b{bg}{}", fg_seq(alt)));
                } else {
                    // p10k:692 — %b$bg$ss$style$left_space (subsep keeps
                    // the previous segment's foreground).
                    out.push_str(&format!("%b{bg}"));
                }
                out.push_str(&st.subsep);
                out.push_str(&style);
                out.push_str(&left_space);
            } else {
                // Case 4 (p10k:694): full powerline separator, drawn
                // fg=prev_bg on the new background.
                out.push_str(&format!("%b{bg}"));
                out.push_str(&st.sep);
                out.push_str(&style);
                out.push_str(&left_space);
            }
        }
    }

    // p10k:770-772 / 810-812 — icon styling: VISUAL_IDENTIFIER_COLOR
    // defaults to the segment fg; icon is drawn %b$bg%F{vi}.
    let vi_color = resolve_color(seg, "VISUAL_IDENTIFIER_COLOR", &fg_color);
    let icon_style = format!("%b{bg}{}", fg_seq(&vi_color));
    // p10k:781/803 — LEFT_MIDDLE_WHITESPACE between icon and content,
    // emitted only when BOTH render non-empty (the `e == 11` gate,
    // p10k:785/807).
    let middle = get_icon(Some(seg), "LEFT_MIDDLE_WHITESPACE", " ");

    // p10k:761-762 — ICON_BEFORE_CONTENT: anything but the literal
    // "false" keeps the left-side default of icon first.
    if seg_param(seg, "ICON_BEFORE_CONTENT", "") != "false" {
        // Icon-first branch (p10k:763-791).
        let prefix = getkeystring(&seg_param(seg, "PREFIX", "")).0; // p10k:763-765 ${(g::)}
        out.push_str(&prefix);
        let need_style = prefix.contains('%'); // p10k:767
        if has_icon {
            if icon_style != style {
                // p10k:774-775 — recolor icon, then restore style.
                out.push_str(&icon_style);
                out.push_str(&icon);
                out.push_str(&style);
            } else {
                if need_style {
                    out.push_str(&style); // p10k:777
                }
                out.push_str(&icon); // p10k:778
            }
            if !middle.is_empty() && has_content {
                // p10k:781-786. NOTE: p10k:784 `[[ _p9k__ret == *%* ]]`
                // is missing the `$` — the style re-append after a
                // %-containing middle whitespace never fires in the
                // original; bug preserved for output parity.
                out.push_str(&middle);
            }
        } else if need_style {
            out.push_str(&style); // p10k:787-788
        }
        out.push_str(&content); // p10k:791
        out.push_str(&style);
    } else {
        // Icon-last branch (p10k:792-817).
        let prefix = getkeystring(&seg_param(seg, "PREFIX", "")).0; // p10k:793-796
        out.push_str(&prefix);
        if prefix.contains('%') {
            out.push_str(&style); // p10k:797
        }
        out.push_str(&content); // p10k:799
        out.push_str(&style);
        if has_icon {
            let mut need_style = false; // p10k:802
            if !middle.is_empty() {
                if has_content {
                    out.push_str(&middle); // p10k:807 (e == 11 gate)
                }
                need_style = middle.contains('%'); // p10k:806
            }
            if icon_style != style || need_style {
                out.push_str(&icon_style); // p10k:814
            }
            out.push_str(&icon); // p10k:815 — NOTE: no style restore
                                 // after a trailing icon (faithful).
        }
    }

    // p10k:819-824 — SUFFIX then trailing whitespace.
    let suffix = getkeystring(&seg_param(seg, "SUFFIX", "")).0;
    out.push_str(&suffix);
    if suffix.contains('%') && !right_space.is_empty() {
        out.push_str(&style); // p10k:823
    }
    out.push_str(&right_space);

    // p10k:826-829 — publish separator state for the next segment and
    // the end-of-line closer. Empty bg_color publishes %f, not `%F{}`
    // (p10k:827 interpolates `$bg_color` raw, but only ever with the
    // resolved non-empty palette value on the themes that set one; the
    // `%F{}` form must never reach the expander here).
    // p10k:649 — LEFT_PROMPT_LAST_SEGMENT_END_SYMBOL defaults to the
    // segment separator.
    let end_sep = get_icon(Some(seg), "LEFT_PROMPT_LAST_SEGMENT_END_SYMBOL", &sep);
    st.sep = format!("{}{sep}", fg_seq(&bg_color));
    st.subsep = subsep;
    st.sss = format!("{}{end_sep}", fg_seq(&bg_color));
    st.bg = Some(bg_color);
    true
}

/// Render one full LEFT prompt line (segments + closing separator).
/// Line prefix state per p10k:7958 (`bg=NONE`, default `sss`); line
/// suffix per p10k:7959 (`%b%k$_p9k__sss%b%k%f`).
fn render_left_line(segs: &[Segment]) -> String {
    // p10k:7955-7958 — default end-of-line closer for a line that
    // renders no segments: `%f` + the empty_line-scoped end symbol
    // (which itself defaults to LEFT_SEGMENT_SEPARATOR).
    let default_sep = get_icon(None, "LEFT_SEGMENT_SEPARATOR", "");
    let empty_line = Segment { name: "empty_line".to_string(), ..Default::default() };
    let default_end =
        get_icon(Some(&empty_line), "LEFT_PROMPT_LAST_SEGMENT_END_SYMBOL", &default_sep);
    let mut st = LeftState {
        bg: None,
        sep: String::new(),
        subsep: String::new(),
        sss: format!("%f{default_end}"),
    };
    let mut body = String::new();
    // p10k:8414-8422 — `_p9k_left_join`: each segment's join-group
    // anchor is itself when not `_joined`, else the previous segment's
    // anchor. p10k:696/710 — case 2 fires when `_p9k__i` (index of the
    // last segment that actually RENDERED, p10k:828) is >= the anchor.
    let mut group_start = 0usize;
    let mut last_rendered: Option<usize> = None;
    for (i, seg) in segs.iter().enumerate() {
        if !is_joined_name(&seg.name).1 {
            group_start = i;
        }
        let joined = last_rendered.is_some_and(|li| li >= group_start);
        if render_left_segment(seg, joined, &mut st, &mut body) {
            last_rendered = Some(i);
        }
    }
    let body = fix_backspace_hack(&body); // p10k:5915
    // p10k:7959 — '%b%k$_p9k__sss%b%k%f'
    format!("{body}%b%k{}%b%k%f", st.sss)
}

// ---------------------------------------------------------------------------
// Right prompt side — port of _p9k_right_prompt_segment (p10k:853-1099)
// ---------------------------------------------------------------------------

/// Right-side fold state. The right prompt DEFERS each segment's
/// trailing whitespace + style into `_p9k__w` (p10k:1055-1067), which
/// the NEXT segment's transition consumes (the `$w` prefix of `_p9k_t`
/// cases 2-4, p10k:923-925); the LAST segment's `_p9k__sss`
/// (p10k:1069-1075) closes the line.
struct RightState {
    bg: Option<String>,
    /// `_p9k__w` — previous segment's deferred
    /// `<style><right_space>%b%K{prev_bg}%F{prev_fg}`.
    w: String,
    /// `_p9k__sss` — line closer from the last rendered segment.
    sss: String,
}

/// Render one right segment, appending to `out` (p10k:853-1099).
/// `joined` is the resolved case-2 predicate (p10k:927/940-941);
/// returns whether the segment rendered.
fn render_right_segment(
    seg: &Segment,
    joined: bool,
    st: &mut RightState,
    out: &mut String,
) -> bool {
    // p10k:5849 — `${..%_joined}`: param/icon lookups use the BASE name.
    let stripped;
    let seg = match is_joined_name(&seg.name) {
        (base, true) => {
            stripped = Segment { name: base.to_string(), ..seg.clone() };
            &stripped
        }
        _ => seg,
    };
    let content = resolved_content(seg);
    let icon = resolved_icon(seg);
    let has_content = visibly_nonempty(&content);
    let has_icon = !icon.is_empty();
    // p10k:980-990 — the e == 00 gate, as on the left: an empty segment
    // renders nothing, never background padding.
    if !has_content && !has_icon {
        return false;
    }

    // p10k:855-867 — colors and style.
    let bg_color = resolve_color(seg, "BACKGROUND", &seg.bg);
    let fg_color = resolve_color(seg, "FOREGROUND", &seg.fg);
    let bg = bg_seq(&bg_color);
    let fg = fg_seq(&fg_color);
    let style = format!("%b{bg}{fg}");

    // p10k:869-876 — separators; the right subseparator gets the style
    // appended when it contains prompt escapes.
    let sep = get_icon(Some(seg), "RIGHT_SEGMENT_SEPARATOR", "");
    let mut subsep = get_icon(Some(seg), "RIGHT_SUBSEGMENT_SEPARATOR", "");
    if subsep.contains('%') {
        subsep.push_str(&style); // p10k:876
    }

    // p10k:893-903 — whitespace.
    let space = get_icon(Some(seg), "WHITESPACE_BETWEEN_RIGHT_SEGMENTS", " ");
    let mut left_space = get_icon(Some(seg), "RIGHT_LEFT_WHITESPACE", &space);
    if left_space.contains('%') {
        left_space.push_str(&style); // p10k:898
    }
    let mut right_space = get_icon(Some(seg), "RIGHT_RIGHT_WHITESPACE", &space);
    if right_space.contains('%') {
        right_space.push_str(&style); // p10k:903
    }

    // Segment separator logic (p10k:909-925), mirrored for the right:
    //   1 line start / 2 joined / 3 same bg / 4 different bg.
    match &st.bg {
        None => {
            // p10k:885-887 + 922 — start symbol defaults to the
            // segment separator, drawn %b%k%F{bg}. Empty bg_color →
            // %f (fg_seq), never the `%F{}` form.
            let start = get_icon(Some(seg), "RIGHT_PROMPT_FIRST_SEGMENT_START_SYMBOL", &sep);
            if !start.is_empty() {
                out.push_str(&format!("%b%k{}{start}", fg_seq(&bg_color)));
            }
            out.push_str(&style);
            out.push_str(&left_space);
        }
        Some(_) if joined => {
            // Case 2 (p10k:923) — $w$style: deferred whitespace, then
            // style only; no separator, no leading whitespace.
            out.push_str(&st.w);
            out.push_str(&style);
        }
        Some(prev_bg) => {
            // p10k:915 — $_p9k__bg == (${bg_color}|${bg_color:-0})
            let same_bg = *prev_bg == bg_color || (bg_color.is_empty() && prev_bg == "0");
            out.push_str(&st.w); // deferred prev whitespace+style ($w)
            if same_bg {
                // p10k:924 — $w$style$subsep$left_space
                out.push_str(&style);
                out.push_str(&subsep);
                out.push_str(&left_space);
            } else {
                // p10k:925 — $w%F{$bg_color}$sep$style$left_space:
                // separator drawn fg=NEW bg on the previous background
                // ($w restored it). Empty bg_color → %f, not `%F{}`.
                out.push_str(&format!("{}{sep}", fg_seq(&bg_color)));
                out.push_str(&style);
                out.push_str(&left_space);
            }
        }
    }

    let vi_color = resolve_color(seg, "VISUAL_IDENTIFIER_COLOR", &fg_color);
    let icon_style = format!("%b{bg}{}", fg_seq(&vi_color));
    let middle = get_icon(Some(seg), "RIGHT_MIDDLE_WHITESPACE", " ");

    // p10k:992-993 — right-side default is content first (icon after)
    // unless ICON_BEFORE_CONTENT is the literal "true".
    if seg_param(seg, "ICON_BEFORE_CONTENT", "") != "true" {
        // Content-first branch (p10k:994-1017).
        let prefix = getkeystring(&seg_param(seg, "PREFIX", "")).0; // p10k:994-997
        out.push_str(&prefix);
        if prefix.contains('%') {
            out.push_str(&style); // p10k:998
        }
        out.push_str(&content); // p10k:1000
        out.push_str(&style);
        if has_icon {
            let mut need_style = false; // p10k:1003
            if !middle.is_empty() {
                if has_content {
                    out.push_str(&middle); // p10k:1008 (e == 11 gate)
                }
                need_style = middle.contains('%'); // p10k:1007
            }
            if icon_style != style || need_style {
                out.push_str(&icon_style); // p10k:1015
            }
            out.push_str(&icon); // p10k:1016 — no style restore
        }
    } else {
        // Icon-first branch (p10k:1019-1047).
        let prefix = getkeystring(&seg_param(seg, "PREFIX", "")).0;
        out.push_str(&prefix);
        let need_style = prefix.contains('%'); // p10k:1023
        if has_icon {
            if icon_style != style {
                out.push_str(&icon_style); // p10k:1030-1031
                out.push_str(&icon);
                out.push_str(&style);
            } else {
                if need_style {
                    out.push_str(&style); // p10k:1033
                }
                out.push_str(&icon); // p10k:1034
            }
            if !middle.is_empty() && has_content {
                // p10k:1037-1042 (same missing-`$` bug at p10k:1040 —
                // no style append after the middle whitespace).
                out.push_str(&middle);
            }
        } else if need_style {
            out.push_str(&style); // p10k:1043-1044
        }
        out.push_str(&content); // p10k:1047
        out.push_str(&style);
    }

    // p10k:1050-1053 — SUFFIX (no style/space handling on the right).
    let suffix = getkeystring(&seg_param(seg, "SUFFIX", "")).0;
    out.push_str(&suffix);

    // p10k:1055-1067 — the deferred `_p9k__w`: trailing whitespace +
    // restore of THIS segment's %b, bg and fg (with the fg==bg
    // conflict fallback of p10k:1057-1062).
    let wfg = if !fg_color.is_empty() && fg_color == bg_color {
        let (c1, c2) = scheme_colors();
        fg_seq(if fg_color == c1 { c2 } else { c1 })
    } else {
        fg.clone() // p10k:1064
    };
    let mut w = String::new();
    if !right_space.is_empty() {
        w.push_str(&style); // p10k:1067 — ${right_space_:+$style_}
    }
    w.push_str(&right_space);
    w.push_str(&format!("%b{bg}{wfg}"));
    st.w = w;

    // p10k:1069-1075 — the line closer `_p9k__sss`. Empty bg_color →
    // %f, not `%F{}`.
    let end_sep = get_icon(Some(seg), "RIGHT_PROMPT_LAST_SEGMENT_END_SYMBOL", "");
    let mut sss = String::new();
    sss.push_str(&style); // p10k:1070
    sss.push_str(&right_space);
    if right_space.contains('%') {
        sss.push_str(&style); // p10k:1071
    }
    if !end_sep.is_empty() {
        sss.push_str(&format!("%k{}{end_sep}{style}", fg_seq(&bg_color))); // p10k:1072-1074
    }
    st.sss = sss;

    st.bg = Some(bg_color); // p10k:1077
    true
}

/// Render one full RIGHT prompt line; "" when nothing rendered
/// (p10k:5871 — `right=` stays empty unless the side produced output).
fn render_right_line(segs: &[Segment]) -> String {
    let mut st = RightState { bg: None, w: String::new(), sss: String::new() };
    let mut body = String::new();
    // p10k:8424-8433 — `_p9k_right_join`: same anchor fold as the left.
    let mut group_start = 0usize;
    let mut last_rendered: Option<usize> = None;
    for (i, seg) in segs.iter().enumerate() {
        if !is_joined_name(&seg.name).1 {
            group_start = i;
        }
        let joined = last_rendered.is_some_and(|li| li >= group_start);
        if render_right_segment(seg, joined, &mut st, &mut body) {
            last_rendered = Some(i);
        }
    }
    if body.is_empty() {
        // TODO(phase-1): _p9k_line_never_empty_right (p10k:7962) —
        // EMPTY_LINE_RIGHT_PROMPT_FIRST_SEGMENT_START_SYMBOL forcing a
        // non-empty right line is not wired.
        return String::new();
    }
    let body = fix_backspace_hack(&body); // p10k:5869
    // p10k:7964 — '$_p9k__sss%b%k%f'
    format!("{body}{}%b%k%f", st.sss)
}

// ---------------------------------------------------------------------------
// Gap alignment — port of _p9k_gap_pre (p10k:8087-8095) +
// _p9k_build_gap_post (p10k:7879-7922)
// ---------------------------------------------------------------------------

/// Strip raw ANSI escape sequences (CSI `ESC[...X`, OSC `ESC]...BEL/ST`,
/// other two-byte `ESC x`) from an already prompt-EXPANDED string, so
/// only printable text remains for width measurement.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match it.peek().copied() {
            Some('[') => {
                // CSI: parameters/intermediates until a final byte @-~.
                it.next();
                for c in it.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                // OSC: until BEL or ST (ESC \).
                it.next();
                let mut prev_esc = false;
                for c in it.by_ref() {
                    if c == '\u{7}' || (prev_esc && c == '\\') {
                        break;
                    }
                    prev_esc = c == '\u{1b}';
                }
            }
            Some(_) => {
                it.next(); // two-byte escape (ESC c, ESC 7, …)
            }
            None => {}
        }
    }
    out
}

/// Visible display width of a zsh PROMPT-ESCAPE string: expand through
/// `promptexpand` (src/ported/prompt.rs:407 — `ns=0` chucks the
/// \x01/\x02 width-ignore markers, matching the `${(%):-...}` form
/// p10k's own width probe uses at p10k:8090), strip the raw ANSI the
/// expansion emitted, then sum `WCWIDTH` (src/ported/zsh_h.rs:4758)
/// over the remainder, counting non-printing (< 0) as 0.
fn prompt_visible_width(s: &str) -> usize {
    let (expanded, _, _) = crate::ported::prompt::promptexpand(s, 0, None);
    strip_ansi(&expanded)
        .chars()
        .map(|c| WCWIDTH(c).max(0) as usize)
        .sum()
}

/// p10k:8126 — `_p9k__ind::=${${ZLE_RPROMPT_INDENT:-1}/#-*/0}`:
/// ZLE_RPROMPT_INDENT, unset/empty → 1, leading `-` (negative) → 0.
/// (The `_p9k_emulate_zero_rprompt_indent` branch at p10k:8111-8124 is
/// a workaround for zsh < 5.7.2 only and is not ported.)
fn rprompt_indent() -> usize {
    match getsparam("ZLE_RPROMPT_INDENT") {
        Some(v) if !v.is_empty() => {
            if v.starts_with('-') {
                0
            } else {
                // Non-numeric behaves like zsh arith on a bare word: 0.
                v.parse().unwrap_or(0)
            }
        }
        _ => 1,
    }
}

/// Gap-align one non-last prompt line: left content, styled gap fill,
/// right content, line terminator. Eager port of `_p9k_build_gap_post`
/// (p10k:7878-7922) plus its consumption in `_p9k_set_prompt`
/// (p10k:5936/5959-5960) and the gap math of `_p9k_gap_pre`
/// (p10k:8087-8094).
///
/// p10k:8094 — `_p9k__m = _p9k__clm - x - _p9k__ind - 1`, where x is
/// the combined visible width of `$_p9k__lprompt$_p9k__rprompt` and
/// `_p9k__ind` is ZLE_RPROMPT_INDENT. p10k:7906-7917 — the emitted
/// template is
/// `$style${${${_p9k__m:#-*}:+<gap>$_p9k__rprompt<term>}:-\n}` plus a
/// `%b%k%f` reset when the gap style is non-empty (p10k:7918):
/// NEGATIVE m DROPS the right side of the line (the `:-\n` arm) —
/// never an inline append. Otherwise the gap is m+1 copies of
/// MULTILINE_{FIRST,NEWLINE}_PROMPT_GAP_CHAR (p10k:7884-7891,
/// validated to display width 1, else ' ') styled with the
/// `multiline_*_prompt_gap` BACKGROUND/FOREGROUND (p10k:7892-7898),
/// and the terminator is `_p9k_t[1+!_p9k__ind]` (p10k:8054 —
/// `_p9k_t=($'\n' $'%{\n%}' '')`: the newline is zero-width-marked
/// when ind==0 because the right side then ends in the terminal's
/// last column). Unported: MULTILINE_*_PROMPT_GAP_EXPANSION
/// (p10k:7900-7910, default passthrough only), the `p10k display
/// */gap` toggles (`_p9k__g`/`_p9k__<i>g`), and the
/// `_p9k_prompt_overflow_bug` `%{%G\n%}` terminator variant
/// (p10k:8055). `measure` is injectable so tests need no live shell
/// state.
fn align_line(
    left: &str,
    right: &str,
    line: usize,
    columns: usize,
    measure: &dyn Fn(&str) -> usize,
) -> String {
    if right.is_empty() {
        // p10k:5959-5960 — `[[ -n $right ]] || _p9k__prompt+=$'\n'`:
        // no right side, no gap machinery.
        return format!("{left}\n");
    }
    // p10k:7879-7883 — line 1 uses the FIRST params, later lines NEWLINE.
    let kind = if line == 0 { "FIRST" } else { "NEWLINE" };
    // p10k:7884-7886 — gap char; `${_p9k__ret:- }`.
    let mut ch = get_icon(None, &format!("MULTILINE_{kind}_PROMPT_GAP_CHAR"), "");
    if ch.is_empty() {
        ch = " ".to_string();
    }
    // p10k:7887-7891 — must be exactly one char of display width 1;
    // p10k warns to stderr, zshrs logs (no terminal chatter).
    if ch.chars().count() != 1 || ch.chars().next().map(WCWIDTH) != Some(1) {
        tracing::warn!(target: "p10k", gap_char = %ch, kind,
            "MULTILINE_*_PROMPT_GAP_CHAR is not one character long; using ' '");
        ch = " ".to_string();
    }
    // p10k:7892-7898 — gap style; an empty color contributes nothing
    // (`[[ -n $_p9k__ret ]] && …`, no %k/%f reset in the style).
    let scope = format!("multiline_{}_prompt_gap", kind.to_ascii_lowercase());
    let mut style = String::new();
    let bgc = translate_color(&p9k_param(&scope, None, "BACKGROUND", ""));
    if !bgc.is_empty() {
        style.push_str(&bg_seq(&bgc));
    }
    let fgc = translate_color(&p9k_param(&scope, None, "FOREGROUND", ""));
    if !fgc.is_empty() {
        style.push_str(&fg_seq(&fgc));
    }
    let reset = if style.is_empty() { "" } else { "%b%k%f" }; // p10k:7918
    let ind = rprompt_indent();
    // p10k:8094 — _p9k__m = _p9k__clm - x - _p9k__ind - 1.
    let m = columns as i64 - (measure(left) + measure(right)) as i64 - ind as i64 - 1;
    if m < 0 {
        // p10k:7906 — `${_p9k__m:#-*}` misses → the `:-\n` arm: the
        // right side of the line is DROPPED, never wrapped inline.
        return format!("{left}{style}\n{reset}");
    }
    // p10k:8054/8060 — terminator `_p9k_t[1+!_p9k__ind]`.
    let term = if ind == 0 { "%{\n%}" } else { "\n" };
    format!(
        "{left}{style}{}{right}{term}{reset}",
        ch.repeat(m as usize + 1) // p10k:7907 — ${(pl.$((_p9k__m+1))..<char>.)}
    )
}

// ---------------------------------------------------------------------------
// Multiline assembly — port of _p9k_set_prompt (p10k:5815-5964) +
// _p9k_init_lines frame handling (p10k:7986-8049)
// ---------------------------------------------------------------------------

/// Multiline frame connector for a left line (p10k:7992-8010,
/// 8029-8038): MULTILINE_{FIRST,NEWLINE,LAST}_PROMPT_PREFIX, applied
/// only when the user param is SET or PROMPT_ON_NEWLINE is on.
///
/// NOTE p10k:7995/8032 — `[[ _p9k__ret == *%* ]]` is missing the `$`
/// (it compares the literal string `_p9k__ret`), so the `%b%k%f`
/// append after a %-containing frame never fires in the original;
/// bug preserved for output parity.
fn left_frame_prefix(line: usize, num_lines: usize) -> String {
    if num_lines < 2 {
        return String::new();
    }
    let key = if line == 0 {
        "MULTILINE_FIRST_PROMPT_PREFIX" // p10k:7992-7999
    } else if line + 1 == num_lines {
        "MULTILINE_LAST_PROMPT_PREFIX" // p10k:8002-8009
    } else {
        "MULTILINE_NEWLINE_PROMPT_PREFIX" // p10k:8029-8037
    };
    let prompt_on_newline = p9k_global("PROMPT_ON_NEWLINE", "") == "true";
    // p10k:7992/8002/8029 — `$+_POWERLEVEL9K_<key> == 1`: the param
    // being SET (even to empty) enables the frame.
    if getsparam(&format!("POWERLEVEL9K_{key}")).is_none() && !prompt_on_newline {
        return String::new();
    }
    get_icon(None, key, "")
}

/// Multiline right frame connector (p10k:8012-8026, 8040-8047):
/// MULTILINE_{FIRST,NEWLINE,LAST}_PROMPT_SUFFIX, appended after the
/// right side of the line. Unlike the prefixes these are NOT gated on
/// the param being set (p10k:8012/8019/8040 probe unconditionally).
fn right_frame_suffix(line: usize, num_lines: usize) -> String {
    if num_lines < 2 {
        return String::new();
    }
    let key = if line == 0 {
        "MULTILINE_FIRST_PROMPT_SUFFIX"
    } else if line + 1 == num_lines {
        "MULTILINE_LAST_PROMPT_SUFFIX"
    } else {
        "MULTILINE_NEWLINE_PROMPT_SUFFIX"
    };
    get_icon(None, key, "")
}

/// Assemble PROMPT and RPROMPT from per-line segment lists.
///
/// p10k:5815-5964 — `_p9k_set_prompt`, rendered eagerly. Lines are
/// pre-split by the caller ("newline" elements); this fn aligns the
/// two sides (p10k:7932-7944), renders each line, and returns zsh
/// prompt-escape strings for the prompt expander (never raw ANSI).
pub fn render_prompt(
    left_lines: &[Vec<Segment>],
    right_lines: &[Vec<Segment>],
) -> (String, String) {
    static EMPTY: Vec<Segment> = Vec::new();

    // p10k:5828 — _POWERLEVEL9K_DISABLE_RPROMPT kills the right side.
    let disable_rprompt = p9k_global("DISABLE_RPROMPT", "") == "true";

    // p10k:7932-7944 — align line counts: extra right lines push empty
    // left lines on TOP; extra left lines get empty right lines
    // appended at the BOTTOM (or on top under RPROMPT_ON_NEWLINE).
    let num_lines = left_lines.len().max(right_lines.len()).max(1);
    let rprompt_on_newline = p9k_global("RPROMPT_ON_NEWLINE", "") == "true";
    let line_pair = |i: usize| -> (&Vec<Segment>, &Vec<Segment>) {
        let left = if num_lines > left_lines.len() {
            // p10k:7935 — left padded with LEADING newline lines.
            let pad = num_lines - left_lines.len();
            if i < pad { &EMPTY } else { &left_lines[i - pad] }
        } else {
            left_lines.get(i).unwrap_or(&EMPTY)
        };
        let right = if num_lines > right_lines.len() && rprompt_on_newline {
            // p10k:7939 — right padded with LEADING newline lines.
            let pad = num_lines - right_lines.len();
            if i < pad { &EMPTY } else { &right_lines[i - pad] }
        } else {
            // p10k:7941 — right padded with TRAILING newline lines.
            right_lines.get(i).unwrap_or(&EMPTY)
        };
        (left, right)
    };

    let mut prompt = String::new();
    // p10k:8109 — _p9k_prompt_prefix_left ends with '%b%k%f'. (The
    // COLUMNS juggling at p10k:8097-8100 and the ZLE_RPROMPT_INDENT
    // workaround at p10k:8111-8126 are template-evaluation machinery
    // with no eager equivalent.)
    prompt.push_str("%b%k%f");

    // p10k:8137-8146 + 6134 — empty line(s) before the prompt when
    // PROMPT_ADD_NEWLINE is on; count from PROMPT_ADD_NEWLINE_COUNT
    // (default 1, declared at p10k:7237).
    if p9k_global("PROMPT_ADD_NEWLINE", "") == "true" {
        let count = p9k_global("PROMPT_ADD_NEWLINE_COUNT", "1")
            .parse::<i64>()
            .unwrap_or(1)
            .max(0);
        for _ in 0..count {
            prompt.push('\n');
        }
    }

    // p10k:8097 — `_p9k__clm::=$COLUMNS` snapshot feeding the gap math
    // (p10k:8094). Unset/zero COLUMNS falls back to 80.
    let columns = match getiparam("COLUMNS") {
        c if c > 0 => c as usize,
        _ => 80,
    };

    let mut rprompt = String::new();
    for i in 0..num_lines {
        let (lsegs, rsegs) = line_pair(i);
        let right = if disable_rprompt { String::new() } else { render_right_line(rsegs) };
        let frame_suffix = right_frame_suffix(i, num_lines);

        // p10k:7998/8008/8035 — frame prefix goes BEFORE the line body.
        let mut line = left_frame_prefix(i, num_lines);
        line.push_str(&render_left_line(lsegs));

        if i + 1 == num_lines {
            prompt.push_str(&line);
            // p10k:5949-5952 — last line: right side becomes RPROMPT.
            // (The _p9k_prompt_prefix_right/_suffix_right COLUMNS
            // juggling, p10k:8098-8100, has no eager counterpart.)
            if !right.is_empty() || !frame_suffix.is_empty() {
                rprompt = format!("{right}{frame_suffix}");
            }
        } else {
            // p10k:5918-5947 — non-last lines with right content are
            // gap-aligned: left, styled gap fill, right, terminator
            // (p10k:8087-8095 + 7878-7922; align_line also emits the
            // bare-newline fallbacks of p10k:7906/5960). The right
            // frame connector rides with the right content, as in
            // p10k's `_p9k_line_suffix_right` (p10k:8015/8044).
            let right_full = format!("{right}{frame_suffix}");
            prompt.push_str(&align_line(&line, &right_full, i, columns, &prompt_visible_width));
        }
    }

    (prompt, rprompt)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(name: &str, content: &str, fg: &str, bg: &str) -> Segment {
        Segment {
            name: name.to_string(),
            state: None,
            content: content.to_string(),
            icon: None,
            fg: fg.to_string(),
            bg: bg.to_string(),
        }
    }

    /// Serialize against paramtab-touching tests (same pattern as the
    /// sibling config.rs test module). No params are set here — the
    /// icon lookups must fall through to the static nerdfont table.
    fn locked<F: FnOnce()>(body: F) {
        let _g = crate::test_util::global_state_lock();
        body();
    }

    /// Two same-bg left segments: the boundary must use the thin
    /// LEFT_SUBSEGMENT_SEPARATOR (U+E0B1) and never a full
    /// %F{prev_bg} separator transition (p10k:684-693).
    #[test]
    fn left_same_bg_uses_subsegment_separator() {
        locked(|| {
            let a = seg("aseg", "AAA", "000", "004");
            let b = seg("bseg", "BBB", "001", "004");
            let line = render_left_line(&[a, b]);

            // First segment opens with its style (bg 004, fg 000).
            assert!(line.contains("%b%K{004}%F{000}"), "line: {line}");
            // Same-bg boundary: %b%K{004} then U+E0B1 then B's style.
            assert!(
                line.contains("%b%K{004}\u{E0B1}%b%K{004}%F{001}"),
                "subsegment boundary missing: {line}"
            );
            // No full separator (U+E0B0) may appear BETWEEN the
            // segments — only the end-of-line closer uses it.
            let boundary = &line[..line.find("BBB").expect("BBB rendered")];
            assert!(
                !boundary.contains('\u{E0B0}'),
                "full separator leaked into same-bg boundary: {line}"
            );
            // p10k:7959 — line closes %b%k%F{004}<end sep>%b%k%f, end
            // symbol defaulting to LEFT_SEGMENT_SEPARATOR (p10k:649).
            assert!(
                line.ends_with("%b%k%F{004}\u{E0B0}%b%k%f"),
                "line closer wrong: {line}"
            );
        });
    }

    /// Two different-bg left segments: the boundary is the full
    /// LEFT_SEGMENT_SEPARATOR drawn fg=prev_bg on the NEW background
    /// (p10k:694 + :827 — %b%K{new}%F{prev}<sep><new style>).
    #[test]
    fn left_diff_bg_full_separator_prev_fg_next_bg() {
        locked(|| {
            let a = seg("aseg", "AAA", "000", "004");
            let b = seg("bseg", "BBB", "007", "002");
            let line = render_left_line(&[a, b]);

            assert!(
                line.contains("%b%K{002}%F{004}\u{E0B0}%b%K{002}%F{007}"),
                "diff-bg boundary (fg=prev_bg on next_bg) missing: {line}"
            );
            // Closer carries the LAST segment's background.
            assert!(
                line.ends_with("%b%k%F{002}\u{E0B0}%b%k%f"),
                "line closer wrong: {line}"
            );
        });
    }

    /// Right side, different bg: separator (U+E0B2) drawn fg=NEW bg on
    /// the PREVIOUS background — the deferred `_p9k__w` restored the
    /// previous style first (p10k:925, 1067).
    #[test]
    fn right_diff_bg_separator_new_fg_on_prev_bg() {
        locked(|| {
            let a = seg("aseg", "AAA", "000", "004");
            let b = seg("bseg", "BBB", "007", "002");
            let line = render_right_line(&[a, b]);

            // Deferred w of A: style + ' ' + %b%K{004}%F{000}, then B's
            // separator recolored to B's bg while still on A's bg.
            assert!(
                line.contains("%b%K{004}%F{000}%F{002}\u{E0B2}%b%K{002}%F{007}"),
                "right diff-bg boundary missing: {line}"
            );
            assert!(line.ends_with("%b%k%f"), "right closer wrong: {line}");
        });
    }

    /// Right side, same bg: thin RIGHT_SUBSEGMENT_SEPARATOR (U+E0B3)
    /// after the deferred w, in the NEW segment's style (p10k:924).
    #[test]
    fn right_same_bg_uses_subsegment_separator() {
        locked(|| {
            let a = seg("aseg", "AAA", "000", "004");
            let b = seg("bseg", "BBB", "001", "004");
            let line = render_right_line(&[a, b]);

            assert!(
                line.contains("%b%K{004}%F{000}%b%K{004}%F{001}\u{E0B3}"),
                "right same-bg subseparator missing: {line}"
            );
            let boundary = &line[..line.find("BBB").expect("BBB rendered")];
            // U+E0B2 appears once as the line-start symbol (default =
            // segment separator, p10k:885); it must NOT appear again
            // between the two same-bg segments.
            assert_eq!(
                boundary.matches('\u{E0B2}').count(),
                1,
                "full right separator leaked into same-bg boundary: {line}"
            );
        });
    }

    /// Segments with empty content and no icon render NOTHING — no
    /// separators, no state change (the e == 00 gate, p10k:750-759).
    #[test]
    fn empty_segment_is_skipped_entirely() {
        locked(|| {
            let a = seg("aseg", "AAA", "000", "004");
            let hidden = seg("hseg", "", "007", "001");
            let b = seg("bseg", "BBB", "001", "004");
            let line = render_left_line(&[a, hidden, b]);
            // a and b are same-bg once hidden drops out: thin
            // subseparator boundary, and bg 001 never appears.
            assert!(line.contains('\u{E0B1}'), "line: {line}");
            assert!(!line.contains("%K{001}"), "hidden segment leaked: {line}");
        });
    }

    /// render_prompt: single line → PROMPT gets the %b%k%f prefix and
    /// the left line; RPROMPT carries the right side.
    #[test]
    fn render_prompt_single_line_shape() {
        locked(|| {
            let left = vec![vec![seg("aseg", "AAA", "000", "004")]];
            let right = vec![vec![seg("bseg", "BBB", "007", "002")]];
            let (prompt, rprompt) = render_prompt(&left, &right);
            assert!(prompt.starts_with("%b%k%f"), "prompt: {prompt}");
            assert!(prompt.contains("AAA"), "prompt: {prompt}");
            assert!(!prompt.contains("BBB"), "right leaked into PROMPT: {prompt}");
            assert!(rprompt.contains("BBB"), "rprompt: {rprompt}");
            assert!(rprompt.ends_with("%b%k%f"), "rprompt: {rprompt}");
        });
    }

    /// Multiline: lines joined with \n; only the LAST line's right
    /// side becomes RPROMPT (p10k:5949-5955).
    #[test]
    fn render_prompt_multiline_last_right_is_rprompt() {
        locked(|| {
            let left = vec![
                vec![seg("aseg", "AAA", "000", "004")],
                vec![seg("cseg", "CCC", "000", "006")],
            ];
            let right = vec![Vec::new(), vec![seg("bseg", "BBB", "007", "002")]];
            let (prompt, rprompt) = render_prompt(&left, &right);
            assert_eq!(prompt.matches('\n').count(), 1, "prompt: {prompt}");
            let (line1, line2) = prompt.split_once('\n').expect("two lines");
            assert!(line1.contains("AAA"), "line1: {line1}");
            assert!(line2.contains("CCC"), "line2: {line2}");
            assert!(rprompt.contains("BBB"), "rprompt: {rprompt}");
        });
    }

    /// p10k:532-541 — color translation forms.
    #[test]
    fn translate_color_forms() {
        assert_eq!(translate_color("4"), "004");
        assert_eq!(translate_color("255"), "255");
        assert_eq!(translate_color("#FFAA00"), "#ffaa00");
        assert_eq!(translate_color("red"), "001");
        assert_eq!(translate_color("bg-red"), "001");
        assert_eq!(translate_color("grey50"), "244");
        assert_eq!(translate_color(""), "");
        assert_eq!(translate_color("nosuchcolor"), "");
    }

    /// Zero-width prompt escapes don't count as visible content.
    #[test]
    fn visibly_nonempty_prompt_escapes() {
        assert!(!visibly_nonempty(""));
        assert!(!visibly_nonempty("%f%k%b"));
        assert!(!visibly_nonempty("%F{004}%K{002}"));
        assert!(!visibly_nonempty("%{zero width%}"));
        assert!(visibly_nonempty(" "));
        assert!(visibly_nonempty("%F{004}x"));
        assert!(visibly_nonempty("%%"));
    }

    /// p10k:5869 — the nerd-font backspace hack rewrite.
    #[test]
    fn backspace_hack_rewrite() {
        assert_eq!(fix_backspace_hack(" %{\u{8}X%}"), "%{%GX%}");
        assert_eq!(fix_backspace_hack("a \u{8}b"), "ab");
        assert_eq!(fix_backspace_hack("plain"), "plain");
    }

    /// Empty color specs must emit the reset escapes (%f/%k), never the
    /// empty-brace forms `%F{}`/`%K{}` (zshrs's expander paints those
    /// as palette 0; p10k never emits them).
    #[test]
    fn empty_color_spec_emits_reset_not_empty_braces() {
        assert_eq!(fg_seq(""), "%f");
        assert_eq!(bg_seq(""), "%k");
        locked(|| {
            // Left: transparent-bg segment then a colored one — the
            // published separator recolor and the line closer both
            // derive from the empty bg.
            let a = seg("aseg", "AAA", "004", "");
            let b = seg("bseg", "BBB", "007", "002");
            let line = render_left_line(&[a.clone(), b]);
            assert!(!line.contains("%F{}"), "empty %F brace leaked: {line}");
            assert!(!line.contains("%K{}"), "empty %K brace leaked: {line}");
            // Case-4 boundary: separator fg = prev (empty) bg → %f.
            assert!(
                line.contains("%b%K{002}%f\u{E0B0}"),
                "empty-bg separator not reset via %f: {line}"
            );

            // Transparent-bg segment alone: closer sss = %f + end sep.
            let solo = render_left_line(&[a]);
            assert!(
                solo.ends_with("%b%k%f\u{E0B0}%b%k%f"),
                "empty-bg line closer wrong: {solo}"
            );

            // Right: transparent bg start symbol + closer paths.
            let ra = seg("aseg", "AAA", "004", "");
            let rline = render_right_line(&[ra]);
            assert!(!rline.contains("%F{}"), "empty %F brace leaked: {rline}");
            assert!(!rline.contains("%K{}"), "empty %K brace leaked: {rline}");
            assert!(
                rline.starts_with("%b%k%f\u{E0B2}"),
                "empty-bg right start symbol not reset via %f: {rline}"
            );
        });
    }

    /// The e == 00 gate on the RIGHT side, including an icon that is
    /// Some("") — no separators, no background, no state change
    /// (p10k:980-990).
    #[test]
    fn right_empty_segment_is_skipped_entirely() {
        locked(|| {
            let a = seg("aseg", "AAA", "000", "004");
            let mut hidden = seg("hseg", "", "007", "001");
            hidden.icon = Some(String::new());
            let b = seg("bseg", "BBB", "001", "004");
            let line = render_right_line(&[a, hidden, b]);
            assert!(line.contains('\u{E0B3}'), "line: {line}");
            assert!(!line.contains("%K{001}"), "hidden segment leaked: {line}");
        });
    }

    /// A line of ONLY empty segments renders no segment body at all.
    #[test]
    fn all_empty_segments_render_no_body() {
        locked(|| {
            let h1 = seg("hseg", "", "007", "001");
            let h2 = seg("iseg", "", "003", "002");
            let left = render_left_line(&[h1.clone(), h2.clone()]);
            // Only the p10k:7959 empty-line closer remains.
            assert_eq!(left, "%b%k%f\u{E0B0}%b%k%f", "left: {left}");
            assert_eq!(render_right_line(&[h1, h2]), "", "right must stay empty");
        });
    }

    /// p10k:5834/5849 — `_joined` suffix parsing.
    #[test]
    fn is_joined_name_suffix() {
        assert_eq!(is_joined_name("dir"), ("dir", false));
        assert_eq!(is_joined_name("vcs_joined"), ("vcs", true));
        assert_eq!(is_joined_name("_joined"), ("", true));
        assert_eq!(is_joined_name("joined"), ("joined", false));
    }

    /// Joined left segment (case 2, p10k:683): style only — no full
    /// separator, no subseparator, no leading whitespace — even across
    /// DIFFERENT backgrounds.
    #[test]
    fn left_joined_segment_has_no_separator() {
        locked(|| {
            let a = seg("aseg", "AAA", "000", "004");
            let b = seg("bseg_joined", "BBB", "007", "002");
            let line = render_left_line(&[a, b]);
            let boundary = {
                let s = line.find("AAA").expect("AAA rendered") + 3;
                let e = line.find("BBB").expect("BBB rendered");
                &line[s..e]
            };
            assert!(
                !boundary.contains('\u{E0B0}') && !boundary.contains('\u{E0B1}'),
                "separator leaked into joined boundary: {line}"
            );
            // A's trailing whitespace, then B's bare style (case 2).
            assert!(
                boundary.ends_with(" %b%K{002}%F{007}"),
                "joined boundary must be prev right_space + style: {line}"
            );
            // Param/icon lookups used the BASE name: closer still the
            // stock separator glyph for bg 002.
            assert!(line.ends_with("%b%k%F{002}\u{E0B0}%b%k%f"), "closer: {line}");
        });
    }

    /// p10k:696 — case 2 needs the last RENDERED segment inside the
    /// join group: when the group's anchor is skipped (empty), the
    /// joined segment falls back to the normal separator against the
    /// segment before the group.
    #[test]
    fn left_joined_falls_back_when_anchor_skipped() {
        locked(|| {
            let a = seg("aseg", "AAA", "000", "004");
            let anchor = seg("hseg", "", "007", "001"); // skipped
            let c = seg("cseg_joined", "CCC", "007", "002");
            let line = render_left_line(&[a, anchor, c]);
            // Full separator between AAA and CCC (case 4, diff bg).
            assert!(
                line.contains("%b%K{002}%F{004}\u{E0B0}%b%K{002}%F{007}"),
                "fallback separator missing: {line}"
            );
        });
    }

    /// Joined right segment (case 2, p10k:923): deferred $w, then
    /// style only — no separator glyphs in the boundary.
    #[test]
    fn right_joined_segment_has_no_separator() {
        locked(|| {
            let a = seg("aseg", "AAA", "000", "004");
            let b = seg("bseg_joined", "BBB", "007", "002");
            let line = render_right_line(&[a, b]);
            let boundary = {
                let s = line.find("AAA").expect("AAA rendered") + 3;
                let e = line.find("BBB").expect("BBB rendered");
                &line[s..e]
            };
            assert!(
                !boundary.contains('\u{E0B2}') && !boundary.contains('\u{E0B3}'),
                "separator leaked into joined right boundary: {line}"
            );
            // $w (A's deferred space + %b bg fg restore) + B's style.
            assert!(
                boundary.contains("%b%K{004}%F{000}%b%K{002}%F{007}"),
                "joined right boundary must be w + style: {line}"
            );
        });
    }

    /// Right-alignment padding math (p10k:8087-8094 + 7906-7917),
    /// injectable width fn. Default ZLE_RPROMPT_INDENT (unset → 1,
    /// p10k:8126) keeps the right edge one column in; overflow (m < 0)
    /// DROPS the right side (`:-\n` arm), never inline-appends.
    #[test]
    fn align_line_padding_math() {
        locked(|| {
            let measure = |s: &str| s.chars().count();
            // m = 10-5-1-1 = 3 → 4 gap chars, right ends at column 9.
            assert_eq!(align_line("LL", "RRR", 0, 10, &measure), "LL    RRR\n");
            // m == 0 → exactly one pad char.
            assert_eq!(align_line("LLL", "RRRRR", 0, 10, &measure), "LLL RRRRR\n");
            // m == -1 → right dropped.
            assert_eq!(align_line("LLLL", "RRRRR", 0, 10, &measure), "LLLL\n");
            // Hard overflow → right dropped.
            assert_eq!(align_line("LLLLLLLL", "RRRRR", 0, 10, &measure), "LLLLLLLL\n");
            // Empty right → left + newline, no gap machinery
            // (p10k:5960).
            assert_eq!(align_line("LL", "", 0, 10, &measure), "LL\n");
        });
    }

    /// Gap char + gap foreground + ZLE_RPROMPT_INDENT=0 — the shape the
    /// user config produces (GAP_CHAR '─', GAP_FOREGROUND 244): styled
    /// fill, zero-width-marked newline terminator (`_p9k_t[2]`,
    /// p10k:8054), trailing %b%k%f reset (p10k:7918).
    #[test]
    fn align_line_gap_char_style_and_indent() {
        locked(|| {
            use crate::ported::params::{setsparam, unsetparam};
            setsparam("POWERLEVEL9K_MULTILINE_FIRST_PROMPT_GAP_CHAR", "─");
            setsparam("POWERLEVEL9K_MULTILINE_FIRST_PROMPT_GAP_FOREGROUND", "244");
            setsparam("ZLE_RPROMPT_INDENT", "0");
            let measure = |s: &str| s.chars().count();
            let first = align_line("LL", "RRR", 0, 10, &measure);
            // Line 2+ uses the NEWLINE params (p10k:7879-7883), which
            // are unset here → plain ' ' fill, no style, no reset.
            let newline = align_line("LL", "RRR", 1, 10, &measure);
            // Overflow still emits the style + reset around the bare
            // newline (p10k:7906 — style precedes the `:-\n` arm).
            let overflow = align_line("LLLLLLLL", "RRRRR", 0, 10, &measure);
            unsetparam("POWERLEVEL9K_MULTILINE_FIRST_PROMPT_GAP_CHAR");
            unsetparam("POWERLEVEL9K_MULTILINE_FIRST_PROMPT_GAP_FOREGROUND");
            unsetparam("ZLE_RPROMPT_INDENT");
            // m = 10-5-0-1 = 4 → 5 fill chars, right flush at column 10.
            assert_eq!(first, "LL%F{244}─────RRR%{\n%}%b%k%f");
            assert_eq!(newline, "LL     RRR%{\n%}");
            assert_eq!(overflow, "LLLLLLLL%F{244}\n%b%k%f");
        });
    }

    /// ANSI stripping for the width probe.
    #[test]
    fn strip_ansi_sequences() {
        assert_eq!(strip_ansi("plain"), "plain");
        assert_eq!(strip_ansi("\u{1b}[38;5;4mabc\u{1b}[0m"), "abc");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}abc"), "abc");
        assert_eq!(strip_ansi("a\u{1b}7b"), "ab");
    }

    /// Visible width of prompt-escape strings: zero-width escapes drop,
    /// %% is one column, colored text keeps only its glyph widths.
    #[test]
    fn prompt_visible_width_measures_display_columns() {
        locked(|| {
            assert_eq!(prompt_visible_width(""), 0);
            assert_eq!(prompt_visible_width("abc"), 3);
            assert_eq!(prompt_visible_width("%%"), 1);
            assert_eq!(prompt_visible_width("%F{004}abc%f"), 3);
            assert_eq!(prompt_visible_width("%K{002}%B x %b%k"), 3);
        });
    }
}
