//! `WizardSettings` — the answer state the interactive wizard collects
//! (internal/wizard.zsh:2003-2018 `local` init block) and feeds to
//! `generate_config`. Field names track the zsh locals.

/// wizard:833-866 — the prompt style. `lean_8colors` is the automatic
/// fallback when the terminal has < 256 colors (wizard:834-841).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptStyle {
    Lean,
    Lean8,
    Classic,
    Rainbow,
    Pure,
}

impl PromptStyle {
    /// wizard:1620/1839 — `${style//_/-}`: the `config/p10k-<name>.zsh`
    /// template basename.
    pub fn template_name(self) -> &'static str {
        match self {
            PromptStyle::Lean => "lean",
            PromptStyle::Lean8 => "lean-8colors",
            PromptStyle::Classic => "classic",
            PromptStyle::Rainbow => "rainbow",
            PromptStyle::Pure => "pure",
        }
    }
}

/// The collected wizard answers. Defaults mirror the `local` init at
/// wizard:2003-2018 (the values present before the first `ask_*` runs).
#[derive(Debug, Clone)]
pub struct WizardSettings {
    pub style: PromptStyle,
    /// wizard:2005 `POWERLEVEL9K_MODE` — charset/glyph mode
    /// (nerdfont-complete, powerline, ascii, …).
    pub mode: String,
    /// wizard:2005 `POWERLEVEL9K_ICON_PADDING` (moderate/none).
    pub icon_padding: String,
    /// wizard:2008 `color` index (1-based in zsh; 0-based here) into the
    /// bg/sep/prefix/frame tables.
    pub color: u8,
    /// wizard:2008 `num_lines` (1 or 2).
    pub num_lines: u8,
    /// wizard:2008 `empty_line` — PROMPT_ADD_NEWLINE.
    pub empty_line: bool,
    /// wizard:2008 `left_frame` / `right_frame`.
    pub left_frame: bool,
    pub right_frame: bool,
    /// wizard:2008 `transient_prompt`.
    pub transient_prompt: bool,
    /// wizard:2003 `instant_prompt` (verbose/quiet/off).
    pub instant_prompt: String,
    /// wizard:2006 `gap_char`.
    pub gap_char: String,
    /// wizard:2006 `prompt_char`.
    pub prompt_char: String,
    /// wizard:2072-2091 separators/heads/tails (empty in non-diamond
    /// modes).
    pub left_sep: String,
    pub right_sep: String,
    pub left_subsep: String,
    pub right_subsep: String,
    pub left_head: String,
    pub right_head: String,
    pub left_tail: String,
    pub right_tail: String,
    /// wizard:2007 `time` — None = hidden, else the format sample.
    pub time: Option<String>,
    /// wizard:2010 `extra_icons` (os/dir/vcs style icons).
    pub extra_icons: [String; 3],
    /// wizard:2013 `prefixes` (the "on"/"took" text prefixes).
    pub prefixes: [String; 2],
    /// wizard:2011 `frame_color` table (244 242 240 238, or 0 7 2 4
    /// under 8 colors).
    pub frame_color: [i32; 4],
    /// wizard:2009 capability flags from the font probes.
    pub cap_diamond: bool,
    pub cap_python: bool,
    pub cap_debian: bool,
    pub cap_lock: bool,
    pub cap_arrow: bool,
    /// wizard:2021 `pure_use_rprompt`.
    pub pure_use_rprompt: bool,
    /// wizard:1714 VCS_BRANCH_ICON source glyph.
    pub branch_icon: String,
    /// wizard:2015-2019 pure palette (truecolor snazzy vs original).
    pub has_truecolor: bool,
    /// wizard:2014 `options` — the wizard-choice breadcrumb for the
    /// header comment.
    pub options: Vec<String>,
}

impl WizardSettings {
    /// Defaults for a given style — the wizard's per-iteration `local`
    /// init (wizard:2003-2013) plus the style selection.
    pub fn for_style(style: PromptStyle) -> Self {
        WizardSettings {
            style,
            mode: String::new(),
            icon_padding: "moderate".to_string(), // wizard:2005
            color: 1,                             // wizard:2008 color=2 (1-based) → idx 1
            num_lines: 2,                         // wizard:2008
            empty_line: false,
            left_frame: true, // wizard:2008 left_frame=1
            right_frame: true,
            transient_prompt: false,
            instant_prompt: "verbose".to_string(), // wizard:2003
            gap_char: " ".to_string(),             // wizard:2006
            prompt_char: "❯".to_string(),          // wizard:2006
            left_sep: String::new(),
            right_sep: String::new(),
            left_subsep: String::new(),
            right_subsep: String::new(),
            left_head: String::new(),
            right_head: String::new(),
            left_tail: String::new(),
            right_tail: String::new(),
            time: None,
            extra_icons: [String::new(), String::new(), String::new()], // wizard:2010
            prefixes: [String::new(), String::new()],                   // wizard:2013
            frame_color: [244, 242, 240, 238],                          // wizard:2011
            cap_diamond: false,
            cap_python: false,
            cap_debian: false,
            cap_lock: false,
            cap_arrow: false,
            pure_use_rprompt: false,
            branch_icon: "\u{f126}".to_string(), // default nerdfont branch glyph
            has_truecolor: false,
            options: Vec::new(),
        }
    }

    /// wizard:28-29 — the pure-style palette entry for `name`, snazzy
    /// (truecolor) or original (8-color).
    pub fn pure_color(&self, name: &str) -> &'static str {
        if self.has_truecolor {
            match name {
                "grey" => "242",
                "red" => "#FF5C57",
                "yellow" => "#F3F99D",
                "blue" => "#57C7FF",
                "magenta" => "#FF6AC1",
                "cyan" => "#9AEDFE",
                "white" => "#F1F1F0",
                _ => "242",
            }
        } else {
            match name {
                "grey" => "242",
                "red" => "1",
                "yellow" => "3",
                "blue" => "4",
                "magenta" => "5",
                "cyan" => "6",
                "white" => "7",
                _ => "242",
            }
        }
    }
}
