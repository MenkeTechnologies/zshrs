//! `vcs_info` hook machinery for the native p10k `vcs` segment.
//!
//! Port of `Functions/VCS_Info/VCS_INFO_hook` (zsh) plus the dynamic-scope
//! contract the `+vi-*` functions rely on: while a hook runs it sees
//! `hook_com`, `vcs_comm`, `user_data`, `backend_misc`, `msgs`, `vcs`,
//! `rrn`, `usercontext`, `quiltmode`, `maxexports` and may set `ret`.
//!
//! Hook resolution (VCS_INFO_hook):
//! 1. static hooks: style `hooks` of `:vcs_info-static_hooks:<hook>`;
//! 2. context hooks: style `hooks` of `:vcs_info:<vcs>+<hook>:<usercontext>:<rrn>`,
//!    appended after the static ones;
//! 3. no hooks -> return 0; otherwise `ret=0` and each name `N` is run as
//!    `+vi-N <args>` (unknown functions are skipped); the first hook whose
//!    exit status is non-zero ends the loop; the result is `$ret`. A hook
//!    that fails to run at all (parse error, aborted function) exits
//!    non-zero like any failing function, so it ends the loop too.
//!
//! Three hook names are implemented natively by the host (the theme's own
//! `+vi-vcs-detect-changes`, `+vi-svn-detect-changes`, `+vi-hg-bookmarks`);
//! every other name is a user shell function called through the embedded
//! shell: a generated anonymous function declares the hook variables as
//! locals, calls `+vi-N`, and copies the possibly-modified variables back
//! into globals that this module reads and unsets.

use std::collections::HashMap;

// ---------------------------------------------------------------------
// Ordered associative array
// ---------------------------------------------------------------------

/// Insertion-ordered string map standing in for a zsh associative array
/// local to `vcs_info`. A missing key reads as the empty string, like
/// `${hook_com[nokey]}`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Assoc(Vec<(String, String)>);

impl Assoc {
    pub(crate) fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        let mut a = Assoc::default();
        for (k, v) in pairs {
            a.set(k, *v);
        }
        a
    }

    /// `hook_com=( k v ... )` from a flat key/value list.
    pub(crate) fn from_flat(flat: &[String]) -> Self {
        let mut a = Assoc::default();
        for kv in flat.chunks(2) {
            a.set(&kv[0], kv.get(1).cloned().unwrap_or_default());
        }
        a
    }

    pub(crate) fn get(&self, key: &str) -> &str {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }

    pub(crate) fn has(&self, key: &str) -> bool {
        self.0.iter().any(|(k, _)| k == key)
    }

    /// `assoc[key]=value` — overwriting keeps the key's position.
    pub(crate) fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value,
            None => self.0.push((key.to_string(), value)),
        }
    }

    /// `assoc[key]+=suffix`.
    pub(crate) fn append(&mut self, key: &str, suffix: &str) {
        let joined = format!("{}{suffix}", self.get(key));
        self.set(key, joined);
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    pub(crate) fn pairs(&self) -> &[(String, String)] {
        &self.0
    }
}

// ---------------------------------------------------------------------
// Hook state
// ---------------------------------------------------------------------

/// The scalar variables of `vcs_info` that contexts are built from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Vars {
    pub vcs: String,
    pub usercontext: String,
    pub rrn: String,
    pub quiltmode: String,
    pub maxexports: usize,
}

impl Vars {
    /// The initial values of `vcs_info` (`vcs='-init-'; rrn='-all-';
    /// quiltmode='addon'`).
    pub(crate) fn initial() -> Self {
        Vars {
            vcs: "-init-".to_string(),
            usercontext: "default".to_string(),
            rrn: "-all-".to_string(),
            quiltmode: "addon".to_string(),
            maxexports: 2,
        }
    }
}

/// Globals the theme's hooks communicate through (`prompt_vcs`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Flags {
    /// `VCS_WORKDIR_DIRTY`
    pub dirty: bool,
    /// `VCS_WORKDIR_HALF_DIRTY`
    pub half_dirty: bool,
    /// `vcs_visual_identifier` — an icon KEY such as `VCS_HG_ICON`.
    pub visual_identifier: String,
}

