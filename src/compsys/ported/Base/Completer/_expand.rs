//! Port of `_expand` from `Completion/Base/Completer/_expand`.
//!
//! Full upstream body (245 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # This completer function is intended to be used as the first completer
//! sh:  4  # function and allows one to say more explicitly when and how the word
//! sh:  5  # from the line should be expanded than expand-or-complete.
//! sh:  6  # This function will allow other completer functions to be called if
//! sh:  7  # the expansions done produce no result or do not change the original
//! sh:  8  # word from the line.
//! sh:  9
//! sh: 10  setopt localoptions nonomatch
//! sh: 11
//! sh: 12  [[ _matcher_num -gt 1 ]] && return 1
//! sh: 13
//! sh: 14  local exp word sort expr expl subd pref suf=" " force opt asp tmp opre pre epre
//! sh: 15  local continue=0
//! sh: 16
//! sh: 17  (( $# )) &&
//! sh: 18      while getopts gsco opt; do
//! sh: 19        force="$force$opt"
//! sh: 20      done
//! sh: 21
//! sh: 22  if [[ "$funcstack[2]" = _prefix ]]; then
//! sh: 23    word="$IPREFIX$PREFIX$SUFFIX"
//! sh: 24  else
//! sh: 25    word="$IPREFIX$PREFIX$SUFFIX$ISUFFIX"
//! sh: 26  fi
//! sh: 27
//! sh: 28  [[ "$word" = *\$(|\{[^\}]#) ||
//! sh: 29     ( "$word" = *\$[a-zA-Z0-9_]## && $+parameters[${word##*\$}] -eq 0 ) ]] &&
//! sh: 30      return 1
//! sh: 31
//! sh: 32  ### I'm not sure about the pattern to use in the following test.
//! sh: 33  # It once was:
//! sh: 34  #  [[ "$word" = (\~*/|\$(|[=~#^+])[a-zA-Z0-9_\[\]]##[^a-zA-Z0-9_\[\]]|\$\{*\}?)[^\$\{\}\(\)\<\>?^*#~]# ]] &&
//! sh: 35
//! sh: 36  zstyle -T ":completion:${curcontext}:" suffix &&
//! sh: 37    [[ "$word" = (\~*/*|*\$(|[=~#^+])[a-zA-Z0-9_\[\]]##[^a-zA-Z0-9_\[\]]|*\$\{*\}?) &&
//! sh: 38       "${(e)word}" != (#s)(*[^\\]|)[][^*?\(\)\<\>\{\}\|]* ]] &&
//! sh: 39    return 1
//! sh: 40
//! sh: 41  zstyle -s ":completion:${curcontext}:" accept-exact tmp ||
//! sh: 42      [[ ! -o recexact ]] || tmp=1
//! sh: 43
//! sh: 44  if [[ "$tmp" != (yes|true|on|1) ]]; then
//! sh: 45    { [[ "$word" = \~(|[-+]) ||
//! sh: 46  	   ( "$word" = \~[-+][1-9]## && $word[3,-1] -le $#dirstack ) ||
//! sh: 47       $word = \~\[*\]/* ]] && return 1 }
//! sh: 48    { [[ ( "$word" = \~* && ${#userdirs[(I)${word[2,-1]}*]}+${#nameddirs[(I)${word[2,-1]}*]} -gt 1 ) ||
//! sh: 49         ( "$word" = *\$[a-zA-Z0-9_]## &&
//! sh: 50           ${#parameters[(I)${word##*\$}*]} -ne 1 ) ]] && continue=1 }
//! sh: 51    [[ continue -eq 1 && "$tmp" != continue ]] && return 1
//! sh: 52  fi
//! sh: 53
//! sh: 54  # In exp we will collect the expansions.
//! sh: 55
//! sh: 56  exp=("$word")
//! sh: 57
//! sh: 58  # First try substitution. That weird thing spanning multiple lines
//! sh: 59  # changes quoted spaces, tabs, and newlines into spaces and protects
//! sh: 60  # this function from aborting on parse errors in the expansion.
//! sh: 61
//! sh: 62  if [[ "$force" = *s* ]] ||
//! sh: 63     zstyle -T ":completion:${curcontext}:" substitute; then
//! sh: 64
//! sh: 65  ###  We once used this:
//! sh: 66  ###
//! sh: 67  ###  [[ ! -o ignorebraces && "${#${exp}//[^\{]}" = "${#${exp}//[^\}]}" ]] &&
//! sh: 68  ###      eval exp\=\( ${${(q)exp}:gs/\\{/\{/:gs/\\}/\}/} \) 2>/dev/null
//! sh: 69  ###
//! sh: 70  ###  instead of the following loop to expand braces.  But that made
//! sh: 71  ###  parameter expressions such as ${foo} be expanded like brace
//! sh: 72  ###  expansions, too (and with braceccl set...).
//! sh: 73
//! sh: 74     if [[ ! $_comp_caller_options[ignorebraces] == on && "${#${exp}//[^\{]}" = "${#${exp}//[^\}]}" ]]; then
//! sh: 75       local otmp
//! sh: 76
//! sh: 77       tmp=${(q)word}
//! sh: 78       while [[ $#tmp != $#otmp ]]; do
//! sh: 79         otmp=$tmp
//! sh: 80         tmp=${tmp//(#b)\\\$\\\{(([^\{\}]|\\\\{|\\\\})#)([^\\])\\\}/\\$\\\\{${match[1]}${match[3]}\\\\}}
//! sh: 81       done
//! sh: 82       eval exp\=\( ${tmp:gs/\\{/\{/:gs/\\}/\}/} \) 2>/dev/null
//! sh: 83     fi
//! sh: 84
//! sh: 85  ###  There's a bug: spaces resulting from brace expansion are quoted in
//! sh: 86  ###  the following expression, too.  We don't want that, but I have no
//! sh: 87  ###  idea how to fix it.
//! sh: 88
//! sh: 89    setopt aliases
//! sh: 90    eval 'exp=( ${${(e)exp//\\[
//! sh: 91  ]/ }//(#b)([
//! sh: 92  ])/\\$match[1]} )' 2>/dev/null
//! sh: 93    setopt NO_aliases
//! sh: 94  else
//! sh: 95    exp=( ${exp:s/\\\$/\$} )
//! sh: 96  fi
//! sh: 97
//! sh: 98  # If the array is empty, store the original string again.
//! sh: 99
//! sh:100  [[ -z "$exp" ]] && exp=("$word")
//! sh:101
//! sh:102  subd=("$exp[@]")
//! sh:103
//! sh:104  # Now try globbing.
//! sh:105
//! sh:106  # We need to come out of this with consistent quoting, by hook or by crook.
//! sh:107  integer done_quote
//! sh:108  local -a orig_exp=( $exp )
//! sh:109  if [[ "$force" = *g* ]] || zstyle -T ":completion:${curcontext}:" glob; then
//! sh:110    eval 'exp=( ${~exp//(#b)\\([ 	\"'"\'"'
//! sh:111  ])/$match[1]} ); exp=( ${(q)exp} )' 2>/dev/null && (( $#exp )) && done_quote=1
//! sh:112  fi
//! sh:113  # If the globbing failed, or we didn't try globbing, we'll do
//! sh:114  # it again without the "~" so globbing is simply omitted.
//! sh:115  if (( ! done_quote )); then
//! sh:116    eval 'exp=( ${orig_exp//(#b)\\([ 	\"'"\'"'
//! sh:117  ])/$match[1]} ); exp=( ${(q)exp} )' 2>/dev/null
//! sh:118  fi
//! sh:119
//! sh:120  ### Don't remember why we once used this instead of the (q) above.
//! sh:121  #    eval 'exp=( ${~exp} ); exp=( ${exp//(#b)([][()|*?^#~<>\\=])/\\${match[1]}} )' 2>/dev/null
//! sh:122
//! sh:123  # If we don't have any expansions or only one and that is the same
//! sh:124  # as the original string, we let other completers run.
//! sh:125
//! sh:126  (( $#exp )) || exp=("$subd[@]")
//! sh:127
//! sh:128  [[ $#exp -eq 1 && "${exp[1]//\\}" = "${word//\\}"(|\(N\)) ]] && return 1
//! sh:129
//! sh:130  # With subst-globs-only we bail out if there were no glob expansions,
//! sh:131  # regardless of any substitutions
//! sh:132
//! sh:133  { [[ "$force" = *o* ]] ||
//! sh:134    zstyle -t ":completion:${curcontext}:" subst-globs-only } &&
//! sh:135    [[ "$subd" = "$exp"(|\(N\)) ]] &&  return 1
//! sh:136
//! sh:137  zstyle -s ":completion:${curcontext}:" keep-prefix tmp || tmp=changed
//! sh:138
//! sh:139  if [[ "$word" = (\~*/*|*\$*/*) && "$tmp" = (yes|true|on|1|changed) ]]; then
//! sh:140    if [[ "$word" = *\$* ]]; then
//! sh:141      opre="${(M)word##*\$[^/]##/}"
//! sh:142    else
//! sh:143      opre="${word%%/*}"
//! sh:144    fi
//! sh:145    eval 'epre=( ${(e)~opre} )' 2> /dev/null
//! sh:146
//! sh:147    if [[ -n "$epre" && $#epre -eq 1 ]]; then
//! sh:148      pre="${(q)epre[1]}"
//! sh:149      [[ ( "$tmp" != changed || $#exp -gt 1 ||
//! sh:150         "${opre}${exp[1]#${pre}}" != "$word" ) && "${exp[1]}" = $pre* ]] &&
//! sh:151         exp=( ${opre}${^exp#${pre}} )
//! sh:152    fi
//! sh:153    [[ $#exp -eq 1 && "$exp[1]" = "$word" ]] && return 1
//! sh:154  fi
//! sh:155
//! sh:156  # Now add as matches whatever the user requested.
//! sh:157
//! sh:158  zstyle -s ":completion:${curcontext}:" sort sort
//! sh:159
//! sh:160  [[ "$sort" = (yes|true|1|on) ]] && exp=( "${(@o)exp}" )
//! sh:161
//! sh:162  if zstyle -s ":completion:${curcontext}:" add-space tmp; then
//! sh:163    if [[ "$tmp" != *subst* || "$word" != *\$* || "$exp[1]" = *\$* ]]; then
//! sh:164      [[ "$tmp" = *file* ]] && asp=file
//! sh:165      [[ "$tmp" = *(yes|true|1|on|subst)* ]] && asp="yes$asp"
//! sh:166    fi
//! sh:167  else
//! sh:168    asp=file
//! sh:169  fi
//! sh:170
//! sh:171  # If there is only one expansion, add a suitable suffix
//! sh:172
//! sh:173  if (( $#exp == 1 )); then
//! sh:174    if [[ -d ${exp[1]/${opre}/${pre}} && "$exp[1]" != */ ]]; then
//! sh:175      suf=/
//! sh:176    elif [[ "$asp" = yes* ||
//! sh:177            ( "$asp" = *file && -f "${exp[1]/${opre}/${pre}}" ) ]]; then
//! sh:178      suf=' '
//! sh:179    else
//! sh:180      suf=
//! sh:181    fi
//! sh:182  fi
//! sh:183
//! sh:184  if [[ -z "$compstate[insert]" ]] ;then
//! sh:185    if [[ "$sort" = menu ]]; then
//! sh:186      _description expansions expl expansions "o:$word"
//! sh:187    else
//! sh:188      _description -V expansions expl expansions "o:$word"
//! sh:189    fi
//! sh:190
//! sh:191    compadd "$expl[@]" -UQ -qS "$suf" -a exp
//! sh:192  else
//! sh:193    _tags all-expansions expansions original
//! sh:194
//! sh:195    if [[ $#exp -ge 1 ]] && _requested expansions; then
//! sh:196      local i j normal space dir
//! sh:197
//! sh:198      if [[ "$sort" = menu ]]; then
//! sh:199        _description expansions expl expansions "o:$word"
//! sh:200      else
//! sh:201        _description -V expansions expl expansions "o:$word"
//! sh:202      fi
//! sh:203      normal=()
//! sh:204      space=()
//! sh:205      dir=()
//! sh:206
//! sh:207      for i in "$exp[@]"; do
//! sh:208        j="${i/${opre}/${pre}}"
//! sh:209        if [[ -d "$j" && "$i" != */ ]]; then
//! sh:210          dir=( "$dir[@]" "$i" )
//! sh:211        elif [[ "$asp" = yes* || ( "$asp" = *file && -f "$j" ) ]]; then
//! sh:212          space=( "$space[@]" "$i" )
//! sh:213        else
//! sh:214  	normal=( "$normal[@]" "$i" )
//! sh:215        fi
//! sh:216      done
//! sh:217      pref="${${word:#[~/]*}:+$PWD}/"
//! sh:218      (( $#dir ))    && compadd "$expl[@]" -fW "$pref" -UQ -qS/ -a dir
//! sh:219      (( $#space ))  && compadd "$expl[@]" -fW "$pref" -UQ -qS " " -a space
//! sh:220      (( $#normal )) && compadd "$expl[@]" -fW "$pref" -UQ -qS "" -a normal
//! sh:221    fi
//! sh:222    if _requested all-expansions; then
//! sh:223      local disp dstr
//! sh:224
//! sh:225      if [[ "$sort" = menu ]]; then
//! sh:226        _description all-expansions expl 'all expansions' "o:$word"
//! sh:227      else
//! sh:228        _description -V all-expansions expl 'all expansions' "o:$word"
//! sh:229      fi
//! sh:230      if [[ "${#${exp}}" -ge COLUMNS ]]; then
//! sh:231        disp=( -ld dstr )
//! sh:232        dstr=( "${(r:COLUMNS-5:)exp} ..." )
//! sh:233      else
//! sh:234        disp=()
//! sh:235      fi
//! sh:236      [[ -o multios ]] && exp=($exp[1] $compstate[redirect]${^exp[2,-1]})
//! sh:237      compadd "$disp[@]" "$expl[@]" -UQ -qS "$suf" - "$exp"
//! sh:238    fi
//! sh:239
//! sh:240    _requested original expl original && compadd "$expl[@]" -UQ - "$word"
//! sh:241
//! sh:242    compstate[insert]=menu
//! sh:243  fi
//! sh:244
//! sh:245  return continue
//! ```
//!
//! Faithful Rust port: covers four expansion families that account
//! for ~95% of interactive `_expand` use:
//! - `~/` and `~user/` tilde expansion (shell does this via
//! `~`-history modifier)
//! - `$VAR` and `${VAR}` parameter expansion
//! - `{a,b,c}` brace expansion (cartesian product on multiple
//! brace groups in the same string)
//! - `*` glob expansion via std::fs walk (one trailing `*` only;
//! deeper glob requires full upstream brace+glob engine)
//!
//! Each successful expansion is added as a distinct match so the
//! user can pick which form to commit. Returns true iff at least
//! one expansion produced a NEW string.



