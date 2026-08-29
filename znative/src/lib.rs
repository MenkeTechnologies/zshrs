//! # `znative` — native plugin SDK for zshrs
//!
//! zshrs is the first JIT-compiled Unix shell; this crate is what makes it
//! the first compiled shell that **hosts third-party plugins written in
//! a native compiled language** (Rust) and loaded at runtime — no
//! recompile of the shell, no zsh script glue. A plugin is an ordinary
//! `cdylib` that the shell `dlopen`s via `zmodload -R <path>`.
//!
//! The boundary between host and plugin is a hand-rolled, versioned
//! **C ABI** (`#[repr(C)]` structs + `extern "C"` fn pointers). Both
//! sides depend on THIS crate so they agree on the exact layout. Nothing
//! about Rust's unstable `repr(Rust)` layout, allocator, or panic ABI
//! crosses the boundary — only C-representable data.
//!
//! ## Writing a plugin
//!
//! ```ignore
//! use znative::{declare_plugin, Args, Host};
//! use std::os::raw::c_int;
//!
//! fn hello(host: &Host, args: &Args) -> c_int {
//!     host.print(&format!("hello from rust, argv={:?}\n", args.to_vec()));
//!     0
//! }
//!
//! declare_plugin! {
//!     name: "hello",
//!     version: "0.1.0",
//!     builtins: { "rhello" => hello },
//! }
//! ```
//!
//! `Cargo.toml`:
//! ```toml
//! [lib]
//! crate-type = ["cdylib"]
//! [dependencies]
//! znative = "0.12"
//! ```
//!
//! `cargo build` produces `libhello.dylib` / `libhello.so`; then inside
//! zshrs: `zmodload -R ~/plugins/libhello.dylib` and `rhello` is a live
//! command.
//!
//! ## Host API
//!
//! Inside a handler, [`Host`] is the shell's callback table:
//!
//! | Method | Purpose |
//! | --- | --- |
//! | [`print`](Host::print) | write to the shell's stdout |
//! | [`eval`](Host::eval) | run shell code, return its exit status |
//! | [`getvar`](Host::getvar) / [`setvar`](Host::setvar) | read/set a shell scalar parameter |
//! | [`getfunction`](Host::getfunction) / [`addfunction`](Host::addfunction) | read a function's deparsed body / define one (also deparse-as-a-service) — ABI v3 |
//! | [`register_builtin`](Host::register_builtin) | register a command handler dynamically |
//! | [`add_match`](Host::add_match) / [`install_completion`](Host::install_completion) | emit a completion candidate / wire a native completion into compsys — ABI v2 |

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

/// ABI version. Bumped on ANY change to [`HostApi`], [`PluginInfo`],
/// [`BuiltinFn`], or [`InitFn`] layout/semantics. The host refuses to
/// load a plugin whose `abi_version` does not match its own — a
/// mismatched struct layout is undefined behaviour, so this is a hard
/// gate, not a warning.
///
/// v2: added [`HostApi::register_completion`] for native completions.
/// v3: added [`HostApi::getfunction`] / [`HostApi::addfunction`].
/// v4: added [`HostApi::register_compfn`] / [`HostApi::comp_dispatch`] /
///     [`HostApi::empty_command_hash`] — override a compsys `_fn` (e.g.
///     `_command_names`) with a native handler that dispatches other
///     completion functions and applies the `local -A +h commands` shadow.
pub const ABI_VERSION: u32 = 4;

/// A plugin-provided compsys function override — the native replacement
/// for a shell completion function named `_NAME` (e.g. `_command_names`).
/// Same C-ABI shape as [`BuiltinFn`]: registered via
/// [`HostApi::register_compfn`] and dispatched by the host's completion
/// router (`try_rust_dispatch`) in place of the built-in Rust port or the
/// autoloaded shell function. `argv[0]` is the `_NAME`; `argv[1..]` are the
/// completion-function arguments. (ABI v4.)
pub type CompFn =
    extern "C" fn(host: *const HostApi, argc: usize, argv: *const *const c_char) -> c_int;