/// Everything a hook can read or write.
#[derive(Clone, Debug)]
pub(crate) struct HookState {
    pub vars: Vars,
    pub hook_com: Assoc,
    pub vcs_comm: Assoc,
    pub user_data: Assoc,
    pub backend_misc: Assoc,
    pub msgs: Vec<String>,
    /// Backend-local arrays hooks may read (`hgbmarks`, `mqpatches`, ...),
    /// by variable name.
    pub arrays: Vec<(String, Vec<String>)>,
    pub flags: Flags,
    /// `ret` of the running `VCS_INFO_hook`.
    pub ret: i32,
}

impl HookState {
    pub(crate) fn new() -> Self {
        HookState {
            vars: Vars::initial(),
            hook_com: Assoc::default(),
            vcs_comm: Assoc::default(),
            user_data: Assoc::default(),
            backend_misc: Assoc::default(),
            msgs: Vec::new(),
            arrays: Vec::new(),
            flags: Flags::default(),
            ret: 0,
        }
    }

    pub(crate) fn array(&self, name: &str) -> &[String] {
        self.arrays
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
    }

    pub(crate) fn set_array(&mut self, name: &str, value: Vec<String>) {
        match self.arrays.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = value,
            None => self.arrays.push((name.to_string(), value)),
        }
    }
}

// ---------------------------------------------------------------------
// Host (style lookup + hook dispatch)
// ---------------------------------------------------------------------

/// What the hook runner needs from its environment. The prompt
/// implementation reads the zstyle table and calls the shell; tests
/// substitute canned values.
pub(crate) trait HookHost {
    /// `zstyle -a <ctx> <style>` — the style's values, empty when unset.
    fn style(&self, ctx: &str, style: &str) -> Vec<String>;

    /// A hook implemented natively. `None`: not a native hook name.
    fn native_hook(&self, name: &str, args: &[String], st: &mut HookState) -> Option<i32>;

    /// `(( $+functions[func] ))`.
    fn user_function_exists(&self, func: &str) -> bool;

    /// Call the user function; the return value is its exit status.
    fn call_user_function(&self, func: &str, args: &[String], st: &mut HookState) -> i32;

    /// The type and value of the shell parameter `name` (`${(Pt)name}`,
    /// `${(P)name}`, `${(kvP)name}`), as `VCS_INFO_quilt-standalone-detect`
    /// reads it: an array parameter is made unique in place on the way.
    fn param(&self, _name: &str) -> ParamValue {
        ParamValue::Unset
    }

    /// `zstyle -s` — the values joined with a space; `None` when unset.
    fn style_s(&self, ctx: &str, style: &str) -> Option<String> {
        let v = self.style(ctx, style);
        (!v.is_empty()).then(|| v.join(" "))
    }

    /// `zstyle -t` — true only when set and the first value is a true word.
    fn style_t(&self, ctx: &str, style: &str) -> bool {
        style_is_true(&self.style(ctx, style))
    }

    /// `zstyle -T` — like `-t` but an unset style counts as true.
    fn style_tt(&self, ctx: &str, style: &str) -> bool {
        let v = self.style(ctx, style);
        v.is_empty() || style_is_true(&v)
    }
}

/// A shell parameter as `${(Pt)name}` reports it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ParamValue {
    Unset,
    Scalar(String),
    Array(Vec<String>),
    /// Key/value pairs in `${(kv)name}` order.
    Assoc(Vec<(String, String)>),
}

/// zutil.c `zstyle -t`: `yes`, `true`, `1`, `on` as the first value.
pub(crate) fn style_is_true(vals: &[String]) -> bool {
    matches!(vals.first().map(String::as_str), Some("yes" | "true" | "1" | "on"))
}

/// `:vcs_info:<vcs>:<usercontext>:<rrn>` — the context for plain styles.
pub(crate) fn style_context(v: &Vars) -> String {
    format!(":vcs_info:{}:{}:{}", v.vcs, v.usercontext, v.rrn)
}

/// `:vcs_info:<vcs>+<hook>:<usercontext>:<rrn>`.
pub(crate) fn hook_context(v: &Vars, hook_name: &str) -> String {
    format!(":vcs_info:{}+{hook_name}:{}:{}", v.vcs, v.usercontext, v.rrn)
}

