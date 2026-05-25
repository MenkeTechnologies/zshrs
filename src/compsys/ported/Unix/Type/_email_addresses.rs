//! Port of `_email_addresses` from `Completion/Unix/Type/_email_addresses`.
//!
//! Full upstream body (187 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2  # options:
//! sh:  3  #
//! sh:  4  # -n plugin - can complete nicknames from specified plugin
//! sh:  5  # -s sep    - complete a list of addresses separated by specified character
//! sh:  6  # -c        - e-mail address must be of form user@host (no comments or aliases)
//! sh:  7  #
//! sh:  8  # TODO: with -n, have the named plugin complete not only aliases but also addresses?
//! sh:  9  #
//! sh: 10  # Plugins are written as separate functions with names starting `_email-'.
//! sh: 11  # They should either do their own completion or return the addresses in the
//! sh: 12  # reply array in the form 'alias:address' and return 300. The -c option is
//! sh: 13  # passed on to plugins (and -n could be if needed ever). New plugins will be
//! sh: 14  # picked up and run automatically.
//! sh: 15
//! sh: 16  # plugins
//! sh: 17  (( $+functions[_email-mail] )) ||
//! sh: 18  _email-mail() {
//! sh: 19    local rc rcfiles i
//! sh: 20
//! sh: 21    rcfiles=( $files[$plugin] )
//! sh: 22    for ((i=1;i<=$#rcfiles;i++)); do
//! sh: 23      rcfiles+=( ${~${(M)${(f)"$(<$rcfiles[i])"}:#source*}##source[[:blank:]]##}(N) )
//! sh: 24    done
//! sh: 25    reply=()
//! sh: 26    for rc in $rcfiles; do
//! sh: 27      reply+=( ${${${(M)${(f)"$(<$rc)"}:#alias*}##alias[[:blank:]]##}/[[:blank:]]##/:} )
//! sh: 28    done
//! sh: 29    return 300
//! sh: 30  }
//! sh: 31  (( $+functions[_email-mutt] )) || _email-mutt() { _email-mail }
//! sh: 32  (( $+functions[_email-mush] )) || _email-mush() { _email-mail }
//! sh: 33
//! sh: 34  (( $+functions[_email-MH] )) ||
//! sh: 35  _email-MH() {
//! sh: 36    reply=( ${${(f)"$(_call_program aliases ali 2>/dev/null)"}/: /:} )
//! sh: 37    return 300
//! sh: 38  }
//! sh: 39
//! sh: 40  (( $+functions[_email-pine] )) ||
//! sh: 41  _email-pine() {
//! sh: 42    reply=( ${${${${${(f)"$(<~/.addressbook)"}:#*DELETED*}:#\ *}/	[^	]#	/:}%%	*} )
//! sh: 43    return 300
//! sh: 44  }
//! sh: 45
//! sh: 46  (( $+functions[_email-ldap] )) ||
//! sh: 47  _email-ldap() {
//! sh: 48    local -a expl ali res filter
//! sh: 49    local -A opts
//! sh: 50    local dn cn mail
//! sh: 51
//! sh: 52    zparseopts -D -E -A opts c
//! sh: 53
//! sh: 54    zstyle -a ":completion:${curcontext}:$curtag" filter filter
//! sh: 55    (( $#filter )) || return
//! sh: 56
//! sh: 57    filter=( "("${filter}"=${PREFIX}*${SUFFIX})" )
//! sh: 58    (( $#filter > 1 )) && filter="(|"${(j..)filter}")"
//! sh: 59    res=( ${(f)"$(_call_program $curtag ldapsearch -LLL \$filter cn mail 2>/dev/null)"} )
//! sh: 60    (( $#res > 1 )) || return
//! sh: 61
//! sh: 62    for dn cn mail in "${res[@]}"; do
//! sh: 63      if (( $+opts[-c] )); then
//! sh: 64        ali+=( "${mail#*: }" )
//! sh: 65      else
//! sh: 66        cn="${cn#*: }"
//! sh: 67        [[ $cn = *$~__specials* ]] && cn="\"$cn\""
//! sh: 68        ali+=( "$cn <${mail#*: }>" )
//! sh: 69      fi
//! sh: 70    done
//! sh: 71    compstate[insert]=menu
//! sh: 72    _wanted email-ldap expl 'matching name' \
//! sh: 73        compadd -U -i "$IPREFIX" -I "$ISUFFIX" "$@" -a - ali
//! sh: 74  }
//! sh: 75
//! sh: 76  (( $+functions[_email-local] )) ||
//! sh: 77  _email-local() {
//! sh: 78    local suf opts
//! sh: 79    zparseopts -D -E -A opts c S:=suf
//! sh: 80
//! sh: 81    if compset -P '*@'; then
//! sh: 82      _hosts "$@" "$suf[@]"
//! sh: 83    else
//! sh: 84      suf=()
//! sh: 85      compset -S '@*' || suf=( -qS @ )
//! sh: 86      _users "$suf[@]" "$@"
//! sh: 87    fi
//! sh: 88  }
//! sh: 89
//! sh: 90  _email_addresses() {
//! sh: 91    local -a plugins reply list args
//! sh: 92    local -A opts files
//! sh: 93    local plugin rcfile muttrc expl sep ret fret
//! sh: 94
//! sh: 95    local __specialx='][()<>@,;:\\".'
//! sh: 96    local __spacex=" 	"				# Space, tab
//! sh: 97    local __specials="[$__specialx]"
//! sh: 98    local __atom="[^$__specialx$__spacex]##"
//! sh: 99    local __space="[$__spacex]#"				# Really, space or comment
//! sh:100    local __qtext='[^"\\]'
//! sh:101    local __qpair='\\?'
//! sh:102    local __beginq='"'
//! sh:103    local __endq='(|[^\\])"'
//! sh:104    local __dot="$__space.$__space"
//! sh:105
//! sh:106    local __domainref="$__atom"
//! sh:107    local __domainlit='\[([^]]|'"$__qpair"')#(|[^\\])\]'
//! sh:108    local __quotedstring="$__beginq($__qtext|$__qpair)#$__endq"
//! sh:109    local __word="($__atom|$__quotedstring)"
//! sh:110    local __phrase="($__space$__word$__space)#"		# Strictly, should use `##'
//! sh:111    local __localpart="$__word($__dot$__word)#"
//! sh:112
//! sh:113    local __subdomain="($__domainref|$__domainlit)"
//! sh:114    local __domain="$__subdomain($__dot$__subdomain)#"
//! sh:115    local __addrspec="$__localpart$__space@$__space$__domain"
//! sh:116
//! sh:117    local __addresses="($__qtext|$__quotedstring)##"
//! sh:118
//! sh:119    zparseopts -D -E -A opts n: s: c
//! sh:120    set -- "$@" -M 'r:|[.@]=* r:|=* m:{a-zA-Z}={A-Za-z}'
//! sh:121
//! sh:122    if [[ -n $opts[-s] ]]; then
//! sh:123      # remove up to the last unquoted separator
//! sh:124      if [[ ${(Q)PREFIX} = (#b)($~__addresses$opts[-s])* ]]; then
//! sh:125        IFS="$opts[-s]" eval 'compset -P $(( ${#${=${:-x${match[1]}x}}} - 1 )) "*${opts[-s]}"'
//! sh:126      fi
//! sh:127
//! sh:128      # for the suffix, I'm too lazy to work out how to preserve quoted separators
//! sh:129      compset -S "$opts[-s]*" || set -- -q -S "$opts[-s]" "$@"
//! sh:130    fi
//! sh:131
//! sh:132    # get list of all plugins except any with missing config files
//! sh:133    if ! zstyle -s ":completion:${curcontext}:email-addresses" muttrc muttrc; then
//! sh:134      [[ -e ~/mutt/muttrc ]] && muttrc="~/mutt/muttrc" || muttrc="~/.muttrc"
//! sh:135    fi
//! sh:136    files=( MH ${MH:-~/.mh_profile} mutt $~muttrc mush ~/.mushrc mail ${MAILRC:-~/.mailrc} pine ~/.addressbook )
//! sh:137    plugins=(
//! sh:138      ${${(k)functions[(I)_email-*]#*-}:#(${(kj.|.)~files})}
//! sh:139      $files(Ne:'REPLY=( ${(k)files[(r)$REPLY]} ):')
//! sh:140    )
//! sh:141
//! sh:142    ret=1
//! sh:143    _tags email-$plugins
//! sh:144    while _tags; do
//! sh:145      for plugin in $plugins; do
//! sh:146        if _requested email-$plugin; then
//! sh:147  	while _next_label email-$plugin expl 'email address'; do
//! sh:148
//! sh:149            args=()
//! sh:150  	  if (( $+opts[-c] )) || zstyle -t \
//! sh:151  	      ":completion:${curcontext}:$curtag" strip-comments
//! sh:152  	  then
//! sh:153  	    args=( '-c' )
//! sh:154  	  fi
//! sh:155
//! sh:156  	  if ! _call_function fret _email-$plugin "$@" $args; then
//! sh:157  	    _message "$plugin: plugin not found"
//! sh:158  	    continue
//! sh:159  	  fi
//! sh:160  	  ret=$(( ret && fret ))
//! sh:161
//! sh:162  	  if (( fret == 300 )); then
//! sh:163  	    if (( ! $+opts[-c] )) && [[ $opts[-n] = $plugin ]]; then
//! sh:164  	      zstyle -s ":completion:${curcontext}:$curtag" list-separator sep || sep=--
//! sh:165  	      zformat -a list " $sep " "${reply[@]}"
//! sh:166  	      _wanted mail-aliases expl 'alias' compadd "$@" \
//! sh:167  		  -d list - ${reply%%:*} && ret=0
//! sh:168  	    else
//! sh:169  	      if (( $#args )); then
//! sh:170  		reply=( ${(SM)${reply#*:}##$~__addrspec} )
//! sh:171  	      else
//! sh:172  		# remove lines not containing `@' as they probably aren't addresses
//! sh:173  		reply=( "${(@)${(M@)reply:#*@*}#*:}" )
//! sh:174  	      fi
//! sh:175  	      compadd -a "$@" "$expl[@]" reply && ret=0
//! sh:176  	    fi
//! sh:177  	  fi
//! sh:178  	done
//! sh:179        fi
//! sh:180      done
//! sh:181      (( ret )) || return 0
//! sh:182    done
//! sh:183
//! sh:184    return 1
//! sh:185  }
//! sh:186
//! sh:187  _email_addresses "$@"
//! ```