/// The one symbol every plugin `cdylib` must export. The host resolves
/// it with `dlsym` after `dlopen`. Signature is [`InitFn`].
pub const INIT_SYMBOL: &[u8] = b"znative_init\0";

/// A plugin-provided command handler.
///
/// * `host`   — the host API table (call back into the shell through it).
/// * `argc`   — number of elements in `argv`.
/// * `argv`   — NUL-terminated C strings; `argv[0]` is the command name,
///   `argv[1..]` the arguments. Valid only for the duration of the call;
///   copy anything you need to keep.
///
/// Returns the command's exit status (0 = success), like any shell
/// builtin.
pub type BuiltinFn =
    extern "C" fn(host: *const HostApi, argc: usize, argv: *const *const c_char) -> c_int;

/// Signature of [`INIT_SYMBOL`]. Called exactly once, right after the
/// dylib is loaded. The plugin registers its builtins through
/// `host.register_builtin` and returns a pointer to a `'static`
/// [`PluginInfo`] describing itself (or null on failure).
pub type InitFn = extern "C" fn(host: *const HostApi) -> *const PluginInfo;

/// The host API table handed to the plugin. Every field is a C-ABI
/// function pointer into zshrs. Layout is frozen by [`ABI_VERSION`].
///
/// A single instance lives for the whole process; plugins may store the
/// `*const HostApi` they are given and call through it from any builtin.
#[repr(C)]
pub struct HostApi {
    /// Must equal [`ABI_VERSION`]. Checked by the plugin's own
    /// `declare_plugin!` glue before it trusts the rest of the table.
    pub abi_version: u32,
    /// Reserved for the host; opaque to plugins. Currently null.
    pub ctx: *mut c_void,
    /// Register a command name → handler. Returns 0 on success. Names
    /// registered here resolve as commands in the shell (after real
    /// builtins, before PATH lookup). `name` is copied by the host.
    pub register_builtin:
        extern "C" fn(host: *const HostApi, name: *const c_char, handler: BuiltinFn) -> c_int,
    /// Write text to the shell's stdout (no trailing newline added).
    pub print: extern "C" fn(host: *const HostApi, text: *const c_char),
    /// Evaluate a fragment of shell code in the host and return its exit
    /// status. `code` is UTF-8, NUL-terminated.
    pub eval: extern "C" fn(host: *const HostApi, code: *const c_char) -> c_int,
    /// Read a shell scalar parameter by name. Returns a freshly
    /// allocated C string the caller MUST release with `free_cstring`,
    /// or null if the parameter is unset.
    pub getvar: extern "C" fn(host: *const HostApi, name: *const c_char) -> *mut c_char,
    /// Set a shell scalar parameter. Returns 0 on success.
    pub setvar:
        extern "C" fn(host: *const HostApi, name: *const c_char, value: *const c_char) -> c_int,
    /// Release a string previously returned by `getvar`.
    pub free_cstring: extern "C" fn(host: *const HostApi, s: *mut c_char),
    /// Register a native completion for command `cmd`. `generator` names
    /// a builtin that prints one candidate per line for the current word
    /// (see [`Host::add_match`]). The host wires it into zsh's completion
    /// system (compsys) lazily — the actual `compdef` runs the first time
    /// completion fires, at a safe point in the completion pipeline, so
    /// this is cheap and safe to call from `znative_init`. Returns 0
    /// on success. (ABI v2.)
    pub register_completion:
        extern "C" fn(host: *const HostApi, cmd: *const c_char, generator: *const c_char) -> c_int,
    /// Read a shell **function**'s body by name — the same deparsed text
    /// `${functions[name]}` yields (one statement per line, tab-indented;
    /// `builtin autoload -X…` for an autoload stub). Returns a freshly
    /// allocated C string the caller MUST release with `free_cstring`, or
    /// null if no such function is defined. This is the only structured
    /// read of a function; `getvar` reads scalars, `eval` returns only a
    /// status. (ABI v3.)
    pub getfunction: extern "C" fn(host: *const HostApi, name: *const c_char) -> *mut c_char,
    /// Define (or replace) a shell function `name` with `body` — exactly
    /// like `functions[name]=body`: the body is parsed and installed in
    /// `shfunctab`, so a subsequent `getfunction` returns its deparsed
    /// form and the shell can call it (as a command, ZLE widget, hook,
    /// …). Returns 0 on success, non-zero if `body` fails to parse.
    /// (ABI v3.)
    pub addfunction:
        extern "C" fn(host: *const HostApi, name: *const c_char, body: *const c_char) -> c_int,
    /// Register a native override for the compsys function `name` (a
    /// `_NAME`, e.g. `_command_names`). Once registered, the host's
    /// completion router dispatches `handler` in place of the built-in
    /// Rust port or the autoloaded shell function. Returns 0 on success.
    /// `name` is copied by the host. (ABI v4.)
    pub register_compfn:
        extern "C" fn(host: *const HostApi, name: *const c_char, handler: CompFn) -> c_int,
    /// Invoke a compsys function `name` (e.g. `_alternative`, `_path_commands`)
    /// with `argv[0..argc]` as its arguments — the host resolves it exactly
    /// as the completer chain would (native port, then shell autoload), in
    /// the *current* completion scope so `local`/tag state nests correctly.
    /// Returns the function's status. This is how an overriding
    /// [`CompFn`] delegates the bulk of its work. (ABI v4.)
    pub comp_dispatch: extern "C" fn(
        host: *const HostApi,
        name: *const c_char,
        argc: usize,
        argv: *const *const c_char,
    ) -> c_int,
    /// Empty the shell's command-name hash (`cmdnamtab`), the observable
    /// effect of `local -a +h path; local -A +h commands` inside a
    /// completion function: a subsequent `compadd -k commands` then adds
    /// nothing until the hash refills. Used by an overriding
    /// `_command_names` to reproduce the customized shadow. (ABI v4.)
    pub empty_command_hash: extern "C" fn(host: *const HostApi),
}