/// `:vcs_info-static_hooks:<hook>`.
pub(crate) fn static_context(hook_name: &str) -> String {
    format!(":vcs_info-static_hooks:{hook_name}")
}

/// The ordered hook names for `hook_name`: static hooks first, context
/// hooks after them.
pub(crate) fn resolve_hooks(host: &dyn HookHost, v: &Vars, hook_name: &str) -> Vec<String> {
    let mut hooks = host.style(&static_context(hook_name), "hooks");
    hooks.extend(host.style(&hook_context(v, hook_name), "hooks"));
    hooks
}

/// `VCS_INFO_hook <hook_name> <args...>`; returns `$ret`.
pub(crate) fn run_hook(
    host: &dyn HookHost,
    st: &mut HookState,
    hook_name: &str,
    args: &[String],
) -> i32 {
    let hooks = resolve_hooks(host, &st.vars, hook_name);
    if hooks.is_empty() {
        return 0;
    }
    st.ret = 0;
    for hook in hooks {
        let func = format!("+vi-{hook}");
        let status = if let Some(rc) = host.native_hook(&hook, args, st) {
            rc
        } else if host.user_function_exists(&func) {
            host.call_user_function(&func, args, st)
        } else {
            tracing::debug!(target: "p10k", hook = %func, "vcs_info hook function not defined");
            continue;
        };
        if status != 0 {
            break;
        }
    }
    st.ret
}

// ---------------------------------------------------------------------
// Calling a user `+vi-*` function through the shell
// ---------------------------------------------------------------------

/// Globals the generated script copies results into.
const OUT_META: &str = "__p10k_vi_meta";
const OUT_HOOK_COM: &str = "__p10k_vi_hc";
const OUT_USER_DATA: &str = "__p10k_vi_ud";
const OUT_VCS_COMM: &str = "__p10k_vi_vc";
const OUT_BACKEND_MISC: &str = "__p10k_vi_bm";
const OUT_MSGS: &str = "__p10k_vi_msgs";
const OUT_ARRAY_PREFIX: &str = "__p10k_vi_arr_";