use std::path::{Path, PathBuf};

use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;

pub struct EmailAddressesOpts<'a> {
    /// `-n plugin` — restrict to entries from the named plugin.
    pub only_plugin: Option<&'a str>,
    /// `-s sep` — chew `*sep` from the front of PREFIX so user can
    /// complete the Nth entry in a `addr1, addr2, addr3` list.
    pub separator: Option<&'a str>,
    /// `-c` — only emit RFC822 `user@host` form, drop nickname/
    /// realname annotations. Strip-comments style override.
    pub bare_addresses: bool,
    /// Override the home dir used to locate `.mailrc` / `.muttrc` /
    /// `.addressbook` / `.mh_profile`. Defaults to `$HOME` when None.
    /// Test-friendly: pass a tmpdir without mutating process env.
    pub home_dir: Option<&'a Path>,
    /// LDAP `filter` zstyle from shell:51-58. Each entry is an LDAP
    /// attribute name (e.g. "cn", "uid", "mail"). When non-empty,
    /// triggers an `ldapsearch` invocation building a filter like
    /// `(|(cn=PREFIX*)(uid=PREFIX*))`. Empty disables LDAP entirely.
    pub ldap_filter: Vec<String>,
    /// `local` plugin user list — names that complete BEFORE the
    /// `@` in `user@host`. Caller pulls from /etc/passwd or NSS.
    pub users: Vec<String>,
    /// `local` plugin host list — names that complete AFTER the
    /// `@`. Caller pulls from /etc/hosts, ~/.ssh/known_hosts, etc.
    pub hosts: Vec<String>,
    /// Optional `-S` suffix value passed to LDAP filter building.
    pub suffix: Option<&'a str>,
}

