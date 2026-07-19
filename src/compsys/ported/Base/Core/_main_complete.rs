//! Port of `_main_complete` from
//! `Completion/Base/Core/_main_complete`.
//!
//! Full upstream body (418 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 25  local func funcs ret=1 tmp _compskip … _saved_* state snapshots
//! sh: 52  [[ -z "$curcontext" ]] && curcontext=:::          # FLOOR
//! sh: 56  zstyle -s … insert-tab tmp → pending-tab short-circuit
//! sh: 70  GLOB_COMPLETE second-attempt re-prep
//! sh: 79  special-context dispatch: equals, ~[, …
//! sh:120  collect completer chain (style + default)
//! sh:170  for _completer in chain: build curcontext, run matcher-list × completer
//! sh:340  if ret != 0 and we have a default-message format, _message
//! sh:380  post-funcs
//! sh:400  restore compstate snapshots
//! sh:418  return ret
//! ```
//!
//! `_main_complete` is the primary entry-point invoked by every
//! completion widget. Ported behaviors (sh:N → impl-line):
//!   * sh:52    curcontext `:::` floor
//!   * sh:60-68 pending-tab short-circuit (`insert-tab=pending`)
//!   * sh:70-79 tab-init handling (consume `tab` from `compstate[insert]`)
//!   * sh:83-89 GLOB_COMPLETE second-attempt PREFIX/SUFFIX split
//!   * sh:91-106 special-context dispatch: `=`, `~[`, `~user`
//!   * sh:110   `_setup default`
//!   * sh:122-133 list-prompt / select-prompt / select-scroll styles
//!   * sh:137-151 completer-chain `-`/call-mode form
//!   * sh:170-340 matcher-list × completer-fn nested loop
//!   * sh:350-371 warnings-format emission when nm==0
//!   * sh:373-378 ambiguous-color injection into `_comp_colors`
//!   * sh:380-382 force-list dispatch
//!   * sh:399-405 post-funcs (`comppostfuncs`)
//!   * sh:407-417 `_lastcomp` snapshot
//!   * sh:384-396 ZLS_COLORS save/restore

