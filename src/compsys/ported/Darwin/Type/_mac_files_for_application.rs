//! Port of `_mac_files_for_application` from
//! `Completion/Darwin/Type/_mac_files_for_application`.
//!
//! Full upstream body (75 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  _mac_rsrc_check() {
//! sh: 4    [[ ! -s "$REPLY/..namedfork/rsrc" ]] && return 1
//! sh: 5    if [[ -x /Developer/Tools/GetFileInfo ]]; then
//! sh: 6      local ftype="$(command /Developer/Tools/GetFileInfo -t $REPLY)"
//! sh: 7      ftype="${ftype//\"/}"
//! sh: 8      [[ -n "$types[(r)$ftype]" ]]
//! sh: 9    else
//! sh:10        grep --quiet "\(${(j/\|/)types}\)" "$REPLY/..namedfork/rsrc"
//! sh:11    fi
//! sh:12  }
//! sh:14  _mac_parse_info_plist() {
//! sh:18    local s='
//! sh:19      BEGIN { RS="<" }
//! sh:20        /^key>/ { sub(/key>/, ""); reading_key=$0 }
//! sh:21        /^string>/ {
//! sh:22          sub(/string>/, "")
//! sh:23          if (reading_key == "CFBundleTypeExtensions") exts=exts " \"" $0 "\""
//! sh:24          if (reading_key == "CFBundleTypeOSTypes") types=types " \"" $0 "\""
//! sh:25        }
//! sh:26        END {
//! sh:27          print "exts=(" exts ")\ntypes=(" types ")"
//! sh:28        }
//! sh:29      '
//! sh:30      command awk $s "$app_path/Contents/Info.plist" | while read; do
//! sh:31        eval "$REPLY"
//! sh:32      done
//! sh:33  }
//! sh:36  _mac_files_for_application() {
//! sh:37    local -a opts
//! sh:38    zparseopts -D -a opts q n 1 2 o+: P: S: r: R: W: x+: X+: M+: F: J+: V+:
//! sh:40    local app_path
//! sh:41    _retrieve_mac_apps
//! sh:42    app_path="${_mac_apps[(r)*/$1(|.app)]:-$1}"
//! sh:44    local -a glob_patterns
//! sh:45    glob_patterns=()
//! sh:48    if [[ -f "$app_path/Contents/Info.plist" ]]; then
//! sh:49      local -a exts types
//! sh:50      _mac_parse_info_plist
//! sh:52      if [[ -n "$exts[(r)\*]" ]]; then
//! sh:53        glob_patterns=( "*" )
//! sh:54      else
//! sh:55        if (( #exts != 0 )); then
//! sh:56          glob_patterns+=( "*.(${(j/|/)exts})(N)" )
//! sh:57        fi
//! sh:59        if (( #types != 0 )); then
//! sh:60          glob_patterns+=( "^*.[[:alnum:]]##(.Ne:_mac_rsrc_check:)" )
//! sh:61        fi
//! sh:62      fi
//! sh:63    else
//! sh:64      glob_patterns=( "*" )
//! sh:65    fi
//! sh:67    case ${#glob_patterns} in
//! sh:68      0) return 1 ;;
//! sh:69      1) _files "$opts[@]" -g "$glob_patterns[1]" ;;
//! sh:70      *) _files "$opts[@]" -g "{${(j/,/)glob_patterns}}" ;;
//! sh:71    esac
//! sh:72  }
//! sh:74  _mac_files_for_application "$@"
//! ```
//!
//! `_retrieve_mac_apps` (`Completion/Darwin/Type/_retrieve_mac_apps`) is not
//! yet ported to Rust — invoked via the shell-fallback
//! `dispatch_function_call`, matching `_mac_applications::_mac_applications`
//! (same directory, identical dependency). It populates the global
//! `_mac_apps` array param with application bundle paths.
//!
//! `_mac_rsrc_check` and `_mac_parse_info_plist` are upstream helper
//! functions defined in the same `#autoload` file (zsh source-once-use-many
//! convention). Both are ported below as private Rust fns:
//! - `_mac_parse_info_plist` (sh:14-33) is invoked directly (sh:50) and is
//!   fully ported: same `awk` script, same two-line `NAME=( "a" "b" )`
//!   output shape, parsed directly instead of the shell's `eval`.
//! - `_mac_rsrc_check` (sh:3-12) is referenced only *by name* inside a glob
//!   qualifier string (`(.Ne:_mac_rsrc_check:)`, sh:60) that is handed to
//!   `_files -g` as a literal pattern — the callback wiring from `_files`'s
//!   glob-qualifier evaluator back to a named predicate function does not
//!   exist yet in the Rust port, so this port cannot invoke it during glob
//!   evaluation (same gap as upstream `_files`'s `e:...:` qualifier
//!   support in general). Ported here as `mac_rsrc_check` — a pure fn
//!   taking `$REPLY`/`$types` as explicit parameters instead of zsh's
//!   dynamic scoping — so the logic is faithfully available once that
//!   wiring lands.

