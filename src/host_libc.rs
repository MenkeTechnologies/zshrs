//! Rust-only utility (NOT a port — lives outside `src/ported/` by design).
//!
//! Where zshrs names what the HOST libc does that C zsh gets for free by
//! calling it. zsh hands printf conversions to libc `fprintf`
//! (`print_val`, c:Src/builtin.c:4571), so flag combinations the C standard
//! leaves undefined behave however the platform's libc behaves. zshrs
//! formats by hand and has to state the host's answer.

/// Whether libc `printf` pads `%c` / `%s` with ZEROS when given the `0` flag.
/// The BSD/macOS libc does (`printf %04c x` → `000x`); glibc and musl ignore
/// `0` for string conversions and pad with spaces (`   x`).
pub fn printf_zero_flag_pads_strings() -> bool {
    cfg!(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))
}