/// Single-quote `s` for the shell.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn assoc_literal(a: &Assoc) -> String {
    a.pairs()
        .iter()
        .map(|(k, v)| format!("{} {}", shell_quote(k), shell_quote(v)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn array_literal(a: &[String]) -> String {
    a.iter().map(|s| shell_quote(s)).collect::<Vec<_>>().join(" ")
}

/// `true`/`false` as the theme stores them in `VCS_WORKDIR_*`.
fn bool_word(b: bool) -> &'static str {
    if b {
        "true"
    } else {
        "false"
    }
}

/// The script that runs `func args...` with the hook variables in scope
/// (the `vcs_info` options are `emulate -L zsh; setopt extendedglob
/// NO_warn_create_global`) and exports the results.
pub(crate) fn build_hook_script(func: &str, args: &[String], st: &HookState) -> String {
    let v = &st.vars;
    let mut s = String::new();
    s.push_str("() {\n");
    s.push_str("emulate -L zsh\nsetopt extendedglob no_warn_create_global\n");
    s.push_str("local -A hook_com vcs_comm user_data backend_misc\n");
    s.push_str("local -a msgs\n");
    for (name, _) in &st.arrays {
        s.push_str(&format!("local -a {name}\n"));
    }
    s.push_str("local __k\nlocal -i __rc ret=0\n");
    s.push_str(&format!(
        "local vcs={} usercontext={} rrn={} quiltmode={}\n",
        shell_quote(&v.vcs),
        shell_quote(&v.usercontext),
        shell_quote(&v.rrn),
        shell_quote(&v.quiltmode)
    ));
    s.push_str(&format!("local -i maxexports={}\n", v.maxexports));
    s.push_str(&format!("hook_com=( {} )\n", assoc_literal(&st.hook_com)));
    s.push_str(&format!("vcs_comm=( {} )\n", assoc_literal(&st.vcs_comm)));
    s.push_str(&format!("user_data=( {} )\n", assoc_literal(&st.user_data)));
    s.push_str(&format!("backend_misc=( {} )\n", assoc_literal(&st.backend_misc)));
    s.push_str(&format!("msgs=( {} )\n", array_literal(&st.msgs)));
    for (name, vals) in &st.arrays {
        s.push_str(&format!("{name}=( {} )\n", array_literal(vals)));
    }
    // VCS_INFO_hook: `typeset -g -r vcs rrn usercontext maxexports msgs vcs_comm`
    // — a hook assigning to one of them fails like any readonly write.
    s.push_str("typeset -r vcs rrn usercontext maxexports msgs vcs_comm\n");
    s.push_str(&format!("VCS_WORKDIR_DIRTY={}\n", bool_word(st.flags.dirty)));
    s.push_str(&format!("VCS_WORKDIR_HALF_DIRTY={}\n", bool_word(st.flags.half_dirty)));
    s.push_str(&format!(
        "vcs_visual_identifier={}\n",
        shell_quote(&st.flags.visual_identifier)
    ));
    let quoted_args = args.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
    s.push_str(&format!("{func} {quoted_args}\n__rc=$?\n"));
    for (out, var) in [
        (OUT_HOOK_COM, "hook_com"),
        (OUT_USER_DATA, "user_data"),
        (OUT_VCS_COMM, "vcs_comm"),
        (OUT_BACKEND_MISC, "backend_misc"),
    ] {
        s.push_str(&format!(
            "typeset -ga {out}\n{out}=()\nfor __k in \"${{(@k){var}}}\"; do {out}+=( \"$__k\" \"${{{var}[$__k]}}\" ); done\n"
        ));
    }
    s.push_str(&format!("typeset -ga {OUT_MSGS}\n{OUT_MSGS}=( \"${{(@)msgs}}\" )\n"));
    for (name, _) in &st.arrays {
        s.push_str(&format!(
            "typeset -ga {OUT_ARRAY_PREFIX}{name}\n{OUT_ARRAY_PREFIX}{name}=( \"${{(@){name}}}\" )\n"
        ));
    }
    s.push_str(&format!("typeset -ga {OUT_META}\n{OUT_META}=( $__rc $ret )\n"));
    s.push_str("}\n");
    s
}

/// What the script exported.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct HookOut {
    pub meta: Vec<String>,
    pub hook_com: Vec<String>,
    pub user_data: Vec<String>,
    pub vcs_comm: Vec<String>,
    pub backend_misc: Vec<String>,
    pub msgs: Vec<String>,
    pub arrays: Vec<(String, Vec<String>)>,
    pub dirty: Option<String>,
    pub half_dirty: Option<String>,
    pub visual_identifier: Option<String>,
}

/// Fold `out` back into `st`; returns the function's exit status.
pub(crate) fn apply_hook_out(st: &mut HookState, out: HookOut) -> i32 {
    let rc = out.meta.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    st.ret = out.meta.get(1).and_then(|s| s.parse().ok()).unwrap_or(st.ret);
    st.hook_com = Assoc::from_flat(&out.hook_com);
    st.user_data = Assoc::from_flat(&out.user_data);
    st.vcs_comm = Assoc::from_flat(&out.vcs_comm);
    st.backend_misc = Assoc::from_flat(&out.backend_misc);
    st.msgs = out.msgs;
    for (name, vals) in out.arrays {
        st.set_array(&name, vals);
    }
    if let Some(d) = out.dirty {
        st.flags.dirty = d == "true";
    }
    if let Some(d) = out.half_dirty {
        st.flags.half_dirty = d == "true";
    }
    if let Some(v) = out.visual_identifier {
        st.flags.visual_identifier = v;
    }
    rc
}

