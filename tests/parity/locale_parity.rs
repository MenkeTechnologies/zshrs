//! Runtime locale-assignment parity.
//!
//! zsh installs a GSU setfn on `$LANG`, `$LC_ALL` and each `$LC_*`
//! (`Src/params.c:257-262` `lang_gsu`/`lc_all_gsu`/`lc_blah_gsu`, registered
//! at `Src/params.c:332-341`), so assigning one of them at RUNTIME calls
//! `setlocale()` and every later libc-driven decision — `mbrtowc`,
//! `iswprint`, `strcoll` — moves with it.
//!
//! The observable consequence tested here is `${(q)}`: `quotestring`
//! (`Src/utils.c:6422-6442`) passes a character through verbatim when
//! `WC_ISPRINT(cc)` holds and escapes it as `$'\NNN'` otherwise, and whether a
//! UTF-8 sequence decodes as ONE character or three Latin-1 bytes is a
//! property of the C library locale. A shell that never re-runs `setlocale`
//! keeps quoting `日本語` as `346$'\227'245…` after `export LC_ALL=<utf8>`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}

fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A UTF-8 locale this host actually has, or None.
///
/// `locale -a` is POSIX and present on both macOS and every Linux image that
/// ships locales at all; a host with none (a bare container) makes the whole
/// question unmeasurable, so the tests below skip rather than guess.
fn utf8_locale() -> Option<String> {
    let out = Command::new("locale").arg("-a").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| {
            let low = l.to_ascii_lowercase();
            low.ends_with("utf-8") || low.ends_with("utf8")
        })
        .map(str::to_string)
        .collect();
    // Prefer en_US so the collation table is the same one the developer sees.
    names.sort_by_key(|n| !n.starts_with("en_US"));
    names.into_iter().next()
}

fn run(bin: &str, args: &[&str], script: &str) -> Vec<u8> {
    let mut cmd = Command::new(bin);
    cmd.args(args).arg(script);
    // Both shells start in a SINGLE-BYTE locale, so the only thing that can
    // move them to UTF-8 is the assignment inside the script.
    cmd.env("LC_ALL", "C").env("LANG", "C");
    cmd.env_remove("ZSHRS_CACHE");
    cmd.output().expect("shell run").stdout
}

fn assert_locale_parity(script: &str) {
    if !zsh_available() {
        return;
    }
    let want = run(zsh_path(), &["-f", "-c"], script);
    let got = run(
        zshrs_bin().to_str().expect("bin path"),
        &["--zsh", "-f", "-c"],
        script,
    );
    assert_eq!(
        String::from_utf8_lossy(&want),
        String::from_utf8_lossy(&got),
        "script: {script}",
    );
}

#[test]
fn lc_all_assignment_reaches_the_c_library_locale() {
    let Some(loc) = utf8_locale() else { return };
    // `q1` warms whatever one-shot locale read the implementation caches
    // BEFORE the assignment, which is the case that regressed: a cold shell
    // reads the environment lazily and happens to be right.
    assert_locale_parity(&format!(
        "a=日本語; q1=${{(q)a}}; export LC_ALL={loc}; print -rn -- ${{(q)a}}"
    ));
}

#[test]
fn lang_assignment_reaches_the_c_library_locale() {
    let Some(loc) = utf8_locale() else { return };
    // `LC_ALL` must go first: `langsetfn` -> `setlang` returns early while a
    // non-empty `$LC_ALL` is set (Src/params.c:4797).
    assert_locale_parity(&format!(
        "a=日本語; q1=${{(q)a}}; unset LC_ALL; export LANG={loc}; print -rn -- ${{(q)a}}"
    ));
}

#[test]
fn lc_ctype_assignment_reaches_the_c_library_locale() {
    let Some(loc) = utf8_locale() else { return };
    assert_locale_parity(&format!(
        "a=日本語; q1=${{(q)a}}; unset LC_ALL; export LC_CTYPE={loc}; print -rn -- ${{(q)a}}"
    ));
}

