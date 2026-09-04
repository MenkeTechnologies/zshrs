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
use crate::ported::params::{
    getaparam, gethkparam, gethparam, getiparam, getsparam, setaparam, sethparam, setsparam,
    unsetparam,
};
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

/// `$_comps[key]` — one key of the completer registry (sh:104).
///
/// `_comps` is a PM_HASHED assoc, and `getaparam` returns `None` for a hash
/// (params.c:3108 tests `PM_TYPE(...) == PM_ARRAY`), so it has to be read out
/// of the hashed backing store — the same route `_dispatch`/`_complete` take.
fn comps_entry(key: &str) -> String {
    crate::ported::params::paramtab_hashed_storage()
        .lock()
        .ok()
        .and_then(|t| t.get("_comps").and_then(|h| h.get(key).cloned()))
        .unwrap_or_default()
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

/// sh:164-166 / sh:169-171 — the two handler bodies, verbatim. Only the
/// exit status differs (130 = 128+SIGINT, 131 = 128+SIGQUIT).
const COMP_TRAPS: [(&str, &str); 2] = [
    (
        "TRAPINT",
        "\tzle -M \"Killed by signal in ${funcstack[2]} after ${SECONDS}s\";\n\tzle -R\n\treturn 130",
    ),
    (
        "TRAPQUIT",
        "\tzle -M \"Killed by signal in ${funcstack[2]} after ${SECONDS}s\";\n\tzle -R\n\treturn 131",
    ),
];

/// sh:161-172 — the interrupt/quit handlers the completer installs
/// around the whole completer chain:
///
/// ```text
/// # We assume localtraps to be in effect here ...
/// integer SECONDS=0
/// TRAPINT() {
///   zle -M "Killed by signal in ${funcstack[2]} after ${SECONDS}s";
///   zle -R
///   return 130
/// }
/// TRAPQUIT() { ... return 131 }
/// ```
///
/// Defining a `TRAP<SIG>` shell function IS how the trap gets installed:
/// `setfunction` (c:Src/Modules/parameter.c:305-313) recognises the
/// `TRAP` prefix and calls `settrap(sn, NULL, ZSIG_FUNC)`, so one call
/// both publishes the name into `${(k)functions}` and arms the signal.
/// Without them a ^C landing inside a slow completer unwound through the
/// shell's default SIGINT path instead of returning 130 from the widget
/// with a `zle -M` notice on screen.
///
/// The comment at sh:161 ("We assume localtraps to be in effect here")
/// refers to `setopt localtraps` in `_comp_setup` (compinit sh:182):
/// both the trap and the function definition are undone when
/// `_main_complete` returns, so the names must NOT outlive the widget.
/// `CompSetupGuard` covers the option half; this guard covers the
/// function half, and — like localtraps — restores any same-named
/// handler the caller had rather than blindly deleting.
///
/// The `${SECONDS}s` both bodies interpolate is the local shadow
/// `declare_local_seconds` installs for sh:162 — elapsed seconds inside
/// this completion, not the caller's uptime counter.
struct CompTrapGuard {
    /// Per name, the handler body that was installed before — `None`
    /// when the caller had no such function.
    saved: Vec<(&'static str, Option<String>)>,
}

impl CompTrapGuard {
    /// sh:163-172 — install both handlers, remembering what they replaced.
    fn install() -> Self {
        let mut saved = Vec::with_capacity(COMP_TRAPS.len());
        for (name, body) in COMP_TRAPS {
            // The caller's handler is restored by BODY, not by node: the
            // trap arming lives in `settrap`, which only `setfunction`
            // (c:Src/Modules/parameter.c:305-313) performs — re-adding a
            // raw `shfunctab` node would leave the signal disarmed.
            let prev = crate::ported::hashtable::shfunctab_lock()
                .read()
                .ok()
                .and_then(|t| t.get(name).and_then(|f| f.body.clone()));
            crate::ported::modules::parameter::setfunction(name, body.to_string(), 0);
            saved.push((name, prev));
        }
        CompTrapGuard { saved }
    }
}

impl Drop for CompTrapGuard {
    /// `localtraps` (compinit sh:182) — restore the caller's handlers on
    /// every exit path, including the early `return`s in the chain loop.
    /// `removeshfuncnode` also `removetrap`s the signal (hashtable.rs
    /// c:Src/hashtable.c removeshfuncnode), so a completion never leaves
    /// SIGINT pointing at a function that is no longer defined.
    fn drop(&mut self) {
        for (name, prev) in self.saved.drain(..) {
            crate::ported::hashtable::removeshfuncnode(name);
            if let Some(body) = prev {
                crate::ported::modules::parameter::setfunction(name, body, 0);
            }
        }
    }
}

/// sh:41-43 — "Hide any `_comp_priv_prefix` variable that happens to be
/// defined in the calling scope": `local _comp_priv_prefix` followed by
/// `unset _comp_priv_prefix`.
///
/// Both halves are load-bearing. The bare `local` shadows a caller's
/// value but also CREATES the parameter, so `$+_comp_priv_prefix` reads
/// 1 until the `unset` removes it; the local scope survives the unset,
/// which is why upstream writes it as two lines. The port had only the
/// `local`, which left `$+_comp_priv_prefix` == 1 for every completion
/// and flipped every completer that tests it into its "running under
/// sudo" branch: `_chown:78`'s guard
/// `(( EGID && $+commands[groups] && ! $+_comp_priv_prefix ))` went
/// false, so `chown root:<TAB>` skipped `compadd -- $(groups)` (16
/// names, what zsh lists) and fell through to `_groups` (165 names from
/// dscacheutil) — over LISTMAX, hence "do you wish to see all 165
/// possibilities (55 lines)?" where zsh prints the list. `_chflags:6`
/// and `_file_flags:5` carry the same guard.
fn hide_comp_priv_prefix() {
    use crate::compsys::ported::shared::declare_locals;
    declare_locals(&["_comp_priv_prefix"], 0); // sh:42
    unsetparam("_comp_priv_prefix"); // sh:43
}

/// sh:52 — `local -ar builtin_precommands=(- builtin eval exec
/// nocorrect noglob time)`.
///
/// The declaration and the VALUE are one statement upstream; the port
/// split them (declare in the `local` block, assign here) because
/// `declare_locals` cannot stamp `PM_READONLY` before the write. Both
/// halves are observable:
///
///   * the list itself — `_command_names:28` and `_pick_variant:15`
///     branch on `(( ${#precommands:|builtin_precommands} ))`, i.e.
///     "is any active precommand NOT one of these six". With the array
///     left empty every precommand looked non-builtin, so `sudo <TAB>`
///     took the external-only branch.
///   * the `-r` bit — `${(t)builtin_precommands}` must read
///     `array-local-readonly`, which is what the `_parameters`
///     `~*local*` filter and `typeset -p` both report.
fn seed_builtin_precommands() {
    setaparam(
        "builtin_precommands",
        [
            "-",
            "builtin",
            "eval",
            "exec",
            "nocorrect",
            "noglob",
            "time",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
    );
    crate::compsys::ported::shared::mark_readonly(&["builtin_precommands"]);
}

/// sh:162 — `integer SECONDS=0`.
///
/// `$SECONDS` is a `PM_SPECIAL` backed by a live timer. `integer` inside
/// a function creates a LOCAL shadow of it (`Src/builtin.c:2469-2575`
/// takes the same PM_LOCAL branch for specials), so the frozen zero is
/// visible only for the duration of the completion and `endparamscope`
/// hands the caller's live `$SECONDS` back. That is exactly the
/// semantics upstream relies on for the "Killed by signal in … after
/// ${SECONDS}s" trap text at sh:164/169.
///
/// The port previously skipped this line to avoid freezing the user's
/// `$SECONDS`; that reasoning was wrong (the shadow is scoped), and the
/// cost was visible: `${(t)SECONDS}` read `integer-special` instead of
/// `integer-local-special`, so `_parameters`' `[(R)…~*local*]` filter
/// kept `SECONDS` and `unset <TAB>` offered a name zsh does not.
fn declare_local_seconds() {
    use crate::compsys::ported::shared::{declare_locals, PM_INTEGER};
    declare_locals(&["SECONDS"], PM_INTEGER);
    let _ = crate::ported::params::setiparam("SECONDS", 0);
}

/// sh:115-124 — `_def_menu_style=( "$_last_menu_style[@]" )` followed by
/// `_last_menu_style=()`.
///
/// `_setup default` (sh:114) stashes the `menu` style it resolved into
/// `_last_menu_style`; these two lines move that value into
/// `_def_menu_style` and clear the staging array so each completer's own
/// `_setup` call starts empty. The menu decision at sh:241 appends
/// `_def_menu_style` to `_menu_style` — with the move never ported,
/// `$_def_menu_style` was permanently empty and the context-default
/// `menu` style was dropped from every decision.
///
/// It also settles the type: both names read `array-local` in zsh, where
/// the port left them at the `scalar-local` its `local` line created.
fn move_menu_style_to_default() {
    let last = getaparam("_last_menu_style").unwrap_or_default();
    setaparam("_def_menu_style", last); // sh:115
    setaparam("_last_menu_style", Vec::new()); // sh:124
}

/// sh:176-180 — `funcs=( "$compprefuncs[@]" ); compprefuncs=(); for func
/// in "$funcs[@]"; do "$func"; done`.
///
/// The pre-functions half of the `compprefuncs`/`comppostfuncs` pair.
/// The port had only the post half (sh:405), so a function registered on
/// `compprefuncs` — the documented hook for "run once before the next
/// completion", used by `_complete_debug`, `_correct_word` and by user
/// widgets — never ran and never got cleared, leaving the array to grow
/// across completions.
fn run_compprefuncs() {
    let funcs = getaparam("compprefuncs").unwrap_or_default(); // sh:176
    setaparam("funcs", funcs.clone());
    setaparam("compprefuncs", Vec::new()); // sh:177
    for f in &funcs {
        // sh:178-180 — `for func in "$funcs[@]"; do "$func"; done`.
        let _ = setsparam("func", f);
        let _ = crate::ported::exec::dispatch_function_call(f, &[]);
    }
}

/// `_main_complete` — primary completion-dispatch entry. Args
/// (when non-empty) override the configured `completer` style with
/// the supplied chain.
pub fn _main_complete(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_main_complete");
    tracing::debug!(target: "compsys_args", ?args, "_main_complete ENTER");
    // sh:11 `local IFS=$' \t\n\0'`, sh:27-52 the big `local` block,
    // sh:54 `typeset -U _lastdescr _comp_ignore _comp_colors`,
    // sh:158 `integer SECONDS=0`, plus the `local -A
    // _comp_caller_options` / `local -a reply` / `local REPLY` /
    // `local REPORTTIME` that `eval "$_comp_setup"` (sh:25,
    // compinit sh:180-190) contributes.
    //
    // Every name below is scratch space for the completer chain. The
    // upstream function scopes them; this port used to create them at
    // level 0 with `setsparam`, so they leaked out of the completion
    // AND read back as `scalar` instead of `scalar-local`. Both
    // matter: `_parameters` filters on `~*local*`, so the leaked
    // names showed up as candidates for `unset <TAB>`.
    {
        use crate::compsys::ported::shared::{
            declare_locals, declare_locals_keeping_value, PM_ARRAY, PM_HASHED, PM_UNIQUE,
        };
        // sh:11 + compinit sh:183 — `local IFS=$' \t\n\0'`. Declared
        // before CompSetupGuard so the guard's write lands in the
        // shadow and the caller's $IFS is restored by endparamscope.
        declare_locals(&["IFS"], 0);
        // compinit sh:180,187-190 — the `_comp_setup` locals.
        declare_locals(&["_comp_caller_options"], PM_HASHED);
        declare_locals(&["REPLY", "REPORTTIME"], 0);
        // compinit sh:189-190 — `local REPORTTIME;` is immediately
        // followed by `unset REPORTTIME`, so the name is scoped away
        // from the caller AND left PM_UNSET for the whole completion.
        // `scanpmparameters` (Src/Modules/parameter.c:138-139) skips
        // PM_UNSET nodes, so zsh's `$parameters` does not contain
        // REPORTTIME during completion. The port declared the local but
        // dropped the `unset`, leaving a set `scalar-local` behind and
        // one extra `$parameters` key versus zsh.
        crate::ported::params::unsetparam("REPORTTIME"); // compinit sh:190
        declare_locals(&["reply"], PM_ARRAY);
        // sh:27-40 — `local func funcs ret=1 tmp _compskip format nm
        // call match min max i num _completers _completer
        // _completer_num curtag _comp_force_list _matchers _matcher
        // _c_matcher _matcher_num _comp_tags _comp_mesg mesg str
        // context state state_descr line opt_args val_args
        // curcontext=… _last_nmatches=-1 _last_menu_style
        // _def_menu_style _menu_style sel _tags_level=0 _saved_exact=…
        // _saved_lastprompt=… _saved_list=… _saved_insert=…
        // _saved_colors=… _saved_colors_set=… _ambiguous_color=''`.
        declare_locals(
            &[
                "func",
                "funcs",
                "ret",
                "tmp",
                "_compskip",
                "format",
                "nm",
                "call",
                "match",
                "min",
                "max",
                "i",
                "num",
                "_completers",
                "_completer",
                "_completer_num",
                "curtag",
                "_comp_force_list",
                "_matchers",
                "_matcher",
                "_c_matcher",
                "_matcher_num",
                "_comp_tags",
                "_comp_mesg",
                "mesg",
                "str",
                "context",
                "state",
                "state_descr",
                "line",
                "opt_args",
                "val_args",
                "_last_nmatches",
                "_last_menu_style",
                "_def_menu_style",
                "_menu_style",
                "sel",
                "_tags_level",
                "_saved_exact",
                "_saved_lastprompt",
                "_saved_list",
                "_saved_insert",
                "_saved_colors",
                "_saved_colors_set",
                "_ambiguous_color",
            ],
            0,
        );
        // sh:42 `local _comp_priv_prefix`, sh:46 `local -a
        // precommands`, sh:52 `local -ar builtin_precommands`. The
        // `-r` half of sh:52 is applied by `seed_builtin_precommands`
        // below, after the list is assigned — see `mark_readonly`.
        hide_comp_priv_prefix();
        declare_locals(&["precommands", "builtin_precommands"], PM_ARRAY);
        // sh:46 `local -a precommands` — an array of length ZERO:
        //
        //     % zsh -f -c 'f(){ local -a p; print ${#p} ${(t)p} }; f'
        //     0 array-local
        //
        // `declare_locals` reaches `createparam(name, PM_ARRAY|PM_LOCAL)`,
        // which stamps the type on the node but leaves the value slot empty
        // (`params.rs` `u_arr: None`); a read through `"${(@P)precommands}"`
        // then falls back to the scalar view and produces ONE EMPTY WORD.
        // The stock-utility sweep read `precommands[1] = ''` after `_normal`
        // where zsh reads `precommands[0] =`, and the count is load-bearing:
        // `_command_names` sh:28 branches on
        // `(( ${#precommands:|builtin_precommands} ))` to decide whether to
        // offer only hashed commands.
        //
        // Materialised HERE rather than inside `declare_locals`, because
        // doing it for every PM_ARRAY declaration changed the return value
        // of `_options` and `_path_commands` (0 where they answer 1 today) —
        // an effect this fix has not accounted for, and not one to take
        // blind. sh:46 is the one declaration with a measured divergence.
        // `builtin_precommands` needs no seed: `seed_builtin_precommands`
        // assigns its sh:52 value on the next line.
        setaparam("precommands", Vec::new());
        seed_builtin_precommands();
        // sh:31 — `curcontext="$curcontext"`: local, but seeded from
        // the enclosing scope (a widget may have set it).
        declare_locals_keeping_value(&["curcontext"]);
        // sh:54 — `typeset -U _lastdescr _comp_ignore _comp_colors`.
        // `typeset` inside a function is local unless `-g`.
        declare_locals(&["_lastdescr", "_comp_ignore", "_comp_colors"], PM_UNIQUE);
        // sh:162 `integer SECONDS=0` — see `declare_local_seconds`.
        declare_local_seconds();
    }
    // Merge any finished background compinit scan BEFORE the completer
    // lookup. The bg worker ships _comps/_services/_patcomps back over a
    // channel, but the lazy drain's only other caller was the --doctor
    // benchmark — in a live shell $_comps stayed EMPTY, `_dispatch`
    // resolved comp="" for every command, and everything fell to
    // -default- file completion (option completion dead shell-wide).
    crate::fusevm_bridge::drain_compinit_bg_hook();
    // sh:25 — `_comp_caller_options=(${(kv)options[@]})` from the `_comp_setup`
    // eval, captured HERE, BEFORE CompSetupGuard flips options to the compsys
    // set — so it reflects the USER's option preferences, not compsys's. Every
    // completer that consults the caller's options reads this assoc: `_setopt`/
    // `_unsetopt` (`${(k)_comp_caller_options[(R)on]}`), `_files`/`_path_files`/
    // `_expand`/`_have_glob_qual` (`[[ $_comp_caller_options[extendedglob] == on ]]`).
    // The CompSetupGuard applies only the setopt half of _comp_setup, so this
    // assoc was never populated: `setopt <tab>` produced ZERO matches and the
    // extendedglob-gated `_path_files` branches were dead. zsh makes it `local
    // -A`; the global here is re-snapshotted every completion, so no staleness.
    {
        // sh:25's `${(kv)options[@]}` is a real read of the `options` magic
        // assoc, so it resolves that PM_AUTOLOAD stub
        // (c:Src/params.c:589-594 getparamnode → c:563-585 loadparamnode).
        // Rebuilding the value from the internal option table skips the read,
        // which left `options` typed "undefined"
        // (c:Src/Modules/parameter.c:48-50) and made `_parameters` offer it as
        // a candidate zsh does not offer.
        crate::vm_helper::mark_module_param_used("options");
        use crate::ported::options::{opt_state_get, ZSH_OPTIONS_SET};
        let mut kv: Vec<String> = Vec::with_capacity(ZSH_OPTIONS_SET.len() * 2);
        for name in ZSH_OPTIONS_SET.iter() {
            kv.push(name.to_string());
            kv.push(
                if opt_state_get(name).unwrap_or(false) {
                    "on"
                } else {
                    "off"
                }
                .to_string(),
            );
        }
        let _ = crate::ported::params::sethparam("_comp_caller_options", kv);
    }
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
        tracing::debug!(target: "compsys_args", pending, %insert_tab, "_main_complete EARLY RETURN: pending tab");
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
            tracing::debug!(target: "compsys_args", %cur_insert, %insert_tab, "_main_complete EARLY RETURN: insert-tab");
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

    // sh:93-110  Special completion contexts after `~' and `='.
    //
    //     if [[ -z "$compstate[quote]" ]]; then
    //       if [[ -o equals ]] && compset -P 1 '='; then
    //         compstate[context]=equal
    //       elif [[ "$PREFIX" != */* && "$PREFIX[1]" = '~' ]]; then
    //         if [[ "$PREFIX" = '~['[^\]]# ]]; then
    //           compset -p 2
    //           compset -S '\]*'
    //           compstate[context]=subscript
    //           [[ -n $_comps[-subscript-] ]] && $_comps[-subscript-] && return
    //         else
    //           compset -p 1
    //           compstate[context]=tilde
    //         fi
    //       fi
    //     fi
    let quote = get_compstate_str("quote").unwrap_or_default();
    if quote.is_empty() {
        let prefix = getsparam("PREFIX").unwrap_or_default();
        // Instrumentation for the sh:96 guard. It is a byte test on the LIVE
        // `$PREFIX`, so it fails silently if the published value carries
        // anything ahead of the `~` that the shell's own `${(qq)PREFIX}`
        // rendering hides (a quote/Meta marker, a stray backslash from a
        // requote upstream). `?prefix` prints the value ESCAPED, which is the
        // only way to tell "`~ro`" from "`\~ro`" or "`\x84~ro`" in the log.
        tracing::debug!(
            target: "compsys_args",
            ?prefix,
            ?quote,
            equals = isset(EQUALSOPT),
            "_main_complete special-context test (sh:93-109)"
        );
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
        } else if prefix.starts_with('~') && !prefix.contains('/') {
            // sh:96 — BOTH halves of this guard gate the `~[` arm too: it is
            // the `elif`'s body in the shell source, not a sibling branch.
            // The port had the `~[` test as its own `else if` ahead of this
            // one, so `~[/usr/lo` — which zsh leaves alone, `$PREFIX`
            // containing a `/` — took the subscript path and got its first
            // two characters eaten by `compset -p 2`.
            if prefix.starts_with("~[") && !prefix[2..].contains(']') {
                // sh:97 — `[[ "$PREFIX" = '~['[^\]]# ]]`: `~[` followed by a
                // run of NON-`]` characters and nothing else, i.e. the
                // subscript is still OPEN. `starts_with("~[")` alone also
                // matched the CLOSED `~[foo]bar`, where zsh takes the tilde
                // arm instead.
                //
                // sh:99  Inside ~[...] → subscript context.
                let _ = bin_compset(
                    "compset",
                    &["-p".to_string(), "2".to_string()],
                    &make_ops(),
                    0,
                );
                // sh:102 — ignore everything from the `]` on.
                let _ = bin_compset(
                    "compset",
                    &["-S".to_string(), "\\]*".to_string()],
                    &make_ops(),
                    0,
                );
                set_compstate_str("context", "subscript");
                // sh:104 — `[[ -n $_comps[-subscript-] ]] &&
                //            $_comps[-subscript-] && return`. Dispatch the
                // registered `-subscript-` completer directly and, when it
                // succeeds, return WITHOUT running the completer chain. This
                // line was missing, so `~[<TAB>` fell through to the chain and
                // reached `_subscript` only via `_complete`'s context lookup —
                // which re-derives the context and applies the whole
                // completer/tag machinery zsh deliberately skips here.
                let sub = comps_entry("-subscript-");
                if !sub.is_empty() && dispatch_function_call(&sub, &[]) == Some(0) {
                    return 0;
                }
            } else {
                // sh:106  ~user
                let _ = bin_compset(
                    "compset",
                    &["-p".to_string(), "1".to_string()],
                    &make_ops(),
                    0,
                );
                set_compstate_str("context", "tilde");
            }
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
    // sh:54 — `typeset -U _lastdescr _comp_ignore _comp_colors`. The
    // PM_UNIQUE bit is what makes every later `+=` append (`_setup` per
    // completer, `_path_files`/`_files` ignore accumulation, the
    // ambiguous-color injection) dedupe; zshrs's `setaparam` honors it
    // (params.rs:7367/4627 → arrunique). `declare_locals` above already
    // stamps it inside a function scope; this re-stamp covers the
    // `locallevel == 0` callers (unit tests, `--doctor`) where
    // `declare_locals` is a no-op by construction.
    //
    // The three names are deliberately NOT pre-assigned to empty arrays
    // here. `typeset -U` without `-a` declares SCALARS, so zsh reports
    // `${(t)_lastdescr}` as `scalar-local-unique` at this point and only
    // converts on the first array assignment (`_description`'s
    // `_lastdescr=(…)`). Seeding empty arrays made the port report
    // `array-local-unique` for the whole completion — a `${(t)}`
    // divergence on three names, with no upstream statement behind it.
    // The per-completion reset the seeding provided now comes from the
    // `declare_locals` shadow, which starts empty and is unwound by
    // `endparamscope`.
    {
        let mut tab = crate::ported::params::paramtab().write().unwrap();
        for nm in ["_lastdescr", "_comp_ignore", "_comp_colors"] {
            if let Some(pm) = tab.get_mut(nm) {
                pm.node.flags |= crate::ported::zsh_h::PM_UNIQUE as i32;
            }
        }
    }

    // sh:114-124 — `_setup default` has already run (above); move the
    // `menu` style it staged into `_def_menu_style` and clear the stage.
    move_menu_style_to_default();

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

    // sh:161-172 — "We assume localtraps to be in effect here": install
    // TRAPINT / TRAPQUIT for the duration of the completer chain. Dropped
    // (and the caller's handlers restored) on every exit path below.
    let _comp_traps = CompTrapGuard::install();

    // sh:174-180 — "Call the pre-functions."
    run_compprefuncs();

    let mut ret: i32 = 1;
    let mut completer_num: i64 = 1;
    for completer_spec in &chain {
        // c:Src/exec.c `execlist` — `if (errflag) break;` before each element
        // of a list. In C `_main_complete` is a SHELL function, so an error
        // raised deep inside a completer (`zerr` sets `errflag |=
        // ERRFLAG_ERROR`, c:Src/utils.c:176/194) stops every remaining
        // statement in it, this `for` loop included, and `do_completion`
        // then sees `(nmatches || nmessages) && !errflag`
        // (c:Src/Zle/compcore.c:1031). The completer chain here is a NATIVE
        // port, so nothing unwinds into it and each later completer
        // (`_approximate`, `_ignored`, …) re-ran the whole failing pass —
        // `CC <TAB>`, whose `_CC` glob `*(-/):t:directories` is a genuine
        // bad pattern, spun instead of stopping with the one match zsh
        // shows.
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            break;
        }
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

        // sh:200-201 — `zstyle -a ":completion:${curcontext}:" matcher-list
        // _matchers || _matchers=( '' )`, then sh:205 iterates `$_matchers`.
        //
        // Upstream keeps the list in the `_matchers` PARAMETER, not in a
        // shell-local temporary, and downstream code reads it back:
        // `_path_files:1620` sizes its per-matcher work off
        // `${#_matchers}`, and `${(t)_matchers}` must read `array-local`.
        // The port kept the list only in this Rust `Vec`, so `$_matchers`
        // stayed the empty scalar its `local` line created.
        let matchers = lookupstyle(&format!(":completion:{}:", curcontext), "matcher-list");
        let matcher_list: Vec<String> = if matchers.is_empty() {
            vec!["".to_string()]
        } else {
            matchers
        };
        setaparam("_matchers", matcher_list.clone());
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

            // sh:212 — `_comp_mesg=` clears the flag before every completer
            // call, so the sh:224 test below sees only what THIS completer set.
            let _ = setsparam("_comp_mesg", "");

            // sh:218 — `elif "$tmp"; then`. `$tmp` is an ordinary command
            // word, so a completer named by the `completer` style but never
            // defined is a plain command-not-found: zsh prints
            // `_main_complete:218: command not found: NAME` on stderr and the
            // call yields 127, which just moves the chain along. `None` here
            // is that case (undefined, or `disable -f`'d — C's lookupshfunc
            // returns NULL for both and falls through to PATH). The port
            // folded `None` into a silent non-zero, so a typo in the
            // `completer` style produced no diagnostic whatsoever.
            // sh:218 — publish the calling line so the completer's frame
            // records `_main_complete:218`, matching zsh's `$functrace`. The
            // diagnostic below already cites this line; the frame has to agree.
            crate::compsys::ported::shared::set_sh_lineno(218);
            match dispatch_function_call(&bare, &[]) {
                Some(0) => {
                    ret = 0;
                    break;
                }
                Some(_) => {}
                None => eprintln!("_main_complete:218: command not found: {}", bare),
            }
            matcher_num += 1;
        }
        if ret == 0 {
            break;
        }
        // sh:224 — `[[ -n "$_comp_mesg" ]] && break`. A completer that emitted
        // a message (`_message` sets `_comp_mesg=yes`, sh:8/sh:44) ends the
        // chain even though it returned non-zero: the message IS the result,
        // and later completers must not append to it. The port omitted this,
        // so `_approximate` still ran after a message-producing completer and
        // added a `corrections` group zsh never shows.
        if !getsparam("_comp_mesg").unwrap_or_default().is_empty() {
            break;
        }
        completer_num += 1;
    }

    // Flush the still-open match group at completer completion, before the
    // `$compstate[nmatches]` menu decision reads it. In C the file-scope
    // `matches` list is a pointer-alias of the open group's `lmatches`, so
    // `permmatches` always counts the live matches; the Rust port copies
    // instead, so an unflushed open group (e.g. git's `common-commands` left
    // open by `_describe`/`_arguments`, 23 matches) counts 0 here even though
    // the matches exist — the decision then skips menu-select (interactive
    // never starts for git). endcmgroup(None) flushes that group ONCE, at this
    // single post-loop point (nm: git 0→23, cd 0→16), so the count is correct.
    // It is NOT a per-read change in permmatches (which would perturb every
    // mid-completer-loop read, e.g. kill's `_tags` iterations, and short-circuit
    // kill's flow — verified: kill nm 3→3 here, unchanged). c:begcmgroup alias.
    crate::ported::zle::compcore::endcmgroup(None);
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
                || matches!(
                    e,
                    "yes=long-list" | "true=long-list" | "on=long-list" | "1=long-list"
                )
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
            // sh:252 `sel=( "${(@M)_menu_style:#(yes|true|1|on)*}" )` — a
            // real assignment to the shell-local declared at sh:32, which
            // retypes it from scalar to array. Keeping the match list only
            // in Rust left `${(t)sel}` reading `scalar-local` against zsh's
            // `array-local`.
            crate::ported::params::setaparam(
                "sel",
                sel.iter().map(|e| (*e).clone()).collect::<Vec<String>>(),
            );
            if sel.is_empty() {
                return None;
            }
            let mut m = 9999999i64;
            for i in sel {
                let num = if let Some(eq) = i.find('=') {
                    let rest = &i[eq + 1..];
                    if rest
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false)
                    {
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
            e.starts_with("yes")
                || e.starts_with("true")
                || e.starts_with("1")
                || e.starts_with("on")
        };
        let no_like = |e: &str| {
            e.starts_with("no")
                || e.starts_with("false")
                || e.starts_with("0")
                || e.starts_with("off")
        };
        let auto = has(&|e| e.starts_with("auto"));

        if list_has && tmp > lines && long_list {
            set_compstate_str("insert", "menu"); // sh:246
        } else if cur_insert == saved_insert {
            // sh:247
            let long = has(&|e| matches!(e, "yes=long" | "true=long" | "1=long" | "on=long"));
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
        if get_compstate_str("insert")
            .unwrap_or_default()
            .contains("menu")
        {
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
            let m = crate::comp_match_handles::matches_arc()
                .lock()
                .map(|g| g.len())
                .unwrap_or(0);
            let fm = crate::comp_match_handles::fmatches_arc()
                .lock()
                .map(|g| g.len())
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
            // sh:360 — `tmp=( "\`${(@)^_lastdescr:#}'" )`. The `:#` with an
            // EMPTY pattern drops every element that matches it, i.e. every
            // empty element. That matters because `_description` sh:14
            // (`_lastdescr=( "$_lastdescr[@]" "$3" )`) runs while
            // `_lastdescr` is still the SCALAR sh:54's `typeset -U` declared,
            // so the array it converts to always leads with one empty word.
            // sh:354's `$#_lastdescr -ne 0` above counts that word; these two
            // consumers must not, or the warning reads
            // "`' or `file'" instead of "`file'".
            let nonempty: Vec<&String> = lastdescr.iter().filter(|d| !d.is_empty()).collect();
            let quoted: Vec<String> = nonempty.iter().map(|d| format!("`{}'", d)).collect();
            // sh:362-366 — `case $#tmp in 1) … 2) … *) …`.
            let str_msg = match quoted.len() {
                0 => String::new(),
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
                // sh:369 — `"D:${(F)${(@)_lastdescr:#}}"`: same empty-element
                // strip as sh:360, then joined with newlines by `(F)`.
                format!(
                    "D:{}",
                    nonempty
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
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

    // sh:407  `_lastcomp=( "${(@kv)compstate}" )` — the snapshot STARTS as a
    // full key/value copy of `$compstate`, which is where `_lastcomp[insert]`
    // (`_oldlist` sh:22), `_lastcomp[unambiguous]` (sh:84, `_next_tags` sh:105)
    // and `_lastcomp[unambiguous_cursor]` (sh:85-86) come from. sh:408-416 then
    // OVERRIDE nine keys. The port hand-built a 12-pair list instead, so every
    // compstate key it did not happen to name (`list`, `to_end`, `old_list`,
    // `last_prompt`, `exact`, `context`, …) was simply absent.
    //
    // `compinit` sh:126 declares `typeset -gHA _lastcomp`: it is an
    // ASSOCIATION, and sh:407's flat pair list is assigned INTO that assoc.
    // The port stored it with `setaparam`, which retypes the parameter to a
    // plain indexed array — `${(t)_lastcomp}` read `array-hideval` against
    // zsh's `association-hideval`, so every shell-level `$_lastcomp[key]`
    // subscript (a string subscript on an array) evaluated to the empty
    // string. `_oldlist` sh:3/21/22/38, `_next_tags` sh:90/91/95/105 and any
    // user or plugin code reading the snapshot all saw nothing. `sethparam`
    // (`Src/params.c:3602`) is the assoc-preserving store.
    let mut lastcomp: Vec<String> = Vec::new();
    {
        // sh:407 — `${(@kv)compstate}`: keys and values interleaved.
        //
        // C reaches that through ONE `paramvalarr(pm->gsu.h->getfn(pm),
        // SCANPM_WANTKEYS|SCANPM_WANTVALS)` (`Src/params.c:689-700`): the hash
        // getfn runs once and the single scan emits each node's key and then
        // its value, so every gsu-backed element getter fires exactly once.
        // (C's `gethkparam`, c:params.c:3131-3140, is the WANTKEYS-only form,
        // and `scanparamvals` c:params.c:4064-4079 returns right after the key
        // without ever calling the value getter.)
        //
        // Asking zshrs for keys and values separately runs the getfn phase
        // twice instead, and `$compstate`'s gsu-backed keys are not cheap
        // reads: `list_lines` is a full `calclist()` over every match
        // (`complete.c:1408-1420` → `compresult.c:1446-1459`) and `nmatches`
        // flushes with `permmatches()` (`complete.c:1401-1405`). On a 47k-match
        // listing that second scan was ~20% of the whole pre-paint phase, all
        // of it recomputing values this scan already has.
        //
        // `gethparam` leaves the freshly computed table in the hashed storage,
        // so the keys come out of that same single scan.
        let vals = gethparam("compstate").unwrap_or_default(); // c:params.c:3117-3125
        let keys: Vec<String> = crate::ported::params::paramtab_hashed_storage()
            .lock()
            .ok()
            .and_then(|t| t.get("compstate").map(|h| h.keys().cloned().collect()))
            .unwrap_or_default();
        for (k, v) in keys.iter().zip(vals.iter()) {
            lastcomp.push(k.clone());
            lastcomp.push(v.clone());
        }
    }
    // sh:408-416 — the nine overrides, appended after the copy. `sethparam`
    // consumes the flat list left-to-right, so a later pair wins.
    for (k, v) in [
        ("nmatches", nm.to_string()),                               // sh:408
        ("completer", getsparam("_completer").unwrap_or_default()), // sh:409
        ("prefix", getsparam("PREFIX").unwrap_or_default()),        // sh:410
        ("suffix", getsparam("SUFFIX").unwrap_or_default()),        // sh:411
        ("iprefix", getsparam("IPREFIX").unwrap_or_default()),      // sh:412
        ("isuffix", getsparam("ISUFFIX").unwrap_or_default()),      // sh:413
        ("qiprefix", getsparam("QIPREFIX").unwrap_or_default()),    // sh:414
        ("qisuffix", getsparam("QISUFFIX").unwrap_or_default()),    // sh:415
        ("tags", getsparam("_comp_tags").unwrap_or_default()),      // sh:416
    ] {
        lastcomp.push(k.to_string());
        lastcomp.push(v);
    }
    sethparam("_lastcomp", lastcomp); // sh:407-416, c:params.c:3602

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
    // `compinit` sh:126 declares `typeset -gHA _lastcomp`, so the canonical
    // storage is an ASSOCIATION. The flat-array walk below is the layout the
    // pre-`sethparam` writer left behind, kept as a fallback.
    let keys = gethkparam("_lastcomp").unwrap_or_default();
    if !keys.is_empty() {
        let vals = gethparam("_lastcomp").unwrap_or_default();
        if let Some(i) = keys.iter().position(|k| k == key) {
            return vals.get(i).cloned();
        }
        return None;
    }
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

    /// `${(t)name}` for a live parameter — the exact string
    /// `$parameters[name]` reports, via the same
    /// `Src/Modules/parameter.c:43-95` renderer.
    fn type_of(name: &str) -> String {
        crate::ported::params::paramtab()
            .read()
            .ok()
            .and_then(|t| {
                t.get(name)
                    .map(|pm| crate::ported::modules::parameter::paramtypestr(pm))
            })
            .unwrap_or_default()
    }

    /// Run `body` one function scope deep, so `declare_locals` takes its
    /// `pm->level < locallevel` branch (`Src/builtin.c:2469`) instead of
    /// the `locallevel == 0` early return.
    /// The unwind runs even when `body` panics. An assertion failure
    /// inside the scope used to skip `endparamscope()` outright, leaving
    /// `locallevel` incremented and the half-built local shadows in
    /// `paramtab` for the REST OF THE TEST BINARY. Every later test that
    /// depends on a clean parameter scope then failed too — concretely
    /// `ported::zle::compcore::tests::callcompfunc_publishes_and_tears_
    /// down_zle_params` saw `makezleparams`/`endparamscope` operate at
    /// the leaked level and panicked while holding the compcore test
    /// mutex, poisoning it and taking another ~20 tests down with it.
    /// Catching the unwind here bounds the damage to the one test that
    /// actually failed; the panic is re-raised unchanged so the failure
    /// is still reported.
    fn in_function_scope<T>(body: impl FnOnce() -> T) -> T {
        crate::ported::utils::inc_locallevel();
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        crate::ported::params::endparamscope();
        match out {
            Ok(v) => v,
            Err(p) => std::panic::resume_unwind(p),
        }
    }

    /// sh:52 — `local -ar builtin_precommands=(- builtin eval exec
    /// nocorrect noglob time)`.
    ///
    /// Two regressions in one line. The VALUE was never assigned, so
    /// `_command_names:28` / `_pick_variant:15`'s
    /// `(( ${#precommands:|builtin_precommands} ))` saw an empty
    /// exclusion list; and the `-r` bit was never stamped, so
    /// `${(t)builtin_precommands}` read `array-local` where zsh reads
    /// `array-local-readonly`.
    #[test]
    fn builtin_precommands_is_the_readonly_local_array_upstream_declares() {
        let _g = crate::test_util::global_state_lock();
        in_function_scope(|| {
            crate::compsys::ported::shared::declare_locals(
                &["builtin_precommands"],
                crate::compsys::ported::shared::PM_ARRAY,
            );
            seed_builtin_precommands();
            assert_eq!(
                getaparam("builtin_precommands").unwrap_or_default(),
                vec![
                    "-",
                    "builtin",
                    "eval",
                    "exec",
                    "nocorrect",
                    "noglob",
                    "time"
                ],
                "sh:52 value missing — ${{#precommands:|builtin_precommands}} misreads"
            );
            assert_eq!(
                type_of("builtin_precommands"),
                "array-local-readonly",
                "sh:52 is `local -ar`"
            );
        });
    }

    /// sh:162 — `integer SECONDS=0`.
    ///
    /// The shadow must be LOCAL: `_parameters` drops every candidate
    /// whose type matches `*local*`, so a non-local `SECONDS` was
    /// offered by `unset <TAB>` where zsh offers nothing.
    #[test]
    fn seconds_shadow_is_integer_local_special() {
        let _g = crate::test_util::global_state_lock();
        // A bare test binary never runs `createparamtable`, so stand the
        // PM_SPECIAL timer parameter up the way `Src/params.c` does
        // before checking that the shadow inherits its `-special`.
        let _ = crate::ported::params::setiparam("SECONDS", 7);
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            if let Some(pm) = tab.get_mut("SECONDS") {
                pm.node.flags |= crate::ported::zsh_h::PM_SPECIAL as i32;
            }
        }
        let before = type_of("SECONDS");
        assert_eq!(before, "integer-special", "probe setup failed");
        in_function_scope(|| {
            declare_local_seconds();
            assert_eq!(
                type_of("SECONDS"),
                "integer-local-special",
                "sh:162 `integer SECONDS=0` not mirrored"
            );
        });
        assert_eq!(
            type_of("SECONDS"),
            before,
            "endparamscope must hand the caller's live $SECONDS back"
        );
    }

    /// sh:54 — `typeset -U _lastdescr _comp_ignore _comp_colors`.
    ///
    /// `typeset -U` without `-a` declares SCALARS; zsh reports
    /// `scalar-local-unique` until the first array assignment. Seeding
    /// them as empty arrays made the port report `array-local-unique`
    /// for the whole completion.
    #[test]
    fn typeset_u_trio_declares_unique_local_scalars() {
        let _g = crate::test_util::global_state_lock();
        in_function_scope(|| {
            crate::compsys::ported::shared::declare_locals(
                &["_lastdescr", "_comp_ignore", "_comp_colors"],
                crate::compsys::ported::shared::PM_UNIQUE,
            );
            for nm in ["_lastdescr", "_comp_ignore", "_comp_colors"] {
                assert_eq!(type_of(nm), "scalar-local-unique", "sh:54 type for {nm}");
            }
            // …and a full pass through `_main_complete` must leave them
            // that way. The regression this pins was three
            // `setaparam(nm, Vec::new())` calls in the body, which
            // converted all three to `array-local-unique` before any
            // completer ran — a `${(t)}` divergence with no upstream
            // statement behind it.
            let _ = _main_complete(&[]);
            for nm in ["_lastdescr", "_comp_ignore", "_comp_colors"] {
                assert_ne!(
                    type_of(nm),
                    "array-local-unique",
                    "sh:54 declares scalars; {nm} was seeded as an array"
                );
            }
        });
    }

    /// sh:115/124 — `_def_menu_style=( "$_last_menu_style[@]" )` then
    /// `_last_menu_style=()`. Without the move, `$_def_menu_style` stayed
    /// empty and the context-default `menu` style never reached the
    /// sh:241 decision; both names also read `scalar-local` instead of
    /// `array-local`.
    #[test]
    fn menu_style_moves_from_stage_to_default() {
        let _g = crate::test_util::global_state_lock();
        in_function_scope(|| {
            crate::compsys::ported::shared::declare_locals(
                &["_last_menu_style", "_def_menu_style"],
                0,
            );
            setaparam("_last_menu_style", vec!["select=2".to_string()]);
            move_menu_style_to_default();
            assert_eq!(
                getaparam("_def_menu_style").unwrap_or_default(),
                vec!["select=2".to_string()],
                "sh:115 move missing"
            );
            assert!(
                getaparam("_last_menu_style").unwrap_or_default().is_empty(),
                "sh:124 clear missing"
            );
            assert_eq!(type_of("_def_menu_style"), "array-local");
            assert_eq!(type_of("_last_menu_style"), "array-local");
        });
    }

    /// sh:176-180 — the `compprefuncs` half of the pre/post hook pair.
    /// The port shipped only the post half, so a registered
    /// pre-function never ran and the array was never drained.
    #[test]
    fn compprefuncs_are_consumed_and_published_as_funcs() {
        let _g = crate::test_util::global_state_lock();
        in_function_scope(|| {
            crate::compsys::ported::shared::declare_locals(&["funcs"], 0);
            setaparam(
                "compprefuncs",
                vec!["_zzz_probe_a".to_string(), "_zzz_probe_b".to_string()],
            );
            run_compprefuncs();
            assert_eq!(
                getaparam("funcs").unwrap_or_default(),
                vec!["_zzz_probe_a".to_string(), "_zzz_probe_b".to_string()],
                "sh:176 `funcs=( \"$compprefuncs[@]\" )` missing"
            );
            assert!(
                getaparam("compprefuncs").unwrap_or_default().is_empty(),
                "sh:177 `compprefuncs=()` missing"
            );
            assert_eq!(type_of("funcs"), "array-local");
        });
    }

    /// sh:41-43 — after the hide idiom, `$+_comp_priv_prefix` must read
    /// 0 even when the caller had the variable set. Dropping the `unset`
    /// half made every completer's `$+_comp_priv_prefix` test true, which
    /// is what sent `chown root:<TAB>` down `_groups` (165 matches, past
    /// LISTMAX) instead of `compadd -- $(groups)` (16, listed outright).
    #[test]
    fn hide_comp_priv_prefix_leaves_parameter_unset() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setaparam("_comp_priv_prefix", vec!["sudo".to_string()]);
        assert!(
            getaparam("_comp_priv_prefix").is_some(),
            "probe setup failed"
        );
        hide_comp_priv_prefix();
        assert!(
            getaparam("_comp_priv_prefix").is_none(),
            "sh:43 unset missing — $+_comp_priv_prefix still reads 1"
        );
    }

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

    /// sh:161-172 — the completer chain runs with `TRAPINT`/`TRAPQUIT`
    /// defined, so a ^C inside a slow completer returns 130 from the
    /// widget with a `zle -M` notice instead of unwinding through the
    /// shell's default SIGINT path. The names are observable: real zsh
    /// answers `${+functions[TRAPINT]}` == 1 from inside a completer and
    /// 0 once the widget has returned (`localtraps`, compinit sh:182).
    /// zshrs answered 0 in both places — the block was never ported.
    #[test]
    fn comp_trap_guard_installs_both_handlers_and_restores_caller() {
        use crate::ported::hashtable::{removeshfuncnode, shfunctab_lock};
        use crate::ported::modules::parameter::setfunction;
        let _g = crate::test_util::global_state_lock();

        let body_of = |n: &str| -> Option<String> {
            shfunctab_lock()
                .read()
                .ok()
                .and_then(|t| t.get(n).and_then(|f| f.body.clone()))
        };

        // A caller-installed SIGINT handler must survive the completion.
        removeshfuncnode("TRAPINT");
        removeshfuncnode("TRAPQUIT");
        setfunction("TRAPINT", "\tprint caller-int".to_string(), 0);

        {
            let _traps = CompTrapGuard::install();
            // sh:166 / sh:171 — the two bodies differ only in the status.
            assert!(
                body_of("TRAPINT")
                    .as_deref()
                    .is_some_and(|b| b.contains("return 130")),
                "sh:163-167 TRAPINT not installed for the completer chain"
            );
            assert!(
                body_of("TRAPQUIT")
                    .as_deref()
                    .is_some_and(|b| b.contains("return 131")),
                "sh:168-172 TRAPQUIT not installed for the completer chain"
            );
        }

        // localtraps — the caller's handler is back, verbatim, and the
        // name that had none is gone rather than left behind.
        assert_eq!(
            body_of("TRAPINT").as_deref(),
            Some("\tprint caller-int"),
            "the caller's TRAPINT must be restored, not clobbered"
        );
        assert!(
            body_of("TRAPQUIT").is_none(),
            "TRAPQUIT was undefined before the completion and must not leak out"
        );
        removeshfuncnode("TRAPINT");
    }

    /// Same contract at the real call site: no completion may leave the
    /// handlers behind, on any return path out of the chain loop.
    #[test]
    fn main_complete_leaves_no_trap_functions_behind() {
        use crate::ported::hashtable::{removeshfuncnode, shfunctab_lock};
        let _g = crate::test_util::global_state_lock();
        removeshfuncnode("TRAPINT");
        removeshfuncnode("TRAPQUIT");
        let _ = setsparam("curcontext", "a:b:c:d");
        let _ = _main_complete(&[]);
        let tab = shfunctab_lock();
        let tab = tab.read().unwrap();
        for n in ["TRAPINT", "TRAPQUIT"] {
            assert!(tab.get(n).is_none(), "{n} leaked out of _main_complete");
        }
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

    /// `compinit` sh:126 declares `typeset -gHA _lastcomp`, and sh:407-416
    /// assigns the snapshot INTO that association. Writing it with
    /// `setaparam` retypes the parameter to an indexed array, and a shell
    /// `$_lastcomp[key]` subscript on an array is a string-subscript miss
    /// that evaluates empty — which is what `_oldlist` sh:3/21/22 and
    /// `_next_tags` sh:90/91/95/105 read. Pin the storage TYPE, not just the
    /// values: `gethkparam` returns `None` for anything that is not
    /// `PM_HASHED`, so this fails outright if the writer regresses to
    /// `setaparam`. The duplicate `nmatches` pair mirrors sh:408 overriding
    /// the sh:407 copy — last pair must win, as C's hash insert does.
    #[test]
    fn lastcomp_snapshot_is_stored_as_an_association() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("_lastcomp");
        sethparam(
            "_lastcomp",
            vec![
                "nmatches".to_string(),
                "0".to_string(),
                "insert".to_string(),
                "automenu-unambiguous".to_string(),
                "nmatches".to_string(),
                "40".to_string(),
            ],
        );
        assert_eq!(
            gethkparam("_lastcomp").map(|k| k.len()),
            Some(2),
            "sh:407-416 snapshot must live in PM_HASHED storage"
        );
        assert_eq!(lastcomp_get("nmatches").as_deref(), Some("40"));
        assert_eq!(
            lastcomp_get("insert").as_deref(),
            Some("automenu-unambiguous")
        );
        assert!(lastcomp_get("never-set").is_none());
        unsetparam("_lastcomp");
    }
}