/// Run `func` in the embedded shell. A failure to execute leaves `st`
/// untouched and returns exit status 1: `VCS_INFO_hook` breaks out of its
/// loop on any non-zero `$?`, and a function that cannot run fails.
pub(crate) fn call_shell_function(func: &str, args: &[String], st: &mut HookState) -> i32 {
    use crate::ported::params::{getaparam, getsparam, unsetparam};
    let script = build_hook_script(func, args, st);
    if let Err(e) = crate::ported::exec::execute_script(&script) {
        tracing::warn!(target: "p10k", func, error = %e, "vcs_info hook function failed");
        return 1;
    }
    let take = |name: &str| {
        let v = getaparam(name).unwrap_or_default();
        unsetparam(name);
        v
    };
    let out = HookOut {
        meta: take(OUT_META),
        hook_com: take(OUT_HOOK_COM),
        user_data: take(OUT_USER_DATA),
        vcs_comm: take(OUT_VCS_COMM),
        backend_misc: take(OUT_BACKEND_MISC),
        msgs: take(OUT_MSGS),
        arrays: st
            .arrays
            .iter()
            .map(|(n, _)| (n.clone(), take(&format!("{OUT_ARRAY_PREFIX}{n}"))))
            .collect(),
        dirty: getsparam("VCS_WORKDIR_DIRTY"),
        half_dirty: getsparam("VCS_WORKDIR_HALF_DIRTY"),
        visual_identifier: getsparam("vcs_visual_identifier"),
    };
    apply_hook_out(st, out)
}

// ---------------------------------------------------------------------
// Small zsh-semantics helpers shared by the backends
// ---------------------------------------------------------------------

/// `${path:t}` — trailing slashes are ignored (hist.c `remlpaths`).
pub(crate) fn zsh_tail(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or("")
}

/// The order in which `${(k)assoc}` lists `keys` (given in insertion
/// order) in zsh: a 17-bucket table (`newparamtable(17)`), new nodes at the
/// front of their chain, 4x growth once the count reaches twice the size
/// (hashtable.c `addhashnode2`, `expandhashtable`), scanned bucket by
/// bucket.
pub(crate) fn zsh_hash_order(keys: &[String]) -> Vec<String> {
    use crate::ported::hashtable::hasher;
    let mut size = 17usize;
    let mut buckets: Vec<Vec<String>> = vec![Vec::new(); size];
    let mut count = 0usize;
    let mut seen: Vec<&String> = Vec::new();
    for key in keys {
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        buckets[hasher(key) as usize % size].insert(0, key.clone());
        count += 1;
        if count >= size * 2 {
            let old = std::mem::take(&mut buckets);
            size *= 4;
            buckets = vec![Vec::new(); size];
            for chain in old {
                for k in chain {
                    buckets[hasher(&k) as usize % size].insert(0, k);
                }
            }
        }
    }
    buckets.into_iter().flatten().collect()
}

/// A zsh pattern match of `s` against `pat` (`[[ s == pat ]]`).
pub(crate) fn pattern_matches(pat: &str, s: &str) -> bool {
    let mut tokenized = pat.to_string();
    crate::ported::glob::tokenize(&mut tokenized);
    match crate::ported::pattern::patcompile(&tokenized, 0, None) {
        Some(prog) => crate::ported::pattern::pattry(&prog, s),
        None => false,
    }
}

/// `[[ s == ${~pat} ]]`: `pat` comes from a parameter expansion with
/// `GLOB_SUBST` semantics (`shtokenize`), as `disable-patterns` entries do.
pub(crate) fn subst_pattern_matches(pat: &str, s: &str) -> bool {
    let mut tokenized = pat.to_string();
    crate::ported::glob::shtokenize(&mut tokenized);
    match crate::ported::pattern::patcompile(&tokenized, 0, None) {
        Some(prog) => crate::ported::pattern::pattry(&prog, s),
        None => false,
    }
}

/// `${#${(@M)items:#${(~j,|,)pats}}} -gt 0`: join `pats` with `|`, let the
/// `~` flag make the joined text a pattern, and test whether any of
/// `items` matches it as a whole.
pub(crate) fn any_matches_joined(items: &[String], pats: &[String]) -> bool {
    let joined = pats.join("|");
    items.iter().any(|item| subst_pattern_matches(&joined, item))
}

