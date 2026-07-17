//! Native port of `p10k configure` — the powerlevel10k configuration
//! wizard (internal/wizard.zsh, 2153 lines).
//!
//! Structure mirrors the zsh source:
//!   - `settings`   — `WizardSettings`, the answer state (wizard:2003+).
//!   - `config_gen` — `generate_config` (wizard:1619-1867): the
//!     deterministic settings→config-text core.
//!   - `capability` — charset / truecolor / glyph probes (wizard:582+).
//!   - `tui`        — the interactive full-screen driver: `render_screen`,
//!     the `ask_*` question flow, live prompt previews (wizard:101-1617).
//!   - `zshrc`      — `change_zshrc` (wizard:1869-1974).
//!
//! Entry point: `run()` (the wizard's top-level `while true` driver,
//! wizard:2000-2153), invoked by `p10k configure`.

pub mod capability;
pub mod config_gen;
pub mod settings;
pub mod templates;
pub mod tui;
pub mod zshrc;

use settings::WizardSettings;

/// Run `p10k configure`. Drives the interactive TUI to collect answers,
/// generates the config, and (optionally) edits `.zshrc`. Returns the
/// process exit status (0 success, 1 aborted/error).
///
/// Requires the p10k install root (captured at theme-source intercept)
/// to read the `config/p10k-*.zsh` base templates.
pub fn run(force: bool) -> i32 {
    let Some(root) = crate::p10k::p10k_root_dir() else {
        eprintln!(
            "zshrs: p10k: configure: powerlevel10k install root unknown \
             (source the theme first)"
        );
        return 1;
    };
    match tui::run_wizard(&root, force) {
        Ok(Some(outcome)) => {
            if let Err(e) = commit(&root, &outcome) {
                eprintln!("zshrs: p10k: configure: {e}");
                return 1;
            }
            0
        }
        // Ctrl-C / quit: the TUI restored the screen; nothing written.
        Ok(None) => 1,
        Err(e) => {
            eprintln!("zshrs: p10k: configure: {e}");
            1
        }
    }
}

/// The wizard's collected result: the settings to generate from, plus
/// the config path and whether to wire `.zshrc`.
pub struct Outcome {
    pub settings: WizardSettings,
    /// Destination `.p10k.zsh` (wizard:__p9k_cfg_path).
    pub config_path: String,
    /// wizard:2118 `write_zshrc` — edit `.zshrc` to source the config.
    pub write_zshrc: bool,
    pub zshrc_path: String,
    /// The "generated on …" timestamp string for the header.
    pub timestamp: String,
}

/// wizard:2136-2137 — write the config, then optionally edit `.zshrc`.
fn commit(root: &str, o: &Outcome) -> std::io::Result<()> {
    let base_path = format!(
        "{root}/config/p10k-{}.zsh",
        o.settings.style.template_name()
    );
    let base = std::fs::read_to_string(&base_path).map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("cannot read base template {base_path}: {e}"),
        )
    })?;
    let text = config_gen::generate_config(&base, &o.settings, &o.timestamp);

    // wizard:1861-1866 — mkdir -p, replace the file.
    if let Some(dir) = std::path::Path::new(&o.config_path).parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&o.config_path, text)?;

    if o.write_zshrc {
        zshrc::change_zshrc(&o.zshrc_path, &o.config_path)?; // wizard:1869
    }
    Ok(())
}