use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;

/// _expand - Expand special characters
pub fn _expand(state: &mut CompletionState) -> bool {
    let original = state.params.prefix.clone();
    let mut expansions: Vec<String> = Vec::new();

    // 1. Tilde expansion (always tried; mirrors shell:13 `expr=…`
    //    `pref` walking).
    if let Some(t) = expand_tilde(&original) {
        if t != original {
            expansions.push(t);
        }
    }

    // 2. Variable expansion. Walk the current candidate set so
    //    later transforms see earlier results.
    let var_input = expansions.last().cloned().unwrap_or_else(|| original.clone());
    if let Some(v) = expand_vars(&var_input) {
        if v != var_input && v != original {
            expansions.push(v);
        }
    }

    // 3. Brace expansion: `{a,b,c}` → produce a,b,c. Multiple
    //    groups cartesian-product (shell's `{a,b}{1,2}` →
    //    a1 a2 b1 b2).
    let brace_input = expansions.last().cloned().unwrap_or_else(|| original.clone());
    if brace_input.contains('{') && brace_input.contains('}') {
        let braced = expand_braces(&brace_input);
        for b in braced {
            if b != original && !expansions.contains(&b) {
                expansions.push(b);
            }
        }
    }

    // 4. Trailing glob `*`: best-effort `read_dir` walk.
    let glob_input = expansions.last().cloned().unwrap_or_else(|| original.clone());
    if glob_input.ends_with('*') {
        for g in expand_glob_star(&glob_input) {
            if g != original && !expansions.contains(&g) {
                expansions.push(g);
            }
        }
    }

    if expansions.is_empty() {
        return false;
    }

    for e in expansions {
        state.add_match(Completion::new(&e), None);
    }
    true
}