use crate::compsys::ported::_files::_files;
use crate::compsys::ported::shared::glob_matches;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::getaparam;

/// sh:3-12 `_mac_rsrc_check` — glob-qualifier predicate for
/// `(.Ne:_mac_rsrc_check:)` (sh:60). Not wired into `_files`'s glob
/// evaluator yet (see module doc); kept callable and tested standalone.
fn mac_rsrc_check(reply: &str, types: &[String]) -> bool {
    // sh:4  [[ ! -s "$REPLY/..namedfork/rsrc" ]] && return 1
    let rsrc_path = format!("{}/..namedfork/rsrc", reply);
    let has_rsrc = std::fs::metadata(&rsrc_path)
        .map(|m| m.len() > 0)
        .unwrap_or(false);
    if !has_rsrc {
        return false;
    }

    // sh:5  if [[ -x /Developer/Tools/GetFileInfo ]]; then
    if crate::compsys::ported::shared::is_executable(std::path::Path::new(
        "/Developer/Tools/GetFileInfo",
    )) {
        // sh:6-7  ftype="$(command /Developer/Tools/GetFileInfo -t $REPLY)"; strip quotes.
        let ftype = std::process::Command::new("/Developer/Tools/GetFileInfo")
            .arg("-t")
            .arg(reply)
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .replace('"', "")
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();
        // sh:8  [[ -n "$types[(r)$ftype]" ]]
        types.iter().any(|t| t == &ftype)
    } else {
        // sh:10  grep --quiet "\(${(j/\|/)types}\)" "$REPLY/..namedfork/rsrc"
        let pattern = format!("\\({}\\)", types.join("\\|"));
        std::process::Command::new("grep")
            .arg("--quiet")
            .arg(&pattern)
            .arg(&rsrc_path)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Parse a run of double-quoted, space-separated words — the shape
/// `_mac_parse_info_plist`'s awk script emits for `exts=(...)`/`types=(...)`
/// (sh:27, printed as `NAME=( "a" "b" … )`).
fn parse_quoted_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            let mut word = String::new();
            for c2 in chars.by_ref() {
                if c2 == '"' {
                    break;
                }
                word.push(c2);
            }
            out.push(word);
        }
    }
    out
}

/// sh:14-33 `_mac_parse_info_plist` — run `awk` against the app's
/// `Info.plist`, extracting `CFBundleTypeExtensions`/`CFBundleTypeOSTypes`
/// string values found under their respective `<key>` runs.
fn mac_parse_info_plist(app_path: &str) -> (Vec<String>, Vec<String>) {
    // sh:18-29 — awk script, verbatim. `RS="<"` makes each XML tag/text
    //   chunk its own record; a `key>NAME` record primes `reading_key`, a
    //   following `string>VALUE` record under a matching key appends a
    //   quoted VALUE to `exts`/`types`.
    let script = r#"BEGIN { RS="<" }
      /^key>/ { sub(/key>/, ""); reading_key=$0 }
      /^string>/ {
        sub(/string>/, "")
        if (reading_key == "CFBundleTypeExtensions") exts=exts " \"" $0 "\""
        if (reading_key == "CFBundleTypeOSTypes") types=types " \"" $0 "\""
      }
      END {
        print "exts=(" exts ")\ntypes=(" types ")"
      }
    "#;
    // sh:30  command awk $s "$app_path/Contents/Info.plist"
    let plist = format!("{}/Contents/Info.plist", app_path);
    let output = std::process::Command::new("awk").arg(script).arg(&plist).output();

    let mut exts = Vec::new();
    let mut types = Vec::new();
    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        // sh:30-32  `| while read; do eval "$REPLY"; done` — each printed
        //   line is `NAME=( "a" "b" … )`; parse the two known shapes
        //   directly instead of a general shell `eval`.
        for line in stdout.lines() {
            if let Some(body) = line.strip_prefix("exts=(").and_then(|s| s.strip_suffix(')')) {
                exts = parse_quoted_words(body);
            } else if let Some(body) = line.strip_prefix("types=(").and_then(|s| s.strip_suffix(')'))
            {
                types = parse_quoted_words(body);
            }
        }
    }
    (exts, types)
}