/// Read the parameter `name` as `VCS_INFO_quilt-standalone-detect` does,
/// through the embedded shell. With `uniq_arrays`, an array parameter is
/// first made unique in place (`typeset -gU name`), a side effect the
/// original has.
pub(crate) fn read_shell_param(name: &str, uniq_arrays: bool) -> ParamValue {
    use crate::ported::params::{getaparam, unsetparam};
    const OUT: &str = "__p10k_vi_param";
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return ParamValue::Unset;
    }
    let uniq = if uniq_arrays { format!("typeset -gU {name}\n") } else { String::new() };
    let script = format!(
        "() {{\nemulate -L zsh\nsetopt extendedglob\nlocal __n={name}\ntypeset -ga {OUT}\n\
         case ${{(Pt)__n}} in\n\
         *association*) {OUT}=( assoc \"${{(@kvP)__n}}\" );;\n\
         *array*) {uniq}{OUT}=( array \"${{(@P)__n}}\" );;\n\
         *scalar*) {OUT}=( scalar \"${{(P)__n}}\" );;\n\
         *) {OUT}=( unset );;\n\
         esac\n}}\n"
    );
    if crate::ported::exec::execute_script(&script).is_err() {
        return ParamValue::Unset;
    }
    let v = getaparam(OUT).unwrap_or_default();
    unsetparam(OUT);
    let rest = v.get(1..).unwrap_or(&[]);
    match v.first().map(String::as_str) {
        Some("assoc") => ParamValue::Assoc(
            rest.chunks(2)
                .map(|kv| (kv[0].clone(), kv.get(1).cloned().unwrap_or_default()))
                .collect(),
        ),
        Some("array") => ParamValue::Array(rest.to_vec()),
        Some("scalar") => ParamValue::Scalar(rest.first().cloned().unwrap_or_default()),
        _ => ParamValue::Unset,
    }
}

/// Count of `key`s in a `HashMap` is not needed; this keeps `HashMap`
/// available to the zformat spec builders in callers.
pub(crate) type Specs = HashMap<char, String>;

#[cfg(test)]
pub(crate) mod test_host {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A host with canned styles and recorded function calls.
    #[derive(Default)]
    pub(crate) struct MockHost {
        /// `(context, style)` -> values; the context is matched exactly.
        pub styles: HashMap<(String, String), Vec<String>>,
        pub functions: Vec<String>,
        pub native: Vec<String>,
        pub calls: RefCell<Vec<(String, Vec<String>)>>,
        /// Per function: mutate the state, return the exit status.
        pub behaviour: HashMap<String, fn(&mut HookState) -> i32>,
        /// Shell parameters visible to `param`.
        pub params: HashMap<String, ParamValue>,
    }

    impl MockHost {
        pub(crate) fn with_style(mut self, ctx: &str, style: &str, vals: &[&str]) -> Self {
            self.styles.insert(
                (ctx.to_string(), style.to_string()),
                vals.iter().map(|s| s.to_string()).collect(),
            );
            self
        }
    }

    impl HookHost for MockHost {
        fn style(&self, ctx: &str, style: &str) -> Vec<String> {
            self.styles
                .get(&(ctx.to_string(), style.to_string()))
                .cloned()
                .unwrap_or_default()
        }

        fn native_hook(&self, name: &str, args: &[String], _st: &mut HookState) -> Option<i32> {
            self.native.iter().any(|n| n == name).then(|| {
                self.calls.borrow_mut().push((format!("native:{name}"), args.to_vec()));
                0
            })
        }

        fn user_function_exists(&self, func: &str) -> bool {
            self.functions.iter().any(|f| f == func)
        }

        fn param(&self, name: &str) -> ParamValue {
            self.params.get(name).cloned().unwrap_or(ParamValue::Unset)
        }