/// `${(m)#}` / `${(ml:…:)}` widths are a LOCALE decision, not a Unicode one.
///
/// `MB_METASTRLEN2` is `mb_metastrlenend(str, multi_width, NULL)`
/// (`Src/zsh.h:3281`), whose first act is
/// `if (!isset(MULTIBYTE) || MB_CUR_MAX == 1) return ztrlen(ptr)`
/// (`Src/utils.c:5662-5663`) — so in a SINGLE-BYTE locale every length is a
/// BYTE count and the width argument is never consulted. The port kept only
/// the `MULTIBYTE` half of that guard, ran its `mbrtowc` loop anyway, and
/// charged the C1 bytes (0x80-0x9f) of a UTF-8 sequence zero columns:
/// `${(m)#日本語}` answered 6 under `LC_ALL=C` where zsh answers 9.
///
/// `multi_width` is also an `int`, not a flag: `width == 1` adds the glyph's
/// columns and `width >= 2` adds one per printable character
/// (`Src/utils.c:5722-5725`), which is what separates `(m)` from `(mm)`.
///
/// Every expectation here is real zsh's own stdout for the same script, so
/// the four locales pin behaviour rather than a remembered number.
mod multibyte_width_is_locale_driven {
    use super::{zsh_available, zsh_path, zshrs_bin};
    use std::process::Command;

    /// Locales this host actually has. A host without one skips that case
    /// instead of guessing, exactly as `utf8_locale` above does.
    fn have_locale(name: &str) -> bool {
        let Ok(out) = Command::new("locale").arg("-a").output() else {
            return false;
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.trim().eq_ignore_ascii_case(name))
    }

    fn run_in(bin: &str, locale: &str, script: &str) -> Vec<u8> {
        let o = Command::new(bin)
            .args(["-f", "-c", script])
            .env_remove("LC_CTYPE")
            .env_remove("LANG")
            .env_remove("ZSHRS_CACHE")
            .env("LC_ALL", locale)
            .output()
            .expect("shell");
        o.stdout
    }

    /// zsh is the oracle: both shells run the identical script in the
    /// identical locale and must emit identical bytes.
    fn assert_parity_in(locale: &str, script: &str) {
        if !zsh_available() || !have_locale(locale) {
            return;
        }
        let want = run_in(zsh_path(), locale, script);
        let got = run_in(zshrs_bin().to_str().expect("path"), locale, script);
        assert_eq!(
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(&want),
            "locale {locale}, script {script}"
        );
    }

    const LOCALES: [&str; 4] = ["C", "en_US.UTF-8", "zh_CN.GB2312", "ja_JP.eucJP"];

    /// Character count, column width and the `(mm)` per-character count, for
    /// a narrow accented char, an all-wide string and a mixed one. Under
    /// `LC_ALL=C` zsh answers 9/9/9 for `日本語`; under `en_US.UTF-8`, 3/6/3.
    #[test]
    fn count_width_and_mm_agree_with_zsh_in_every_locale() {
        for loc in LOCALES {
            for value in ["é", "日本語", "aé日"] {
                assert_parity_in(
                    loc,
                    &format!("s={value}; print -rn -- ${{#s}}/${{(m)#s}}/${{(mm)#s}}"),
                );
            }
        }
    }

    /// The same length feeds `dopadding` (c:919-923), so a single-byte
    /// locale pads to a BYTE budget: `${(ml:12::x:)日本語}` is `xxx日本語`
    /// in zsh under `LC_ALL=C`, not `xxxxxx日本語`.
    #[test]
    fn m_padding_uses_the_locale_length_in_every_locale() {
        for loc in LOCALES {
            assert_parity_in(
                loc,
                "s=日本語; print -rn -- \"${(ml:12::x:)s}|${(mr:12::y:)s}\"",
            );
        }
    }

    /// The PAD string is measured the same way (c:922-923): a wide pad
    /// character occupies two columns, so `${(ml:10::中:)ab}` repeats it
    /// four times, not eight. `(l)` without `(m)` still counts characters.
    ///
    /// Only the locales that can ENCODE the pad are pinned here. `中` has no
    /// GB2312 or eucJP form, and zsh rejects the whole flag there
    /// (`error in flags near position 25`) where zshrs pads — a flag-parsing
    /// divergence, not a width one, recorded in the ledger rather than
    /// asserted by a width test.
    ///
    /// `LC_ALL=C` is also left to the ledger, for a different reason: there
    /// the pad is three BYTES and C cuts it mid-character, which `(m)`
    /// padding still does differently at the edges — `${(mr:10::中:)ab}` is
    /// `61 62 e4b8ad e4b8ad e4b8` in zsh (the last pad cut after two bytes)
    /// against zshrs's whole final pad, and `${(ml:10::中:)ab}` is two bytes
    /// in zsh. Unflagged `(l)`/`(r)` already match there byte for byte, so
    /// what remains is the `(m)` mid-character cut, not the width
    /// arithmetic this module pins.
    #[test]
    fn a_wide_pad_string_is_measured_in_columns() {
        for loc in ["en_US.UTF-8"] {
            assert_parity_in(
                loc,
                "s=ab; print -rn -- \"${(ml:10::中:)s}|${(mr:10::中:)s}|${(l:10::中:)s}\"",
            );
        }
    }
}