/// sh:38 — `zparseopts -D -a opts q n 1 2 o+: P: S: r: R: W: x+: X+: M+: F: J+: V+:`
/// pulls every recognized `_files`-style flag (and, for the value-taking
/// ones, its argument) out of `args` into `opts` in `-a` array mode
/// (order/repeats preserved verbatim, exactly as passed to `_files`),
/// leaving the rest — the application name/path positional — in `rest`.
fn zparse_files_opts(args: &[String]) -> (Vec<String>, Vec<String>) {
    const BOOL_FLAGS: &[&str] = &["-q", "-n", "-1", "-2"];
    const VALUE_FLAGS: &[&str] = &[
        "-o", "-P", "-S", "-r", "-R", "-W", "-x", "-X", "-M", "-F", "-J", "-V",
    ];
    let mut opts = Vec::new();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if BOOL_FLAGS.contains(&a) {
            opts.push(args[i].clone());
            i += 1;
        } else if VALUE_FLAGS.contains(&a) {
            opts.push(args[i].clone());
            if i + 1 < args.len() {
                opts.push(args[i + 1].clone());
                i += 2;
            } else {
                i += 1;
            }
        } else {
            rest.push(args[i].clone());
            i += 1;
        }
    }
    (opts, rest)
}

/// `_mac_files_for_application` — complete files an application declares
/// (via `Info.plist`'s `CFBundleTypeExtensions`/`CFBundleTypeOSTypes`, or
/// all files if the app claims `"*"` or has no `Info.plist`) it can open.
pub fn _mac_files_for_application(args: &[String]) -> i32 {
    // sh:37-38  local -a opts; zparseopts -D -a opts ...
    let (opts, rest) = zparse_files_opts(args);

    // sh:41  _retrieve_mac_apps (shell fallback — see module doc).
    let _ = dispatch_function_call("_retrieve_mac_apps", &[]);

    // sh:42  app_path="${_mac_apps[(r)*/$1(|.app)]:-$1}"
    let arg1 = rest.first().cloned().unwrap_or_default();
    let mac_apps = getaparam("_mac_apps").unwrap_or_default();
    let pattern = format!("*/{}(|.app)", arg1);
    let app_path = mac_apps
        .iter()
        .find(|p| glob_matches(&pattern, p))
        .cloned()
        .unwrap_or_else(|| arg1.clone());

    // sh:44-45  local -a glob_patterns; glob_patterns=()
    let mut glob_patterns: Vec<String> = Vec::new();

    // sh:48  if [[ -f "$app_path/Contents/Info.plist" ]]; then
    let info_plist = format!("{}/Contents/Info.plist", app_path);
    if std::path::Path::new(&info_plist).is_file() {
        // sh:49-50  local -a exts types; _mac_parse_info_plist
        let (exts, types) = mac_parse_info_plist(&app_path);

        // sh:52-53  [[ -n "$exts[(r)\*]" ]] && glob_patterns=( "*" )
        if exts.iter().any(|e| e == "*") {
            glob_patterns.push("*".to_string());
        } else {
            // sh:55-57
            if !exts.is_empty() {
                glob_patterns.push(format!("*.({})(N)", exts.join("|")));
            }
            // sh:59-61
            if !types.is_empty() {
                glob_patterns.push("^*.[[:alnum:]]##(.Ne:_mac_rsrc_check:)".to_string());
            }
        }
    } else {
        // sh:64  glob_patterns=( "*" )
        glob_patterns.push("*".to_string());
    }

    // sh:67-71  case ${#glob_patterns} in 0) return 1 ;; 1) … ;; *) … ;; esac
    match glob_patterns.len() {
        0 => 1,
        1 => {
            let mut fargs = opts;
            fargs.push("-g".to_string());
            fargs.push(glob_patterns[0].clone());
            _files(&fargs)
        }
        _ => {
            let mut fargs = opts;
            fargs.push("-g".to_string());
            fargs.push(format!("{{{}}}", glob_patterns.join(",")));
            _files(&fargs)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zparse_pulls_recognized_flags_leaving_rest() {
        // sh:38 — `-q` (bool) and `-o VALUE` (value-taking) go to `opts`;
        //   the app-name positional is left in `rest`.
        let (opts, rest) = zparse_files_opts(&[
            "-q".into(),
            "-o".into(),
            "extended".into(),
            "Safari".into(),
        ]);
        assert_eq!(
            opts,
            vec!["-q".to_string(), "-o".to_string(), "extended".to_string()]
        );
        assert_eq!(rest, vec!["Safari".to_string()]);
    }

    #[test]
    fn zparse_value_flag_with_missing_argument_is_dropped_alone() {
        // A value-taking flag as the last token has no argument to pair
        //   with; it is still consumed into `opts` (zparseopts would error
        //   in the shell, but for this port's purposes it simply carries
        //   the bare flag through).
        let (opts, rest) = zparse_files_opts(&["-P".into()]);
        assert_eq!(opts, vec!["-P".to_string()]);
        assert!(rest.is_empty());
    }

    #[test]
    fn parse_quoted_words_splits_double_quoted_run() {
        // sh:27 — `exts=( "txt" "md" )` shape.
        assert_eq!(
            parse_quoted_words(r#" "txt" "md" "#),
            vec!["txt".to_string(), "md".to_string()]
        );
    }

    #[test]
    fn parse_quoted_words_empty_body_yields_empty_vec() {
        assert_eq!(parse_quoted_words(""), Vec::<String>::new());
    }

    #[test]
    fn mac_rsrc_check_false_when_no_resource_fork() {
        // sh:4 — a plain file has no `..namedfork/rsrc` companion (true on
        //   both macOS, where the path simply won't exist, and Linux CI,
        //   where the pseudo-path is invalid): deterministic `false`.
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "zshrs_test_mac_rsrc_check_{}",
            std::process::id()
        ));
        std::fs::write(&path, b"hello").unwrap();
        let result = mac_rsrc_check(path.to_str().unwrap(), &["APPL".to_string()]);
        let _ = std::fs::remove_file(&path);
        assert!(!result);
    }

    #[test]
    fn returns_one_when_glob_patterns_empty() {
        // sh:67-68 — an `Info.plist` present but declaring neither
        //   `CFBundleTypeExtensions` nor `CFBundleTypeOSTypes` leaves both
        //   `exts` and `types` empty, so neither sh:55-57 nor sh:59-61 adds
        //   a pattern and `glob_patterns` stays empty → return 1.
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::unsetparam("_mac_apps");

        let dir = std::env::temp_dir().join(format!(
            "zshrs_test_mac_files_for_application_{}",
            std::process::id()
        ));
        let contents_dir = dir.join("Contents");
        std::fs::create_dir_all(&contents_dir).unwrap();
        std::fs::write(
            contents_dir.join("Info.plist"),
            "<plist><dict></dict></plist>",
        )
        .unwrap();

        let app_arg = dir.to_string_lossy().to_string();
        let result = _mac_files_for_application(&[app_arg]);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(result, 1);
    }

    #[test]
    fn falls_back_to_star_glob_when_no_info_plist() {
        // sh:63-64 — no `Info.plist` at all ⇒ glob_patterns=( "*" ) ⇒ one
        //   pattern ⇒ `_files` is invoked (sh:69); without a completion
        //   context `_files` returns 1, distinguishable from the
        //   zero-pattern early-return path only by which code path ran —
        //   both currently observably return 1, so this test pins the
        //   "doesn't short-circuit before calling `_files`" behavior via
        //   the INCOMPFUNC-gated non-context return.
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::unsetparam("_mac_apps");
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);

        let dir = std::env::temp_dir().join(format!(
            "zshrs_test_mac_files_for_application_noplist_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let app_arg = dir.to_string_lossy().to_string();
        let result = _mac_files_for_application(&[app_arg]);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(result, 1);
    }
}