impl<'a> Default for EmailAddressesOpts<'a> {
    fn default() -> Self {
        Self {
            only_plugin: None,
            separator: None,
            bare_addresses: false,
            home_dir: None,
            ldap_filter: Vec::new(),
            users: Vec::new(),
            hosts: Vec::new(),
            suffix: None,
        }
    }
}

pub fn _email_addresses(state: &mut CompletionState, opts: &EmailAddressesOpts<'_>) -> bool {
    // shell:121-128 `-s sep` PREFIX chewing. Also trim leading
    // whitespace from the remainder since users typically type
    // `addr1, addr2, addr3` with spaces after each separator.
    if let Some(sep) = opts.separator {
        if let Some(idx) = state.params.prefix.rfind(sep) {
            let chewed_end = idx + sep.len();
            let chewed = state.params.prefix[..chewed_end].to_string();
            state.params.iprefix.push_str(&chewed);
            let rest = state.params.prefix[chewed_end..].to_string();
            let trimmed = rest.trim_start();
            let leading_ws = &rest[..rest.len() - trimmed.len()];
            if !leading_ws.is_empty() {
                state.params.iprefix.push_str(leading_ws);
            }
            state.params.prefix = trimmed.to_string();
        }
    }

    let home = match opts.home_dir {
        Some(p) => p.to_path_buf(),
        None => match std::env::var("HOME") {
            Ok(h) => PathBuf::from(h),
            Err(_) => return false,
        },
    };

    let mut entries: Vec<(String, String)> = Vec::new(); // (plugin, address)

    let want = |name: &str| -> bool {
        opts.only_plugin.map(|p| p == name).unwrap_or(true)
    };

    // ── mail / mutt / mush plugin: `.mailrc`-style files ──────────────
    if want("mail") || want("mutt") || want("mush") {
        let mut files: Vec<PathBuf> = Vec::new();
        if want("mail") {
            let mailrc = std::env::var("MAILRC")
                .map(PathBuf::from)
                .unwrap_or_else(|_| home.join(".mailrc"));
            files.push(mailrc);
        }
        if want("mutt") {
            // shell:135-138: zstyle override OR ~/mutt/muttrc OR ~/.muttrc.
            let muttrc = home.join("mutt").join("muttrc");
            if muttrc.exists() {
                files.push(muttrc);
            } else {
                files.push(home.join(".muttrc"));
            }
        }
        if want("mush") {
            files.push(home.join(".mushrc"));
        }
        for f in &files {
            collect_alias_lines(f, &mut entries, "mail");
        }
    }

    // ── pine plugin: `.addressbook` ───────────────────────────────────
    if want("pine") {
        let pine = home.join(".addressbook");
        if let Ok(content) = std::fs::read_to_string(&pine) {
            for line in content.lines() {
                // shell:42: skip DELETED entries and leading-space cont lines.
                if line.contains("DELETED") || line.starts_with(' ') {
                    continue;
                }
                // Format: NICK\tNAME\tADDR\t…
                let cols: Vec<&str> = line.split('\t').collect();
                if cols.len() >= 3 {
                    entries.push(("pine".into(), cols[2].into()));
                }
            }
        }
    }

    // ── MH plugin: `ali` output (shell:35-37) ─────────────────────────
    if want("MH") {
        let mh_profile = std::env::var("MH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".mh_profile"));
        if mh_profile.exists() {
            if let Ok(out) = std::process::Command::new("ali").output() {
                if out.status.success() {
                    for line in String::from_utf8_lossy(&out.stdout).lines() {
                        if let Some(addr) = line.splitn(2, ": ").nth(1) {
                            entries.push(("MH".into(), addr.to_string()));
                        }
                    }
                }
            }
        }
    }

    // ── ldap plugin: shell:46-74 ──────────────────────────────────────
    // When `opts.ldap_filter` is non-empty, invoke `ldapsearch` and
    // parse `mail:` lines from the LDIF output. Skipped silently when
    // ldapsearch is not installed.
    if want("ldap") && !opts.ldap_filter.is_empty() {
        let prefix = state.params.prefix.clone();
        let suffix_part = opts.suffix.unwrap_or("");
        let filter_combined = if opts.ldap_filter.len() > 1 {
            let inner: String = opts
                .ldap_filter
                .iter()
                .map(|f| format!("({}={}{}*)", f, prefix, suffix_part))
                .collect();
            format!("(|{})", inner)
        } else {
            format!(
                "({}={}{}*)",
                opts.ldap_filter[0], prefix, suffix_part
            )
        };
        let mut cmd = std::process::Command::new("ldapsearch");
        cmd.args(["-LLL", &filter_combined, "cn", "mail"]);
        if let Ok(out) = cmd.output() {
            if out.status.success() {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    if let Some(mail) = line.strip_prefix("mail: ") {
                        entries.push(("ldap".into(), mail.to_string()));
                    }
                }
            }
        }
    }

    // ── local plugin: shell:76-88 ─────────────────────────────────────
    // Pre-`@` → user names; post-`@` → host names. Caller supplies
    // both lists.
    if want("local") {
        let prefix = state.params.prefix.clone();
        if let Some(at_idx) = prefix.find('@') {
            let host_prefix = &prefix[at_idx + 1..];
            for h in &opts.hosts {
                if h.starts_with(host_prefix) {
                    entries.push(("local".into(), format!("{}@{}", &prefix[..at_idx], h)));
                }
            }
        } else {
            for u in &opts.users {
                if u.starts_with(&prefix) {
                    entries.push(("local".into(), u.clone()));
                }
            }
        }
    }

    if entries.is_empty() {
        return false;
    }

    // Apply -c: keep only entries containing `@` and strip name/comment.
    let to_match: Vec<String> = entries
        .iter()
        .map(|(_, raw)| {
            if opts.bare_addresses {
                extract_bare_address(raw)
            } else {
                raw.clone()
            }
        })
        .filter(|a| !opts.bare_addresses || a.contains('@'))
        .collect();

    let prefix = state.params.prefix.clone();
    state.begin_group("email-addresses", true);
    let mut seen = std::collections::HashSet::new();
    for addr in &to_match {
        if !addr.starts_with(&prefix) {
            continue;
        }
        if seen.insert(addr.clone()) {
            state.add_match(Completion::new(addr.clone()), Some("email-addresses"));
        }
    }
    state.end_group();
    state.nmatches > 0
}