use crate::compsys::ported::_setup::_setup;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::{bin_zformat, lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getiparam, getsparam, setaparam, setsparam, unsetparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::{bin_compadd, bin_compset};
use crate::ported::zsh_h::{isset, options, EQUALSOPT, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:25 — `eval "$_comp_setup"`, the option half. Upstream's
/// `_comp_setup` (compinit sh:180-190) runs `setopt localoptions
/// localtraps localpatterns ${_comp_options[@]}` so every completion
/// function executes under the 33 canonical options regardless of the
/// user's global state — e.g. a global `setopt cshnullglob` must NOT
/// leak in (compinit forces `NO_cshnullglob`), or any glob-failing
/// word expansion inside a completion function prints the csh-style
/// `no match` error mid-completion (subst.c:507). Since this port is
/// a Rust fn with no shell function scope for `localoptions`, the
/// guard saves each option it changes and restores on drop (every
/// return path). Nesting is safe: an inner guard saves the
/// already-applied states and its drop is a no-op relative to the
/// outer guard's restore. IFS is likewise forced to the standard
/// `$' \t\r\n\0'` (sh:183 `local IFS=...`) and restored. The
/// remaining `_comp_setup` pieces (`exec </dev/null`, `trap - ZERR`,
/// `enable -p` pattern chars) are not yet applied here.
struct CompSetupGuard {
    saved_opts: Vec<(i32, bool)>,
    saved_ifs: Option<String>,
}

impl CompSetupGuard {
    fn apply() -> Self {
        use crate::ported::options::{dosetopt, optlookup};
        use crate::ported::zsh_h::OPT_INVALID;
        let mut saved_opts = Vec::new();
        for entry in crate::compsys::ported::compinit::COMP_OPTIONS {
            let (name, want) = match entry.strip_prefix("NO_") {
                Some(rest) => (rest, false),
                None => (*entry, true),
            };
            let optno = optlookup(name);
            if optno == OPT_INVALID {
                continue;
            }
            // Alias rows resolve negative; normalise to the real index
            // (dosetopt would re-negate the value, but isset needs the
            // positive index and the alias sign is already folded into
            // `want` by the canonical names in COMP_OPTIONS).
            let idx = optno.abs();
            let cur = isset(idx);
            if cur != want {
                saved_opts.push((idx, cur));
                dosetopt(idx, want as i32, 0);
            }
        }
        let saved_ifs = getsparam("IFS");
        let _ = crate::ported::params::setsparam("IFS", " \t\r\n\0");
        Self {
            saved_opts,
            saved_ifs,
        }
    }
}

impl Drop for CompSetupGuard {
    fn drop(&mut self) {
        use crate::ported::options::dosetopt;
        for &(idx, was) in self.saved_opts.iter().rev() {
            dosetopt(idx, was as i32, 0);
        }
        match self.saved_ifs.take() {
            Some(ifs) => {
                let _ = crate::ported::params::setsparam("IFS", &ifs);
            }
            None => {
                unsetparam("IFS");
            }
        }
    }
}

/// `_main_complete` — primary completion-dispatch entry. Args
/// (when non-empty) override the configured `completer` style with
/// the supplied chain.
pub fn _main_complete(args: &[String]) -> i32 {
    tracing::debug!(target: "compsys_args", ?args, "_main_complete ENTER");
    // Merge any finished background compinit scan BEFORE the completer
    // lookup. The bg worker ships _comps/_services/_patcomps back over a
    // channel, but the lazy drain's only other caller was the --doctor
    // benchmark — in a live shell $_comps stayed EMPTY, `_dispatch`
    // resolved comp="" for every command, and everything fell to
    // -default- file completion (option completion dead shell-wide).
    crate::fusevm_bridge::drain_compinit_bg_hook();
    // sh:25 — `eval "$_comp_setup"` (options + IFS half); restores on
    // every return path via Drop.
    let _comp_setup = CompSetupGuard::apply();
    // sh:25  snapshot compstate so we can restore on exit
    let saved_curcontext = getsparam("curcontext").unwrap_or_default();
    let saved_compskip = getsparam("_compskip").unwrap_or_default();
    let saved_exact = get_compstate_str("exact").unwrap_or_default();
    let saved_lastprompt = get_compstate_str("last_prompt").unwrap_or_default();
    let saved_list = get_compstate_str("list").unwrap_or_default();
    let saved_insert = get_compstate_str("insert").unwrap_or_default();
    let saved_colors = getsparam("ZLS_COLORS").unwrap_or_default();
    let saved_colors_set = getsparam("ZLS_COLORS").is_some();
    let _ = setsparam("_saved_exact", &saved_exact);
    let _ = setsparam("_saved_lastprompt", &saved_lastprompt);
    let _ = setsparam("_saved_list", &saved_list);
    let _ = setsparam("_saved_insert", &saved_insert);

    // sh:52  curcontext floor — the bug the user flagged earlier:
    //   without this, every downstream zstyle query goes to the
    //   wrong field position.
    if saved_curcontext.is_empty() {
        let _ = setsparam("curcontext", ":::");
    }
    let mut curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:60-68  pending-tab short-circuit
    let insert_tab = lookupstyle(&format!(":completion:{}:", curcontext), "insert-tab")
        .first()
        .cloned()
        .unwrap_or_else(|| "yes".to_string());
    let pending = getiparam("PENDING");
    let pending_match = if insert_tab.contains("pending") {
        if let Some(eq_pos) = insert_tab.find("pending=") {
            let tail = &insert_tab[eq_pos + 8..];
            let n: i64 = tail
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(1);
            pending >= n
        } else {
            pending > 0
        }
    } else {
        false
    };
    if pending_match {
        set_compstate_str("insert", "tab");
        return 0;
    }

    // sh:70-79  tab-init handling — if the user pressed TAB and
    //   insert-tab is on for non-vared context, exit immediately.
    let cur_insert = get_compstate_str("insert").unwrap_or_default();
    if cur_insert.starts_with("tab") {
        let on_tab = matches!(insert_tab.trim(), "yes" | "true" | "on" | "1")
            || insert_tab.starts_with("yes ")
            || insert_tab.starts_with("true ")
            || insert_tab.starts_with("on ")
            || insert_tab.starts_with("1 ");
        let vared = get_compstate_str("vared").unwrap_or_default();
        if on_tab
            && (!curcontext.starts_with(':')
                || vared.is_empty()
                || testforstyle(&format!(":completion:vared{}:", curcontext), "insert-tab") == 0)
        {
            return 0;
        }
        // Strip the leading `tab` from compstate[insert]
        let stripped = cur_insert.replace("tab ", "");
        set_compstate_str("insert", &stripped);
    }

    // sh:83-89  GLOB_COMPLETE second-attempt: split PREFIX at the
    //   prior `_lastcomp[unambiguous_cursor]` so the user's typed
    //   characters split into a fresh PREFIX/SUFFIX pair.
    if get_compstate_str("pattern_match").as_deref() == Some("*") {
        let last_prefix = lastcomp_get("unambiguous").unwrap_or_default();
        let prefix = getsparam("PREFIX").unwrap_or_default();
        if last_prefix == prefix {
            if let Some(upos_str) = lastcomp_get("unambiguous_cursor") {
                if let Ok(upos) = upos_str.parse::<usize>() {
                    if upos > 0 && upos <= prefix.len() {
                        let suffix = getsparam("SUFFIX").unwrap_or_default();
                        let new_prefix = &prefix[..upos - 1];
                        let new_suffix = format!("{}{}", &prefix[upos - 1..], suffix);
                        let _ = setsparam("PREFIX", new_prefix);
                        let _ = setsparam("SUFFIX", &new_suffix);
                    }
                }
            }
        }
    }

    // sh:91-106  Special-context dispatch: `=`, `~[`, `~user`.
    let quote = get_compstate_str("quote").unwrap_or_default();
    if quote.is_empty() {
        let prefix = getsparam("PREFIX").unwrap_or_default();
        // sh:94 — `equals` option + leading `=`
        if isset(EQUALSOPT)
            && bin_compset(
                "compset",
                &["-P".to_string(), "1".to_string(), "=".to_string()],
                &make_ops(),
                0,
            ) == 0
        {
            set_compstate_str("context", "equal");
        } else if prefix.starts_with("~[") {
            // sh:96  Inside ~[...] → subscript context.
            let _ = bin_compset(
                "compset",
                &["-p".to_string(), "2".to_string()],
                &make_ops(),
                0,
            );
            let _ = bin_compset(
                "compset",
                &["-S".to_string(), "\\]*".to_string()],
                &make_ops(),
                0,
            );
            set_compstate_str("context", "subscript");
        } else if prefix.starts_with('~') && !prefix.contains('/') {
            // sh:102  ~user
            let _ = bin_compset(
                "compset",
                &["-p".to_string(), "1".to_string()],
                &make_ops(),
                0,
            );
            set_compstate_str("context", "tilde");
        }
    }

    // sh:110  _setup default — propagate the default-tag styles
    //   (list-packed, accept-exact, …) into compstate.
    let _ = _setup(&["default".to_string()]);

    // sh:122-133  list-prompt / select-prompt / select-scroll styles
    let ctx_default = format!(":completion:{}:default", curcontext);
    // sh:128,132,136 — each of list-prompt / select-prompt / select-scroll,
    // when set, ALSO does `zmodload -i zsh/complist`. This loads the module
    // whose `boot_` registers `complistmatches` as the `comp_list_matches`
    // hookfunc — the scroll-paged listing. Without it the hookdef stays at the
    // plain `ilistmatches`, so `LISTPROMPT` is set but never read and long
    // lists dump / fall back to the "see all N possibilities" query instead of
    // paging. The port had this `zmodload` on the menu path (sh:306/322) but
    // dropped it here, so list-prompt paging never fired.
    let load_complist = || {
        let mut ops_i = make_ops();
        ops_i.ind[b'i' as usize] = 1;
        let _ = crate::ported::module::bin_zmodload(
            "zmodload",
            &["zsh/complist".to_string()],
            &ops_i,
            0,
        );
    };
    if let Some(v) = lookupstyle(&ctx_default, "list-prompt").first() {
        let _ = setsparam("LISTPROMPT", v);
        load_complist(); // sh:128
    }
    if let Some(v) = lookupstyle(&ctx_default, "select-prompt").first() {
        let _ = setsparam("MENUPROMPT", v);
        load_complist(); // sh:132
    }
    if let Some(v) = lookupstyle(&ctx_default, "select-scroll").first() {
        let _ = setsparam("MENUSCROLL", v);
        load_complist(); // sh:136
    }

    // sh:31-33  global tag-tracking state init
    let _ = setsparam("_tags_level", "0");
    let _ = setsparam("_comp_tags", "");
    let _ = setsparam("_comp_mesg", "");
    setaparam("_lastdescr", Vec::new());
    setaparam("_comp_ignore", Vec::new());
    setaparam("_comp_colors", Vec::new());
    // sh:54 — `typeset -U _lastdescr _comp_ignore _comp_colors`. This
    // `typeset -U` declaration was not ported: the arrays are created above via
    // `setaparam` (plain, no PM_UNIQUE), so every later `+=` append (`_setup`
    // per completer, `_path_files`/`_files` ignore accumulation, the ambiguous-
    // color injection) duplicates entries. zshrs's `setaparam` DOES honor
    // PM_UNIQUE (params.rs:7367/4627 → arrunique), so flagging the params once
    // here makes all subsequent appends dedupe automatically, matching zsh.
    {
        let mut tab = crate::ported::params::paramtab().write().unwrap();
        for nm in ["_lastdescr", "_comp_ignore", "_comp_colors"] {
            if let Some(pm) = tab.get_mut(nm) {
                pm.node.flags |= crate::ported::zsh_h::PM_UNIQUE as i32;
            }
        }
    }

    // sh:137-151  completer chain
    //   `-` as first arg + ≥3 args → run only argv[1] (call mode)
    //   else when args non-empty → use args verbatim
    //   else read `completer` style, default `(_complete _ignored)`
    let chain: Vec<String> = if !args.is_empty() {
        if args[0] == "-" {
            if args.len() < 3 {
                Vec::new()
            } else {
                vec![args[1].clone()]
            }
        } else {
            args.to_vec()
        }
    } else {
        let style_chain = lookupstyle(&format!(":completion:{}:", curcontext), "completer");
        if !style_chain.is_empty() {
            style_chain
        } else {
            // sh:150 default
            vec!["_complete".to_string(), "_ignored".to_string()]
        }
    };

    // Publish the chain so other completers (e.g. _prefix / _ignored)
    //   can inspect it via `$_completers`.
    setaparam("_completers", chain.clone());

    let mut ret: i32 = 1;
    let mut completer_num: i64 = 1;
    for completer_spec in &chain {
        let _ = setsparam("_completer_num", &completer_num.to_string());

        // sh:165  split `spec` on `:` — left of `:` is the fn name,
        //   right is the curcontext-field suffix.
        let mut parts = completer_spec.splitn(2, ':');
        let bare = parts.next().unwrap_or("").to_string();
        let field_suffix = parts.next().map(|s| s.to_string()).unwrap_or_else(|| {
            bare.strip_prefix('_')
                .map(|s| s.replace('_', "-"))
                .unwrap_or_default()
        });
        let _ = setsparam("_completer", &field_suffix);

        // sh:175  curcontext patch: replace middle `:`-field
        let new_ctx = patch_completer_field(&curcontext, &field_suffix);
        let _ = setsparam("curcontext", &new_ctx);
        curcontext = new_ctx;

        // sh:184-185 — `zstyle -t … show-completer && zle -R "Trying completion
        // for :completion:${curcontext}"`. When the show-completer style is set,
        // flash a progress line naming the completer context being tried (a
        // debugging aid). `zle -R <msg>` = bin_zle_refresh with the message as
        // its sole positional (sets the statusline + refreshes).
        if testforstyle(&format!(":completion:{}:", curcontext), "show-completer") == 0 {
            let msg = format!("Trying completion for :completion:{}", curcontext);
            let _ = crate::ported::zle::zle_thingy::bin_zle_refresh(
                "zle",
                std::slice::from_ref(&msg),
                &make_ops(),
                0,
            );
        }

        // sh:180  matcher-list loop
        let matchers = lookupstyle(&format!(":completion:{}:", curcontext), "matcher-list");
        let matcher_list: Vec<String> = if matchers.is_empty() {
            vec!["".to_string()]
        } else {
            matchers
        };
        let mut matcher_num: i64 = 1;
        let mut combined_matcher = String::new();
        for m in &matcher_list {
            let _ = setsparam("_matcher_num", &matcher_num.to_string());
            if let Some(rest) = m.strip_prefix('+') {
                combined_matcher = format!("{} {}", combined_matcher, rest);
            } else {
                combined_matcher = m.clone();
            }
            let _ = setsparam("_matcher", combined_matcher.trim());

            if dispatch_function_call(&bare, &[]).unwrap_or(1) == 0 {
                ret = 0;
                break;
            }
            matcher_num += 1;
        }
        if ret == 0 {
            break;
        }
        completer_num += 1;
    }

    // Snapshot for the warnings/format branch
    let nm: i64 = get_compstate_str("nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let comp_mesg = getsparam("_comp_mesg").unwrap_or_default();
    let old_list = get_compstate_str("old_list").unwrap_or_default();

    // sh:234-349 — menu-completion decision. When there are enough matches
    // (or we kept an old list), evaluate the `menu` style (stashed by
    // `_setup` into `_last_menu_style`, combined here with `_menu_style` and
    // `_def_menu_style`) to decide whether `compstate[insert]` becomes
    // `menu` and, if so, whether interactive menu-selection (`MENUSELECT`/
    // `MENUMODE`) is enabled. Without this the `menu select` style was inert
    // — `compstate[insert]` never became `menu`, so menucmp never set and
    // the interactive menu never started.
    if old_list == "keep" || nm > 1 {
        // sh:236-237 — re-prepend last-round styles if the count changed.
        let last_nm: i64 = getsparam("_last_nmatches")
            .and_then(|s| s.parse().ok())
            .unwrap_or(-1);
        if last_nm >= 0 && last_nm != nm {
            let mut ms = getaparam("_last_menu_style").unwrap_or_default();
            ms.extend(getaparam("_menu_style").unwrap_or_default());
            setaparam("_menu_style", ms);
        }
        // sh:239 — tmp = list_lines + BUFFERLINES + 1.
        let list_lines: i64 = get_compstate_str("list_lines")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let bufferlines = getiparam("BUFFERLINES");
        let lines = getiparam("LINES");
        let tmp = list_lines + bufferlines + 1;
        // sh:241 — append the default menu styles.
        let mut menu_style = getaparam("_menu_style").unwrap_or_default();
        menu_style.extend(getaparam("_def_menu_style").unwrap_or_default());
        setaparam("_menu_style", menu_style.clone());

        // `_menu_style[(r)PAT]` — first element matching a simple glob.
        let has = |pat_fn: &dyn Fn(&str) -> bool| menu_style.iter().any(|e| pat_fn(e.as_str()));
        // sh:244-245 — select=long-list OR (yes|true|on|1)=long-list.
        let long_list = has(&|e| {
            e == "select=long-list"
                || matches!(e, "yes=long-list" | "true=long-list" | "on=long-list" | "1=long-list")
        });
        let list_has = get_compstate_str("list")
            .map(|l| l == "list" || l.contains(" list") || l.starts_with("list "))
            .unwrap_or(false);
        let cur_insert = get_compstate_str("insert").unwrap_or_default();

        // Compute the smallest numeric threshold across elements matching a
        // yes-like prefix (sh:252-267); mirrors the C min/max loops: a bare
        // word (or `=word`) → 0, `=N` → N (clamped ≥0), otherwise 9999999.
        let threshold = |starts: &dyn Fn(&str) -> bool| -> Option<i64> {
            let sel: Vec<&String> = menu_style.iter().filter(|e| starts(e.as_str())).collect();
            if sel.is_empty() {
                return None;
            }
            let mut m = 9999999i64;
            for i in sel {
                let num = if let Some(eq) = i.find('=') {
                    let rest = &i[eq + 1..];
                    if rest.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        rest.parse::<i64>().unwrap_or(0).max(0)
                    } else {
                        9999999
                    }
                } else {
                    0
                };
                if num < m {
                    m = num;
                }
                if m == 0 {
                    break;
                }
            }
            Some(m)
        };
        let yes_like = |e: &str| {
            e.starts_with("yes") || e.starts_with("true") || e.starts_with("1") || e.starts_with("on")
        };
        let no_like = |e: &str| {
            e.starts_with("no") || e.starts_with("false") || e.starts_with("0") || e.starts_with("off")
        };
        let auto = has(&|e| e.starts_with("auto"));

        if list_has && tmp > lines && long_list {
            set_compstate_str("insert", "menu"); // sh:246
        } else if cur_insert == saved_insert {
            // sh:247
            let long = has(&|e| {
                matches!(e, "yes=long" | "true=long" | "1=long" | "on=long")
            });
            if !cur_insert.is_empty() && long && tmp > lines {
                set_compstate_str("insert", "menu"); // sh:250
            } else {
                let min = threshold(&|e: &str| yes_like(e)); // sh:252-267
                let max = threshold(&|e: &str| no_like(e)); // sh:270-285
                if (min.is_some_and(|mn| nm >= mn) && max.map(|mx| nm < mx).unwrap_or(true))
                    || (auto && cur_insert == "automenu")
                {
                    set_compstate_str("insert", "menu"); // sh:291
                } else if max.is_some_and(|mx| nm >= mx) {
                    set_compstate_str("insert", "unambiguous"); // sh:293
                } else if auto && cur_insert != "automenu" {
                    set_compstate_str("insert", "automenu-unambiguous"); // sh:296
                }
            }
        }

        // sh:301-349 — MENUSELECT/MENUMODE setup for `*menu*` inserts.
        if get_compstate_str("insert").unwrap_or_default().contains("menu") {
            if getsparam("MENUSELECT").as_deref() == Some("00") {
                let _ = setsparam("MENUSELECT", "0"); // sh:302
            }
            if has(&|e| e.starts_with("no-select")) {
                unsetparam("MENUSELECT"); // sh:304
            } else if has(&|e| e.starts_with("select=long")) {
                if tmp > lines {
                    let mut ops_i = make_ops();
                    ops_i.ind[b'i' as usize] = 1;
                    let _ = crate::ported::module::bin_zmodload(
                        "zmodload",
                        &["zsh/complist".to_string()],
                        &ops_i,
                        0,
                    ); // sh:306 zmodload -i zsh/complist
                    let _ = setsparam("MENUSELECT", "00"); // sh:307
                }
            }
            if getsparam("MENUSELECT").as_deref() != Some("00") {
                if let Some(min) = threshold(&|e: &str| e.starts_with("select")) {
                    let mut ops_i = make_ops();
                    ops_i.ind[b'i' as usize] = 1;
                    let _ = crate::ported::module::bin_zmodload(
                        "zmodload",
                        &["zsh/complist".to_string()],
                        &ops_i,
                        0,
                    ); // sh:322 zmodload -i zsh/complist
                    let _ = setsparam("MENUSELECT", &min.to_string()); // sh:323
                } else {
                    unsetparam("MENUSELECT"); // sh:325
                }
            }
            if getsparam("MENUSELECT").is_some() {
                if has(&|e| e.starts_with("interactive")) {
                    let _ = setsparam("MENUMODE", "interactive"); // sh:338
                } else if has(&|e| e.starts_with("search")) {
                    if has(&|e| e.contains("backward")) {
                        let _ = setsparam("MENUMODE", "search-backward"); // sh:341
                    } else {
                        let _ = setsparam("MENUMODE", "search-forward"); // sh:343
                    }
                } else {
                    unsetparam("MENUMODE"); // sh:346
                }
            }
        }
    }
    // sh:350-352 — no matches but a message was set: list it
    else if nm < 1 && !comp_mesg.is_empty() {
        set_compstate_str("insert", "");
        set_compstate_str("list", "list force");
    } else if nm == 0 && comp_mesg.is_empty() && old_list != "keep" {
        // sh:353-371  warnings format emission.
        //
        // Rust-only guard (no C counterpart): the warning fires when
        // `$compstate[nmatches]` reads 0. In zsh `nmatches` is a live GSU
        // integer that always reflects the running match count, so a group
        // that produced matches (e.g. git's `common-commands`, 23 rows) keeps
        // nm > 0 and this branch never runs. In the port `nmatches` is a
        // cached counter recomputed by `permmatches`; a group whose matches
        // still sit in the file-scope `matches`/`fmatches` accumulators —
        // added by `compadd` but not yet flushed into the group's `lmatches`
        // by `endcmgroup` — is invisible to that counter, so nm reads a stale
        // 0 even though live matches exist. Re-check the live accumulators
        // here (read-only; does not touch the count the completer loop /
        // `_tags` already consumed) and suppress the warning when real matches
        // are pending. Without this git's `common-commands` group gets a
        // spurious `-<<No Matches for `common commands'>>-` header above its
        // 23 rows; zsh emits none.
        let live_pending = {
            use crate::ported::zle::compcore as cc;
            let m = cc::matches
                .get()
                .map(|l| l.lock().map(|g| g.len()).unwrap_or(0))
                .unwrap_or(0);
            let fm = cc::fmatches
                .get()
                .map(|l| l.lock().map(|g| g.len()).unwrap_or(0))
                .unwrap_or(0);
            m + fm
        };
        let lastdescr = getaparam("_lastdescr").unwrap_or_default();
        let warn_format = lookupstyle(&format!(":completion:{}:warnings", curcontext), "format")
            .first()
            .cloned()
            .unwrap_or_default();
        if live_pending == 0 && !lastdescr.is_empty() && !warn_format.is_empty() {
            set_compstate_str("list", "list force");
            set_compstate_str("insert", "");
            let quoted: Vec<String> = lastdescr.iter().map(|d| format!("`{}'", d)).collect();
            let str_msg = match quoted.len() {
                1 => quoted[0].clone(),
                2 => format!("{} or {}", quoted[0], quoted[1]),
                _ => {
                    let init = quoted[..quoted.len() - 1].join(", ");
                    format!("{}, or {}", init, quoted[quoted.len() - 1])
                }
            };
            let _ = _setup(&["warnings".to_string()]);
            let zf_argv = vec![
                "-f".to_string(),
                "mesg".to_string(),
                warn_format.clone(),
                format!("d:{}", str_msg),
                format!("D:{}", lastdescr.join("\n")),
            ];
            let _ = setsparam("mesg", "");
            let _ = bin_zformat("zformat", &zf_argv, &make_ops(), 0);
            let mesg = getsparam("mesg").unwrap_or_else(|| warn_format.clone());
            let _ = bin_compadd("compadd", &["-x".to_string(), mesg], &make_ops(), 0);
        }
    }

    // sh:373-378  ambiguous-color injection
    let ambig_color = getsparam("_ambiguous_color").unwrap_or_default();
    if !ambig_color.is_empty() {
        let unambig = get_compstate_str("unambiguous").unwrap_or_default();
        let upos: usize = get_compstate_str("unambiguous_cursor")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if upos > 0 && upos <= unambig.len() + 1 {
            let prefix_chars = &unambig[..upos.saturating_sub(1)];
            if !prefix_chars.is_empty() {
                // `_comp_colors` is PM_UNIQUE (sh:54) — setaparam dedupes.
                let mut colors = getaparam("_comp_colors").unwrap_or_default();
                colors.push(format!(
                    "=(#i){}*=={}",
                    glob_escape(prefix_chars),
                    ambig_color
                ));
                setaparam("_comp_colors", colors);
            }
        }
    }

    // sh:380-382  force-list when style says so
    let force_list = getsparam("_comp_force_list").unwrap_or_default();
    let force_at: i64 = force_list.parse().unwrap_or(0);
    if force_list == "always" || (!force_list.is_empty() && nm >= force_at) {
        let mut list_val = get_compstate_str("list").unwrap_or_default();
        list_val = list_val.replace("messages", "");
        if !list_val.contains("force") {
            list_val = format!("{} force", list_val.trim());
        }
        set_compstate_str("list", list_val.trim());
    }

    // sh:399-405  post-funcs (snapshot + clear so we don't loop)
    let postfuncs = getaparam("comppostfuncs").unwrap_or_default();
    setaparam("comppostfuncs", Vec::new());
    for pf in &postfuncs {
        let _ = dispatch_function_call(pf, &[]);
    }

    // sh:407-417  _lastcomp snapshot — flat key/value array
    let mut lastcomp: Vec<String> = Vec::new();
    lastcomp.push("nmatches".to_string());
    lastcomp.push(nm.to_string());
    lastcomp.push("completer".to_string());
    lastcomp.push(getsparam("_completer").unwrap_or_default());
    lastcomp.push("prefix".to_string());
    lastcomp.push(getsparam("PREFIX").unwrap_or_default());
    lastcomp.push("suffix".to_string());
    lastcomp.push(getsparam("SUFFIX").unwrap_or_default());
    lastcomp.push("iprefix".to_string());
    lastcomp.push(getsparam("IPREFIX").unwrap_or_default());
    lastcomp.push("isuffix".to_string());
    lastcomp.push(getsparam("ISUFFIX").unwrap_or_default());
    lastcomp.push("qiprefix".to_string());
    lastcomp.push(getsparam("QIPREFIX").unwrap_or_default());
    lastcomp.push("qisuffix".to_string());
    lastcomp.push(getsparam("QISUFFIX").unwrap_or_default());
    lastcomp.push("tags".to_string());
    lastcomp.push(getsparam("_comp_tags").unwrap_or_default());
    lastcomp.push("insert".to_string());
    lastcomp.push(get_compstate_str("insert").unwrap_or_default());
    lastcomp.push("unambiguous".to_string());
    lastcomp.push(get_compstate_str("unambiguous").unwrap_or_default());
    lastcomp.push("unambiguous_cursor".to_string());
    lastcomp.push(get_compstate_str("unambiguous_cursor").unwrap_or_default());
    setaparam("_lastcomp", lastcomp);

    // sh:384-396  always-block: ZLS_COLORS save/restore.
    if get_compstate_str("old_list").as_deref() == Some("keep") {
        if saved_colors_set {
            let _ = setsparam("ZLS_COLORS", &saved_colors);
        } else {
            unsetparam("ZLS_COLORS");
        }
    } else {
        let comp_colors = getaparam("_comp_colors").unwrap_or_default();
        if !comp_colors.is_empty() {
            let _ = setsparam("ZLS_COLORS", &comp_colors.join(":"));
        } else {
            unsetparam("ZLS_COLORS");
        }
    }

    // C does NOT restore compstate[insert]/[exact]/[list]/[last_prompt]:
    // `_saved_*` are C locals (set at sh:34-37) that are only READ (the
    // menu decision compares `_saved_insert` at sh:247) — never written
    // back to compstate. The completion's decisions (compstate[insert]=menu
    // from the menu block, list='list force' from force-list, exact from
    // _setup) MUST persist so the completion core reads them back
    // (compcore.c:857 → useline/usemenu → menucmp → the menu_start hook).
    // The earlier port restored all four, wiping the menu decision every
    // time — `menu select` was inert and interactive menu-select never
    // started. Only curcontext/_compskip are restored here, emulating C's
    // `local` scoping (they ARE locals in the C fn, unlike compstate).
    let _ = setsparam("curcontext", &saved_curcontext);
    let _ = setsparam("_compskip", &saved_compskip);
    let _ = (&saved_exact, &saved_lastprompt, &saved_list, &saved_insert); // saved as _saved_* params above (sh:34-37) for other fns
    ret
}

/// Glob-escape regex metacharacters for the ambiguous-color
/// injection (sh:373's `_comp_colors` entry needs the prefix
/// quoted so the regex matcher treats it literally).
fn glob_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if matches!(
            c,
            '=' | '(' | ')' | '|' | '~' | '^' | '?' | '*' | '[' | ']' | '#' | '<' | '>'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `_lastcomp[key]` lookup (`_lastcomp` is the prior-call snapshot).
fn lastcomp_get(key: &str) -> Option<String> {
    let arr = getaparam("_lastcomp")?;
    arr.chunks(2)
        .find(|kv| kv.first().map(|k| k == key).unwrap_or(false))
        .and_then(|kv| kv.get(1).cloned())
}

/// sh:175 — replace the middle `:`-field of `curcontext` with the
/// completer's name. For `a:b:c:d` and `complete`, result is
/// `a:complete:c:d`.
fn patch_completer_field(curcontext: &str, completer: &str) -> String {
    let mut parts: Vec<&str> = curcontext.split(':').collect();
    if parts.len() < 4 {
        // Pad with empty fields to get 4 colons
        while parts.len() < 4 {
            parts.push("");
        }
    }
    parts[1] = completer;
    parts.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_curcontext_initializes_floor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("curcontext", "");
        let _ = _main_complete(&[]);
        // After return, curcontext is restored to ""
        assert_eq!(getsparam("curcontext").as_deref(), Some(""));
    }

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("curcontext", "a:b:c:d");
        assert_eq!(_main_complete(&[]), 1);
    }

    /// sh:25 — `eval "$_comp_setup"`: the user's global options must
    /// not leak into completion (a global `setopt cshnullglob` made
    /// any glob-failing expansion inside a completion function print
    /// the csh `no match` error mid-completion), and the pre-existing
    /// state must be restored on exit. Verified live against zpwr:
    /// csh=off/null=on/aliases=off inside the completer, restored after.
    #[test]
    fn comp_setup_guard_forces_and_restores_options() {
        use crate::ported::options::{dosetopt, optlookup};
        let _g = crate::test_util::global_state_lock();
        let csh = optlookup("cshnullglob").abs();
        let null = optlookup("nullglob").abs();
        let was_csh = isset(csh);
        let was_null = isset(null);
        // Simulate the user's global state: cshnullglob ON, nullglob OFF.
        dosetopt(csh, 1, 0);
        dosetopt(null, 0, 0);
        {
            let _setup = CompSetupGuard::apply();
            // _comp_options forces NO_cshnullglob + nullglob.
            assert!(!isset(csh), "cshnullglob must be forced off");
            assert!(isset(null), "nullglob must be forced on");
            // sh:183 — IFS forced to the standard set (with \r).
            assert_eq!(getsparam("IFS").as_deref(), Some(" \t\r\n\0"));
        }
        // Guard dropped — the simulated globals are back.
        assert!(isset(csh), "cshnullglob must be restored");
        assert!(!isset(null), "nullglob must be restored");
        // Restore the real pre-test state.
        dosetopt(csh, was_csh as i32, 0);
        dosetopt(null, was_null as i32, 0);
    }

    #[test]
    fn explicit_chain_overrides_style() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("curcontext", "a:b:c:d");
        let _ = _main_complete(&["_complete".to_string()]);
        let chain = getaparam("_completers").unwrap_or_default();
        assert_eq!(chain, vec!["_complete"]);
    }

    #[test]
    fn patch_completer_field_replaces_middle() {
        assert_eq!(
            patch_completer_field("a:b:c:d", "complete"),
            "a:complete:c:d"
        );
    }

    // ========================================================
    // patch_completer_field — boundary cases
    // ========================================================

    #[test]
    fn patch_completer_field_pads_short_context_to_four_fields() {
        // 2 fields → padded with two empties → patched at idx 1.
        assert_eq!(patch_completer_field("a:b", "x"), "a:x::");
    }

    #[test]
    fn patch_completer_field_handles_empty_context_padded() {
        // Empty string splits to one segment `""` → padded to 4.
        assert_eq!(patch_completer_field("", "x"), ":x::");
    }

    #[test]
    fn patch_completer_field_preserves_extra_fields_past_four() {
        // 5+ fields should still patch idx 1, leave the rest alone.
        assert_eq!(
            patch_completer_field("a:b:c:d:e", "complete"),
            "a:complete:c:d:e"
        );
    }

    #[test]
    fn patch_completer_field_overwrites_empty_completer_too() {
        // Completer can legitimately be empty — must not be coerced.
        assert_eq!(patch_completer_field("a:b:c:d", ""), "a::c:d");
    }

    #[test]
    fn patch_completer_field_handles_colon_floor_three_colons() {
        // The classic `:::` floor from sh:52.
        assert_eq!(patch_completer_field(":::", "y"), ":y::");
    }

    // ========================================================
    // glob_escape — regex / glob metachar quoting
    // ========================================================

    #[test]
    fn glob_escape_quotes_each_metachar_with_backslash() {
        // Every metachar in the impl list gets a `\` prefix.
        let metas = "=()|~^?*[]#<>";
        let escaped = glob_escape(metas);
        // Each input char produces a `\X` pair → length doubles.
        assert_eq!(
            escaped.len(),
            metas.len() * 2,
            "expected every char doubled, got: {}",
            escaped
        );
        // Every metachar should be preceded by a backslash.
        for c in metas.chars() {
            let pair = format!("\\{}", c);
            assert!(escaped.contains(&pair), "missing {} in {}", pair, escaped);
        }
    }

    #[test]
    fn glob_escape_leaves_ordinary_ascii_unchanged() {
        assert_eq!(glob_escape("hello world 123"), "hello world 123");
    }

    #[test]
    fn glob_escape_empty_input_returns_empty() {
        assert_eq!(glob_escape(""), "");
    }

    #[test]
    fn glob_escape_mixed_text_escapes_only_meta_chars() {
        let s = glob_escape("foo[bar]*baz");
        assert_eq!(s, r"foo\[bar\]\*baz");
    }

    #[test]
    fn glob_escape_does_not_escape_unrelated_punctuation() {
        // `,` and `.` and `-` are NOT in the meta set.
        let s = glob_escape("a-b.c,d");
        assert_eq!(s, "a-b.c,d");
    }

    // ========================================================
    // lastcomp_get — kv-array lookup
    // ========================================================

    #[test]
    fn lastcomp_get_returns_none_when_unset() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("_lastcomp");
        assert!(lastcomp_get("anything").is_none());
    }

    #[test]
    fn lastcomp_get_returns_value_when_key_present() {
        let _g = crate::test_util::global_state_lock();
        setaparam(
            "_lastcomp",
            vec![
                "context".to_string(),
                "a:b:c:d".to_string(),
                "completer".to_string(),
                "_complete".to_string(),
            ],
        );
        assert_eq!(lastcomp_get("completer").as_deref(), Some("_complete"));
        assert_eq!(lastcomp_get("context").as_deref(), Some("a:b:c:d"));
    }

    #[test]
    fn lastcomp_get_returns_none_when_key_missing() {
        let _g = crate::test_util::global_state_lock();
        setaparam("_lastcomp", vec!["k1".to_string(), "v1".to_string()]);
        assert!(lastcomp_get("never-set").is_none());
    }
}