/// What a plugin returns from its [`InitFn`]. The strings must have
/// `'static` lifetime (typically string literals via the
/// `declare_plugin!` macro).
#[repr(C)]
pub struct PluginInfo {
    /// Must equal [`ABI_VERSION`]. Redundant with the host-side check,
    /// but lets the host reject a plugin that lied about its ABI.
    pub abi_version: u32,
    /// Plugin name, NUL-terminated. Used for `zmodload -R` listing and
    /// `zmodload -uR <name>` unload.
    pub name: *const c_char,
    /// Plugin version, NUL-terminated. Informational.
    pub version: *const c_char,
}

// PluginInfo is only ever pointed at `'static` data; it carries no
// interior mutability. Marking it Sync lets the macro place it in a
// `static`.
unsafe impl Sync for PluginInfo {}

// ============================================================
// Ergonomic wrappers for plugin authors. None of this crosses the ABI;
// it is convenience over the raw pointers above.
// ============================================================

/// Safe wrapper over `*const HostApi` for use inside a builtin. Cheap to
/// construct; borrows the host table.
pub struct Host {
    api: *const HostApi,
}

impl Host {
    /// Wrap a raw host pointer. `unsafe` because the caller asserts the
    /// pointer is the valid table the host passed in.
    ///
    /// # Safety
    /// `api` must be the non-null `*const HostApi` the host handed to the
    /// plugin (in `znative_init` or a [`BuiltinFn`] call) and must
    /// remain valid for the lifetime of this `Host`.
    pub unsafe fn from_raw(api: *const HostApi) -> Self {
        Host { api }
    }

    #[inline]
    fn t(&self) -> &HostApi {
        // Safe: constructed only from a valid host pointer.
        unsafe { &*self.api }
    }