        fn call_user_function(&self, func: &str, args: &[String], st: &mut HookState) -> i32 {
            self.calls.borrow_mut().push((func.to_string(), args.to_vec()));
            self.behaviour.get(func).map(|f| f(st)).unwrap_or(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_host::MockHost;
    use super::*;

    fn state(vcs: &str, rrn: &str) -> HookState {
        let mut st = HookState::new();
        st.vars.vcs = vcs.to_string();
        st.vars.rrn = rrn.to_string();
        st
    }

    #[test]
    fn contexts_follow_vcs_info_hook() {
        let st = state("hg-git", "proj");
        assert_eq!(
            hook_context(&st.vars, "set-message"),
            ":vcs_info:hg-git+set-message:default:proj"
        );
        assert_eq!(static_context("post-backend"), ":vcs_info-static_hooks:post-backend");
        assert_eq!(style_context(&st.vars), ":vcs_info:hg-git:default:proj");
        let init = Vars::initial();
        assert_eq!(
            hook_context(&init, "start-up"),
            ":vcs_info:-init-+start-up:default:-all-"
        );
    }

    #[test]
    fn static_hooks_precede_context_hooks() {
        let host = MockHost::default()
            .with_style(":vcs_info-static_hooks:set-message", "hooks", &["s1", "s2"])
            .with_style(":vcs_info:bzr+set-message:default:repo", "hooks", &["c1"]);
        let st = state("bzr", "repo");
        assert_eq!(resolve_hooks(&host, &st.vars, "set-message"), vec!["s1", "s2", "c1"]);
        assert!(resolve_hooks(&host, &st.vars, "post-backend").is_empty());
    }

    #[test]
    fn no_hooks_returns_zero_and_leaves_ret() {
        let host = MockHost::default();
        let mut st = state("bzr", "repo");
        st.ret = 7;
        assert_eq!(run_hook(&host, &mut st, "set-message", &[]), 0);
        assert_eq!(st.ret, 7);
    }

    fn set_ret_one(st: &mut HookState) -> i32 {
        st.ret = 1;
        st.hook_com.set("branch", "replaced");
        0
    }

    fn fail(_st: &mut HookState) -> i32 {
        1
    }

    #[test]
    fn hooks_run_in_order_unknown_skipped_ret_returned() {
        let mut host = MockHost::default().with_style(
            ":vcs_info:bzr+set-message:default:repo",
            "hooks",
            &["first", "missing", "second"],
        );
        host.functions = vec!["+vi-first".into(), "+vi-second".into()];
        host.behaviour.insert("+vi-first".into(), set_ret_one);
        let mut st = state("bzr", "repo");
        let args = vec!["0".to_string(), "%b".to_string()];
        assert_eq!(run_hook(&host, &mut st, "set-message", &args), 1);
        assert_eq!(st.hook_com.get("branch"), "replaced");
        let calls = host.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ("+vi-first".to_string(), args.clone()));
        assert_eq!(calls[1].0, "+vi-second");
    }

    #[test]
    fn nonzero_exit_stops_the_chain_but_ret_still_returned() {
        let mut host = MockHost::default().with_style(
            ":vcs_info:bzr+post-backend:default:repo",
            "hooks",
            &["stop", "never"],
        );
        host.functions = vec!["+vi-stop".into(), "+vi-never".into()];
        host.behaviour.insert("+vi-stop".into(), fail);
        let mut st = state("bzr", "repo");
        st.ret = 9; // ret is reset to 0 once hooks exist
        assert_eq!(run_hook(&host, &mut st, "post-backend", &[]), 0);
        assert_eq!(host.calls.borrow().len(), 1);
    }

    #[test]
    fn native_hooks_take_precedence_over_functions() {
        let mut host = MockHost::default().with_style(
            ":vcs_info:svn+set-message:default:wc",
            "hooks",
            &["vcs-detect-changes"],
        );
        host.native = vec!["vcs-detect-changes".into()];
        host.functions = vec!["+vi-vcs-detect-changes".into()];
        let mut st = state("svn", "wc");
        run_hook(&host, &mut st, "set-message", &[]);
        let calls = host.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "native:vcs-detect-changes");
    }

