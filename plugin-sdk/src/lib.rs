//! # `zshrs-plugin` — native plugin SDK for zshrs
//!
//! zshrs is the first compiled Unix shell; this crate is what makes it
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
//! use zshrs_plugin::{declare_plugin, Args, Host};
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
//! zshrs-plugin = "0.12"
//! ```
//!
//! `cargo build` produces `libhello.dylib` / `libhello.so`; then inside
//! zshrs: `zmodload -R ~/plugins/libhello.dylib` and `rhello` is a live
//! command.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

/// ABI version. Bumped on ANY change to [`HostApi`], [`PluginInfo`],
/// [`BuiltinFn`], or [`InitFn`] layout/semantics. The host refuses to
/// load a plugin whose `abi_version` does not match its own — a
/// mismatched struct layout is undefined behaviour, so this is a hard
/// gate, not a warning.
pub const ABI_VERSION: u32 = 1;

/// The one symbol every plugin `cdylib` must export. The host resolves
/// it with `dlsym` after `dlopen`. Signature is [`InitFn`].
pub const INIT_SYMBOL: &[u8] = b"zshrs_plugin_init\0";

/// A plugin-provided command handler.
///
/// * `host`   — the host API table (call back into the shell through it).
/// * `argc`   — number of elements in `argv`.
/// * `argv`   — NUL-terminated C strings; `argv[0]` is the command name,
///              `argv[1..]` the arguments. Valid only for the duration
///              of the call; copy anything you need to keep.
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
    /// plugin (in `zshrs_plugin_init` or a [`BuiltinFn`] call) and must
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
            Ok(c) => (self.t().eval)(self.api, c.as_ptr()) as i32,
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
        let s = unsafe { CStr::from_ptr(raw) }.to_string_lossy().into_owned();
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

/// Declare a plugin: its identity and the builtins it registers. Expands
/// to the `#[no_mangle] extern "C" fn zshrs_plugin_init` the host looks
/// for, plus the `'static` [`PluginInfo`]. Each handler is
/// `fn(&Host, &Args) -> c_int`.
///
/// ```ignore
/// declare_plugin! {
///     name: "hello",
///     version: "0.1.0",
///     builtins: {
///         "rhello" => hello_handler,
///         "rbye"   => bye_handler,
///     },
/// }
/// ```
#[macro_export]
macro_rules! declare_plugin {
    (
        name: $name:literal,
        version: $version:literal,
        builtins: { $($cmd:literal => $handler:path),+ $(,)? } $(,)?
    ) => {
        static __ZSHRS_PLUGIN_INFO: $crate::PluginInfo = $crate::PluginInfo {
            abi_version: $crate::ABIVERSION_FOR_MACRO,
            name: concat!($name, "\0").as_ptr() as *const ::std::os::raw::c_char,
            version: concat!($version, "\0").as_ptr() as *const ::std::os::raw::c_char,
        };

        #[no_mangle]
        pub extern "C" fn zshrs_plugin_init(
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
            $(
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
            )+
            &__ZSHRS_PLUGIN_INFO as *const $crate::PluginInfo
        }
    };
}

// The macro can't name `ABI_VERSION` inside a `const` initializer of a
// downstream crate without importing it; re-export under a stable path
// the macro hard-codes so users need only `use zshrs_plugin::*` or the
// two names in the doc example.
#[doc(hidden)]
pub const ABIVERSION_FOR_MACRO: u32 = ABI_VERSION;
