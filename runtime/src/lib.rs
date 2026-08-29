//! Static runtime archive for zshrs AOT-compiled objects.
//!
//! This crate has no logic of its own. It exists only to give the `zsh`
//! library a `staticlib` crate-type target that is *not* built as part of
//! `cargo install zshrs` — see `runtime/Cargo.toml` for why that matters.
//!
//! A `staticlib` archive bundles the object files of this crate together with
//! those of every upstream rlib, so the `#[no_mangle] extern "C"` symbols
//! defined in `zsh` (notably `fusevm_aot_register_builtins`, gated on the
//! default-on `aot-hook` feature) land in `libzsh.a` and are resolvable by the
//! linker when `zbuild --native` links a Cranelift-emitted object against it.
//!
//! The `pub use` keeps the dependency edge live and re-exports the runtime API
//! for anything that links this archive as a C library.

pub use zsh::*;