    /// Register a command handler by name. Usually done for you by
    /// `declare_plugin!`; exposed for dynamic registration.
    pub fn register_builtin(&self, name: &str, handler: BuiltinFn) -> bool {
        let Ok(cname) = CString::new(name) else {
            return false;
        };
        ((self.t().register_builtin)(self.api, cname.as_ptr(), handler)) == 0
    }

    /// Write `text` to the shell's stdout.
    pub fn print(&self, text: &str) {
        if let Ok(c) = CString::new(text) {
            (self.t().print)(self.api, c.as_ptr());
        }
    }

    /// Evaluate shell `code`; returns its exit status.
    pub fn eval(&self, code: &str) -> i32 {
        match CString::new(code) {
            Ok(c) => (self.t().eval)(self.api, c.as_ptr()),
            Err(_) => 1,
        }
    }

    /// Read shell scalar `$name`, or `None` if unset.
    pub fn getvar(&self, name: &str) -> Option<String> {
        let cname = CString::new(name).ok()?;
        let raw = (self.t().getvar)(self.api, cname.as_ptr());
        if raw.is_null() {
            return None;
        }
        // Safe: host contract says this is a valid C string owned by us.
        let s = unsafe { CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned();
        (self.t().free_cstring)(self.api, raw);
        Some(s)
    }

    /// Set shell scalar `$name = value`. Returns true on success.
    pub fn setvar(&self, name: &str, value: &str) -> bool {
        let (Ok(cn), Ok(cv)) = (CString::new(name), CString::new(value)) else {
            return false;
        };
        (self.t().setvar)(self.api, cn.as_ptr(), cv.as_ptr()) == 0
    }

    /// Emit one completion candidate. Call this from a **completion
    /// generator** (a builtin wired up via `completions:` in
    /// [`declare_plugin!`] or [`Host::install_completion`]). Each call
    /// prints one candidate on its own line; the compsys glue collects
    /// them and feeds them to `compadd`. Equivalent to
    /// `self.print(&format!("{word}\n"))`.
    pub fn add_match(&self, word: &str) {
        self.print(word);
        self.print("\n");
    }

    /// Wire a native completion generator into zsh's completion system
    /// (compsys) for command `cmd`. `generator_builtin` is the name of a
    /// builtin (registered via [`declare_plugin!`]) that prints one
    /// candidate per line — call [`Host::add_match`] from it.
    ///
    /// The host defers the actual `compdef` wiring until the first time
    /// completion fires, so this is safe to call from
    /// `znative_init`. `declare_plugin!`'s `completions:` section
    /// calls this for you.
    ///
    /// The generator receives, as its arguments: `$CURRENT` (1-based index
    /// of the word being completed) followed by every word on the line —
    /// so `words[current]` is the word to complete.
    pub fn install_completion(&self, cmd: &str, generator_builtin: &str) -> bool {
        let (Ok(cc), Ok(cg)) = (CString::new(cmd), CString::new(generator_builtin)) else {
            return false;
        };
        (self.t().register_completion)(self.api, cc.as_ptr(), cg.as_ptr()) == 0
    }

    /// Read a shell function's deparsed body — the same text
    /// `${functions[name]}` yields — or `None` if it is not defined.
    /// Useful as a deparse-as-a-service: `addfunction(tmp, src)` then
    /// `getfunction(tmp)` returns `src` re-formatted by the shell's own
    /// pretty-printer. (ABI v3.)
    pub fn getfunction(&self, name: &str) -> Option<String> {
        let cname = CString::new(name).ok()?;
        let raw = (self.t().getfunction)(self.api, cname.as_ptr());
        if raw.is_null() {
            return None;
        }
        let s = unsafe { CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned();
        (self.t().free_cstring)(self.api, raw);
        Some(s)
    }

    /// Define (or replace) shell function `name` with `body`, exactly like
    /// `functions[name]=body`. Returns `true` on success, `false` if the
    /// body fails to parse. (ABI v3.)
    pub fn addfunction(&self, name: &str, body: &str) -> bool {
        let (Ok(cn), Ok(cb)) = (CString::new(name), CString::new(body)) else {
            return false;
        };
        (self.t().addfunction)(self.api, cn.as_ptr(), cb.as_ptr()) == 0
    }

    /// Register `handler` as the native override for compsys function
    /// `name` (a `_NAME`). Usually done for you by `declare_plugin!`'s
    /// `compfns:` section. Returns `true` on success. (ABI v4.)
    pub fn register_compfn(&self, name: &str, handler: CompFn) -> bool {
        let Ok(cname) = CString::new(name) else {
            return false;
        };
        (self.t().register_compfn)(self.api, cname.as_ptr(), handler) == 0
    }

    /// Invoke compsys function `name` with `args` in the current
    /// completion scope; returns its status. Delegates to the host's
    /// completer resolution (native port → shell autoload). (ABI v4.)
    pub fn comp_dispatch(&self, name: &str, args: &[&str]) -> i32 {
        let Ok(cname) = CString::new(name) else {
            return 1;
        };
        // Build argv[0..] = name, args… as owned CStrings + a NULL-free
        // pointer array valid for the duration of the call.
        let mut cargs: Vec<CString> = Vec::with_capacity(args.len() + 1);
        cargs.push(cname);
        for a in args {
            match CString::new(*a) {
                Ok(c) => cargs.push(c),
                Err(_) => return 1,
            }
        }
        let argv: Vec<*const c_char> = cargs.iter().map(|c| c.as_ptr()).collect();
        (self.t().comp_dispatch)(self.api, argv[0], argv.len(), argv.as_ptr())
    }

    /// Empty the command-name hash — the observable effect of the
    /// `local -A +h commands` shadow inside a completion function. (ABI v4.)
    pub fn empty_command_hash(&self) {
        (self.t().empty_command_hash)(self.api);
    }
}

/// Safe view over a builtin's `(argc, argv)`. `argv[0]` is the command
/// name.
pub struct Args {
    items: Vec<String>,
}

impl Args {
    /// Decode a raw `(argc, argv)` pair into owned `String`s.
    ///
    /// # Safety
    /// `argv` must point to `argc` valid, NUL-terminated C strings, as
    /// guaranteed by the host when it invokes a [`BuiltinFn`].
    pub unsafe fn from_raw(argc: usize, argv: *const *const c_char) -> Self {
        let mut items = Vec::with_capacity(argc);
        if !argv.is_null() {
            for i in 0..argc {
                let p = *argv.add(i);
                if p.is_null() {
                    break;
                }
                items.push(CStr::from_ptr(p).to_string_lossy().into_owned());
            }
        }
        Args { items }
    }

    /// The command name (`argv[0]`), or `""` if somehow empty.
    pub fn name(&self) -> &str {
        self.items.first().map(String::as_str).unwrap_or("")
    }

    /// The positional arguments (everything after `argv[0]`).
    pub fn rest(&self) -> &[String] {
        if self.items.is_empty() {
            &[]
        } else {
            &self.items[1..]
        }
    }

    /// All of `argv`, name included.
    pub fn to_vec(&self) -> &[String] {
        &self.items
    }
}

/// Declare a plugin: its identity, the builtins it registers, and the
/// native completions it provides. Expands to the `#[no_mangle] extern "C"
/// fn znative_init` the host looks for, plus the `'static`
/// [`PluginInfo`].
///
/// * `builtins:` — each `"name" => handler` registers a command. A handler
///   is `fn(&Host, &Args) -> c_int`.
/// * `completions:` — each `"cmd" => generator` wires a native completion
///   into zsh's completion system (compsys) for command `cmd`. A generator
///   is also `fn(&Host, &Args) -> c_int`; it receives `$CURRENT` (1-based
///   index of the word being completed) followed by every word on the
///   line, and emits candidates with [`Host::add_match`]. (Requires
///   `compdef` — load the plugin after `compinit`.)
///
/// Both sections are optional.
///
/// ```ignore
/// declare_plugin! {
///     name: "greet",
///     version: "0.1.0",
///     builtins:    { "greet" => greet },
///     completions: { "greet" => greet_complete },
/// }
/// ```
#[macro_export]
macro_rules! declare_plugin {
    (
        name: $name:literal,
        version: $version:literal,
        $(builtins: { $($cmd:literal => $handler:path),+ $(,)? } $(,)?)?
        $(completions: { $($ccmd:literal => $cgen:path),+ $(,)? } $(,)?)?
        $(compfns: { $($cfname:literal => $cfhandler:path),+ $(,)? } $(,)?)?
    ) => {
        static __ZSHRS_PLUGIN_INFO: $crate::PluginInfo = $crate::PluginInfo {
            abi_version: $crate::ABIVERSION_FOR_MACRO,
            name: concat!($name, "\0").as_ptr() as *const ::std::os::raw::c_char,
            version: concat!($version, "\0").as_ptr() as *const ::std::os::raw::c_char,
        };

        #[no_mangle]
        pub extern "C" fn znative_init(
            host: *const $crate::HostApi,
        ) -> *const $crate::PluginInfo {
            if host.is_null() {
                return ::std::ptr::null();
            }
            // Verify the host speaks our ABI before touching the table.
            let ver = unsafe { (*host).abi_version };
            if ver != $crate::ABI_VERSION {
                return ::std::ptr::null();
            }
            let h = unsafe { $crate::Host::from_raw(host) };
            $($(
                {
                    // One trampoline per registered handler: adapts the
                    // C-ABI BuiltinFn to the ergonomic fn(&Host,&Args).
                    extern "C" fn __trampoline(
                        host: *const $crate::HostApi,
                        argc: usize,
                        argv: *const *const ::std::os::raw::c_char,
                    ) -> ::std::os::raw::c_int {
                        let h = unsafe { $crate::Host::from_raw(host) };
                        let a = unsafe { $crate::Args::from_raw(argc, argv) };
                        $handler(&h, &a)
                    }
                    h.register_builtin($cmd, __trampoline);
                }
            )+)?
            $($(
                {
                    // Completion generators are registered as hidden
                    // builtins, then wired into compsys via compdef.
                    extern "C" fn __cgen(
                        host: *const $crate::HostApi,
                        argc: usize,
                        argv: *const *const ::std::os::raw::c_char,
                    ) -> ::std::os::raw::c_int {
                        let h = unsafe { $crate::Host::from_raw(host) };
                        let a = unsafe { $crate::Args::from_raw(argc, argv) };
                        $cgen(&h, &a)
                    }
                    // NB: no leading underscore — zsh/compsys treats
                    // `_*` command names as autoloadable completion
                    // functions, which would shadow this builtin before
                    // plugin dispatch. Keep the prefix alphanumeric.
                    let gen_name = concat!("zshrs_complete_", $ccmd);
                    h.register_builtin(gen_name, __cgen);
                    h.install_completion($ccmd, gen_name);
                }
            )+)?
            $($(
                {
                    // One trampoline per compsys `_fn` override: adapts the
                    // C-ABI CompFn to the ergonomic fn(&Host,&Args). The
                    // host's completion router dispatches this in place of
                    // the built-in port / shell autoload for `$cfname`.
                    extern "C" fn __compfn(
                        host: *const $crate::HostApi,
                        argc: usize,
                        argv: *const *const ::std::os::raw::c_char,
                    ) -> ::std::os::raw::c_int {
                        let h = unsafe { $crate::Host::from_raw(host) };
                        let a = unsafe { $crate::Args::from_raw(argc, argv) };
                        $cfhandler(&h, &a)
                    }
                    h.register_compfn($cfname, __compfn);
                }
            )+)?
            &__ZSHRS_PLUGIN_INFO as *const $crate::PluginInfo
        }
    };
}

// The macro can't name `ABI_VERSION` inside a `const` initializer of a
// downstream crate without importing it; re-export under a stable path
// the macro hard-codes so users need only `use znative::*` or the
// two names in the doc example.
#[doc(hidden)]
pub const ABIVERSION_FOR_MACRO: u32 = ABI_VERSION;