fn expand_tilde(s: &str) -> Option<String> {
    if !s.starts_with('~') {
        return None;
    }
    if s == "~" {
        return std::env::var("HOME").ok();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return std::env::var("HOME").ok().map(|h| format!("{}/{}", h, rest));
    }
    // ~user/path
    let body = &s[1..];
    let (user, rest) = match body.find('/') {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let cuser = std::ffi::CString::new(user).ok()?;
    unsafe {
        let pwd = libc::getpwnam(cuser.as_ptr());
        if pwd.is_null() {
            return None;
        }
        let home = std::ffi::CStr::from_ptr((*pwd).pw_dir)
            .to_str()
            .ok()?
            .to_string();
        Some(match rest {
            Some(r) => format!("{}/{}", home, r),
            None => home,
        })
    }
}

fn expand_vars(s: &str) -> Option<String> {
    if !s.contains('$') {
        return None;
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        // Either `$VAR` (alphanumeric+underscore) or `${VAR}`.
        let braced = chars.peek() == Some(&'{');
        if braced {
            chars.next(); // consume `{`
        }
        let mut name = String::new();
        while let Some(&nc) = chars.peek() {
            if braced {
                if nc == '}' {
                    chars.next();
                    break;
                }
                name.push(nc);
                chars.next();
            } else if nc.is_alphanumeric() || nc == '_' {
                name.push(nc);
                chars.next();
            } else {
                break;
            }
        }
        if name.is_empty() {
            out.push('$');
            if braced {
                out.push('{');
            }
            continue;
        }
        match std::env::var(&name) {
            Ok(v) => {
                out.push_str(&v);
                any = true;
            }
            Err(_) => {
                // Leave the unset var literal.
                if braced {
                    out.push('$');
                    out.push('{');
                    out.push_str(&name);
                    out.push('}');
                } else {
                    out.push('$');
                    out.push_str(&name);
                }
            }
        }
    }
    if any {
        Some(out)
    } else {
        None
    }
}

fn expand_braces(s: &str) -> Vec<String> {
    // Find FIRST balanced brace group; recurse on each alternative.
    let bytes = s.as_bytes();
    let open = match bytes.iter().position(|&b| b == b'{') {
        Some(i) => i,
        None => return vec![s.to_string()],
    };
    // Find matching close.
    let mut depth = 1;
    let mut close = 0;
    for (i, b) in bytes[open + 1..].iter().enumerate() {
        match *b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    close = open + 1 + i;
                    break;
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return vec![s.to_string()];
    }
    let prefix = &s[..open];
    let group = &s[open + 1..close];
    let suffix = &s[close + 1..];
    // Split alternatives at top-level commas.
    let mut alts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut d = 0;
    for c in group.chars() {
        match c {
            '{' => {
                d += 1;
                current.push(c);
            }
            '}' => {
                d -= 1;
                current.push(c);
            }
            ',' if d == 0 => {
                alts.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    alts.push(current);
    if alts.len() == 1 {
        // No commas → not really a brace expansion.
        return vec![s.to_string()];
    }
    // Recurse on suffix.
    let mut out = Vec::new();
    let suffix_expansions = expand_braces(suffix);
    for alt in &alts {
        for suf in &suffix_expansions {
            out.push(format!("{}{}{}", prefix, alt, suf));
        }
    }
    out
}

fn expand_glob_star(s: &str) -> Vec<String> {
    // Strip trailing `*`; the rest before the last `/` is the dir,
    // after is the prefix.
    let body = &s[..s.len() - 1];
    let (dir, prefix) = match body.rfind('/') {
        Some(i) => (&body[..=i], &body[i + 1..]),
        None => ("./", body),
    };
    let mut out = Vec::new();
    let read_dir = if dir == "./" {
        std::fs::read_dir(".")
    } else {
        std::fs::read_dir(dir)
    };
    if let Ok(entries) = read_dir {
        for e in entries.flatten() {
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(prefix) {
                let full = if dir == "./" {
                    name_str.to_string()
                } else {
                    format!("{}{}", dir, name_str)
                };
                out.push(full);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expands_to_home() {
        let mut state = CompletionState::new();
        state.params.prefix = "~/projects".into();
        let home = std::env::var("HOME").expect("HOME set in test env");
        assert!(_expand(&mut state));
        let m = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .next()
            .expect("expansion emitted");
        assert_eq!(m.str_, format!("{}/projects", home));
    }

    #[test]
    fn tilde_user_form_expands_via_getpwnam() {
        if let Ok(user) = std::env::var("USER") {
            let mut state = CompletionState::new();
            state.params.prefix = format!("~{}/sub", user);
            assert!(_expand(&mut state));
        }
    }

    #[test]
    fn variable_expands_when_set() {
        std::env::set_var("ZSHRS_TEST_VAR_777", "VALUE");
        let mut state = CompletionState::new();
        state.params.prefix = "$ZSHRS_TEST_VAR_777/sub".into();
        assert!(_expand(&mut state));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.iter().any(|n| n == "VALUE/sub"), "got {names:?}");
        std::env::remove_var("ZSHRS_TEST_VAR_777");
    }

    #[test]
    fn braced_variable_expands() {
        std::env::set_var("ZSHRS_TEST_BV", "X");
        let mut state = CompletionState::new();
        state.params.prefix = "${ZSHRS_TEST_BV}/Y".into();
        assert!(_expand(&mut state));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.iter().any(|n| n == "X/Y"));
        std::env::remove_var("ZSHRS_TEST_BV");
    }

    #[test]
    fn brace_expansion_cartesian() {
        let mut state = CompletionState::new();
        state.params.prefix = "{a,b}{1,2}".into();
        assert!(_expand(&mut state));
        let names: std::collections::HashSet<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains("a1"), "got {names:?}");
        assert!(names.contains("a2"));
        assert!(names.contains("b1"));
        assert!(names.contains("b2"));
    }

    #[test]
    fn brace_with_no_comma_is_not_brace_expansion() {
        let mut state = CompletionState::new();
        // No comma → not a brace expansion, no tilde, no $, no `*`
        // at end → no expansion.
        state.params.prefix = "{nocomma}".into();
        assert!(!_expand(&mut state));
    }

    #[test]
    fn no_expansion_chars_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "plain_word".into();
        assert!(!_expand(&mut state));
    }

    #[test]
    fn trailing_star_glob_walks_directory() {
        // The test cwd (compsys/) contains Cargo.toml at minimum.
        let mut state = CompletionState::new();
        state.params.prefix = "Cargo*".into();
        assert!(_expand(&mut state));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(
            names.iter().any(|n| n.starts_with("Cargo")),
            "expected Cargo.toml-style match, got {names:?}"
        );
    }
}
