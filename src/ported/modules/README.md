# zshrs/src/modules

Direct ports of zsh's loadable modules from `Src/Modules/*.c`. Each
file maps 1:1 to a single C source file; the names match `zsh/<modname>`
as exposed by `zmodload`.

| Rust file          | C source          | Surface                                                                |
|--------------------|-------------------|------------------------------------------------------------------------|
| `attr.rs`          | `attr.c`          | extended-attribute builtins (`zgetattr` etc.)                          |
| `cap.rs`           | `cap.c`           | POSIX capabilities (Linux)                                             |
| `clone.rs`         | `clone.c`         | `clone(2)` syscall builtin                                             |
| `curses.rs`        | `curses.c`        | `zcurses` builtin                                                      |
| `datetime.rs`      | `datetime.c`      | `strftime`, `$EPOCHSECONDS`, `$EPOCHREALTIME`                          |
| `db_gdbm.rs`       | `db_gdbm.c`       | `ztie -d db/gdbm` GDBM-backed assoc                                    |
| `example.rs`       | `example.c`       | documentation/template module — no runtime surface                     |
| `files.rs`         | `files.c`         | builtin chmod / chown / chgrp / mv / ln / mkdir / rm / rmdir / sync    |
| `hlgroup.rs`       | `hlgroup.c`       | ZLE highlight-group SGR conversion (`.zle.esc` / `.zle.sgr`)           |
| `ksh93.rs`         | `ksh93.c`         | ksh93 emulation glue — `nameref` builtin, `${.sh.*}` specials          |
| `langinfo.rs`      | `langinfo.c`      | `${langinfo[CODESET]}` etc. via `nl_langinfo(3)`                       |
| `mapfile.rs`       | `mapfile.c`       | `${mapfile[/path]}` file-as-string assoc                               |
| `mathfunc.rs`      | `mathfunc.c`      | `sin`, `cos`, `log`, … math functions                                  |
| `nearcolor.rs`     | `nearcolor.c`     | RGB → 256-colour quantisation                                          |
| `newuser.rs`       | `newuser.c`       | first-run user-setup wizard                                            |
| `param_private.rs` | `param_private.c` | `private` builtin / `PM_PRIVATE` scope                                 |
| `parameter.rs`     | `parameter.c`     | `$functions`, `$aliases`, `$parameters`, `$dirstack`, … special arrays |
| `pcre.rs`          | `pcre.c`          | `pcre_compile` / `pcre_match` / `pcre_study`                           |
| `random.rs`        | `random.c`        | `$SRANDOM`                                                             |
| `random_real.rs`   | `random_real.c`   | `$RANDOM` 64-bit / high-precision RNG                                  |
| `regex.rs`         | `regex.c`         | `[[ … -regex-match … ]]` POSIX-regex condition                         |
| `socket.rs`        | `socket.c`        | `zsocket` Unix-domain socket                                           |
| `stat.rs`          | `stat.c`          | `zstat` builtin                                                        |
| `system.rs`        | `system.c`        | `sysopen` / `sysread` / `syswrite` / `syserror` / `zsystem`            |
| `tcp.rs`           | `tcp.c`           | `ztcp` TCP socket                                                      |
| `tcp_h.rs`         | `tcp.h`           | `tcp_session` types / `ZTCP_*` flags shared by `tcp.rs` and `zftp.rs` |
| `termcap.rs`       | `termcap.c`       | `echotc`, termcap capability access                                    |
| `terminfo.rs`      | `terminfo.c`      | `${terminfo[capname]}` via ncurses FFI                                 |
| `watch.rs`         | `watch.c`         | `$watch` login/logout monitoring                                       |
| `zftp.rs`          | `zftp.c`          | `zftp` FTP client builtin                                              |
| `zprof.rs`         | `zprof.c`         | function profiler                                                      |
| `zpty.rs`          | `zpty.c`          | `zpty` pseudo-terminal builtin                                         |
| `zselect.rs`       | `zselect.c`       | `zselect` fd-readiness builtin                                         |
| `zutil.rs`         | `zutil.c`         | `zstyle`, `zformat`, `zparseopts`, `zregexparse`                       |

## Usage

Reach for a module via `crate::modules::<modname>::…`. Backwards-compat
flat re-exports (`crate::<modname>::…`) keep older call sites resolving
during the migration; new code should use the `modules::` path.

## Porting rule

Each file's header cites the C source's exact line ranges that drove it
(e.g. `Src/Modules/terminfo.c:135` for `getterminfo()`). Faithful ports
only — no adhoc replacements, no plausible-sounding stubs. When in doubt,
read the C source first.