/// Parse mailrc / muttrc / mushrc-style `alias NAME ADDRESS` lines.
fn collect_alias_lines(path: &Path, out: &mut Vec<(String, String)>, plugin: &'static str) {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return,
    };
    for line in content.lines() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix("alias ") {
            // mailrc: `alias NAME ADDR1 ADDR2 ...`
            // mutt:   `alias NAME ADDR` or `alias NAME=ADDR`
            let rest = rest.trim_start();
            // First whitespace ends NAME.
            let after_name = match rest.find(char::is_whitespace) {
                Some(i) => &rest[i + 1..],
                None => continue,
            };
            // The rest is space-separated address list.
            for addr in after_name.split_whitespace() {
                out.push((plugin.into(), addr.to_string()));
            }
        }
    }
}

/// `Name <addr@host>` → `addr@host`. Standalone `addr@host` returns
/// unchanged. Bare names without `@` return as-is (caller filters).
fn extract_bare_address(raw: &str) -> String {
    if let Some(open) = raw.find('<') {
        if let Some(close) = raw[open..].find('>') {
            return raw[open + 1..open + close].to_string();
        }
    }
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn with_temp_home<R, F: FnOnce(&Path) -> R>(setup: F) -> R {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_email_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let result = setup(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);
        result
    }

    fn write_file(p: &Path, body: &str) {
        let mut f = std::fs::File::create(p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    #[test]
    fn mailrc_alias_lines_become_completions() {
        with_temp_home(|home| {
            write_file(
                &home.join(".mailrc"),
                "alias bob bob@example.com\n\
                 alias alice alice@example.com\n",
            );
            let mut state = CompletionState::new();
            state.params.prefix = "a".into();
            let ok = _email_addresses(
                &mut state,
                &EmailAddressesOpts {
                    only_plugin: Some("mail"),
                    home_dir: Some(home),
                    ..Default::default()
                },
            );
            assert!(ok);
            let names: Vec<&str> = state.groups[0]
                .matches
                .iter()
                .map(|c| c.str_.as_str())
                .collect();
            assert!(names.contains(&"alice@example.com"), "got {names:?}");
            assert!(!names.contains(&"bob@example.com"));
        });
    }

    #[test]
    fn separator_chews_prefix_to_last_separator() {
        with_temp_home(|home| {
            write_file(&home.join(".mailrc"), "alias x x@example.com\n");
            let mut state = CompletionState::new();
            state.params.prefix = "bob@a.com, alice@b.com, x".into();
            let ok = _email_addresses(
                &mut state,
                &EmailAddressesOpts {
                    only_plugin: Some("mail"),
                    separator: Some(","),
                    home_dir: Some(home),
                    ..Default::default()
                },
            );
            assert!(ok);
            // After chew: prefix = "x" (leading space trimmed), iprefix
            // = "bob@a.com, alice@b.com, ".
            assert!(state.params.iprefix.contains("bob@a.com"));
            assert!(state.params.iprefix.ends_with(' '),
                    "leading space after the last `,` should land in iprefix, not prefix");
            assert_eq!(state.params.prefix, "x");
        });
    }

    #[test]
    fn c_flag_strips_name_and_drops_non_at_entries() {
        with_temp_home(|home| {
            write_file(
                &home.join(".mailrc"),
                "alias bob bob@example.com\n\
                 alias group alice@x.com carol@y.com\n",
            );
            let mut state = CompletionState::new();
            let ok = _email_addresses(
                &mut state,
                &EmailAddressesOpts {
                    only_plugin: Some("mail"),
                    bare_addresses: true,
                    home_dir: Some(home),
                    ..Default::default()
                },
            );
            assert!(ok);
            let names: Vec<&str> = state.groups[0]
                .matches
                .iter()
                .map(|c| c.str_.as_str())
                .collect();
            assert!(names.contains(&"bob@example.com"));
            assert!(names.contains(&"alice@x.com"));
            assert!(names.contains(&"carol@y.com"));
        });
    }

    #[test]
    fn no_sources_returns_false() {
        with_temp_home(|home| {
            let mut state = CompletionState::new();
            let ok = _email_addresses(
                &mut state,
                &EmailAddressesOpts {
                    only_plugin: Some("mail"),
                    home_dir: Some(home),
                    ..Default::default()
                },
            );
            assert!(!ok);
        });
    }

    #[test]
    fn local_plugin_before_at_emits_user_candidates() {
        let mut state = CompletionState::new();
        state.params.prefix = "al".into();
        let ok = _email_addresses(
            &mut state,
            &EmailAddressesOpts {
                only_plugin: Some("local"),
                home_dir: Some(std::path::Path::new("/tmp")),
                users: vec!["alice".into(), "bob".into(), "alex".into()],
                ..Default::default()
            },
        );
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"alice"));
        assert!(names.contains(&"alex"));
        assert!(!names.contains(&"bob"));
    }

    #[test]
    fn local_plugin_after_at_emits_host_candidates() {
        let mut state = CompletionState::new();
        state.params.prefix = "alice@ex".into();
        let ok = _email_addresses(
            &mut state,
            &EmailAddressesOpts {
                only_plugin: Some("local"),
                home_dir: Some(std::path::Path::new("/tmp")),
                hosts: vec!["example.com".into(), "other.org".into()],
                ..Default::default()
            },
        );
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(
            names.contains(&"alice@example.com"),
            "host candidates after `@` should yield user@host — got {names:?}"
        );
        assert!(!names.iter().any(|n| n.contains("other.org")));
    }

    #[test]
    fn ldap_skipped_when_filter_empty() {
        let mut state = CompletionState::new();
        let ok = _email_addresses(
            &mut state,
            &EmailAddressesOpts {
                only_plugin: Some("ldap"),
                home_dir: Some(std::path::Path::new("/tmp")),
                ldap_filter: vec![], // empty → skip LDAP entirely
                ..Default::default()
            },
        );
        assert!(!ok, "empty ldap_filter must skip ldap and return false");
    }
}