    #[test]
    fn style_truth_words() {
        let t = |v: &[&str]| style_is_true(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(t(&["true"]) && t(&["yes"]) && t(&["on"]) && t(&["1"]));
        assert!(!t(&["false"]) && !t(&["0"]) && !t(&[]));
        let host = MockHost::default().with_style(":c", "x", &["false"]);
        assert!(!host.style_tt(":c", "x"));
        assert!(host.style_tt(":c", "unset-style"));
        assert!(!host.style_t(":c", "unset-style"));
        assert_eq!(host.style_s(":c", "x").as_deref(), Some("false"));
    }

    #[test]
    fn script_declares_scope_calls_function_and_exports() {
        let mut st = state("hg", "proj");
        st.hook_com.set("branch", "it's");
        st.hook_com.set("staged", "");
        st.set_array("hgbmarks", vec!["a".into(), "b c".into()]);
        st.flags.dirty = true;
        st.flags.visual_identifier = "VCS_HG_ICON".into();
        let s = build_hook_script("+vi-x", &["0".into(), "a'b".into()], &st);
        assert!(s.starts_with("() {\n"));
        assert!(s.contains("local vcs='hg' usercontext='default' rrn='proj' quiltmode='addon'"));
        assert!(s.contains("hook_com=( 'branch' 'it'\\''s' 'staged' '' )"));
        assert!(s.contains("local -a hgbmarks\n"));
        assert!(s.contains("hgbmarks=( 'a' 'b c' )"));
        assert!(s.contains("VCS_WORKDIR_DIRTY=true"));
        assert!(s.contains("VCS_WORKDIR_HALF_DIRTY=false"));
        assert!(s.contains("vcs_visual_identifier='VCS_HG_ICON'"));
        assert!(s.contains("+vi-x '0' 'a'\\''b'\n__rc=$?"));
        assert!(s.contains("__p10k_vi_arr_hgbmarks=( \"${(@)hgbmarks}\" )"));
        // the call happens after every variable is established
        assert!(s.find("hook_com=(").unwrap() < s.find("+vi-x ").unwrap());
        assert!(s.ends_with("}\n"));
    }

    #[test]
    fn exported_results_fold_back_into_state() {
        let mut st = state("hg", "proj");
        st.set_array("hgbmarks", vec!["a".into()]);
        let out = HookOut {
            meta: vec!["3".into(), "1".into()],
            hook_com: vec!["branch".into(), "x".into(), "misc".into(), "".into()],
            user_data: vec!["k".into(), "v".into()],
            vcs_comm: vec![],
            backend_misc: vec![],
            msgs: vec!["m0".into()],
            arrays: vec![("hgbmarks".into(), vec![])],
            dirty: Some("true".into()),
            half_dirty: Some("false".into()),
            visual_identifier: Some("VCS_SVN_ICON".into()),
        };
        assert_eq!(apply_hook_out(&mut st, out), 3);
        assert_eq!(st.ret, 1);
        assert_eq!(st.hook_com.get("branch"), "x");
        assert!(st.hook_com.has("misc"));
        assert_eq!(st.user_data.get("k"), "v");
        assert_eq!(st.msgs, vec!["m0"]);
        assert!(st.array("hgbmarks").is_empty());
        assert!(st.flags.dirty && !st.flags.half_dirty);
        assert_eq!(st.flags.visual_identifier, "VCS_SVN_ICON");
    }

    #[test]
    fn tail_ignores_trailing_slashes() {
        assert_eq!(zsh_tail("/home/u/proj/"), "proj");
        assert_eq!(zsh_tail("/home/u/proj"), "proj");
        assert_eq!(zsh_tail("proj"), "proj");
        assert_eq!(zsh_tail("/"), "");
    }

    #[test]
    fn hash_order_scans_buckets_with_front_insertion() {
        // hasher("M") = 77 -> bucket 77 % 17 = 9, hasher("?") = 63 -> 12,
        // hasher("A") = 65 -> 14 (c:86 `hashval += (hashval << 5) + c`).
        let keys: Vec<String> = ["?", "A", "M"].iter().map(|s| s.to_string()).collect();
        assert_eq!(zsh_hash_order(&keys), vec!["M", "?", "A"]);
        // 17 * 3 = 51: bucket 0 holds "<0x00>"-hash strings only; same-bucket
        // keys list newest first ("B" = 66 and "S" = 83 both land in 15).
        let keys: Vec<String> = ["B", "S"].iter().map(|s| s.to_string()).collect();
        assert_eq!(zsh_hash_order(&keys), vec!["S", "B"]);
        // a repeated key keeps its first position
        let keys: Vec<String> = ["B", "S", "B"].iter().map(|s| s.to_string()).collect();
        assert_eq!(zsh_hash_order(&keys), vec!["S", "B"]);
    }
}
