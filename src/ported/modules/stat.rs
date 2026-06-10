//! File stat module — port of `Src/Modules/stat.c`.
//!
//! C source has 2 enums (`statnum`, `statflags`) — both anonymous-
//! valued integer-constant tables. Rust port mirrors as constants
//! to avoid Rust-only type wrappers.
//!
//! Functions (matching C 1:1):
//!   - statmodeprint  `[c:47]`
//!   - statuidprint   `[c:132]`
//!   - statgidprint   `[c:161]`
//!   - stattimeprint  `[c:191]`
//!   - statulprint    `[c:211]`
//!   - statlinkprint  `[c:219]`
//!   - statprint      `[c:234]`
//!   - bin_stat       `[c:368]`
//!   - 6 module loaders

use crate::ported::params::{setaparam, sethparam, setsparam};
use crate::ported::utils::{zstrtol, ztrftime, zwarnnam};
use crate::ported::zsh_h::{features, module, options};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::sync::{Mutex, OnceLock};

// ============================================================
// Port of `enum statnum` from `Src/Modules/stat.c:33-35`.
// Anonymous integer constants for the per-element index passed
// to `statprint(..., iwhich, ...)`.
// ============================================================
/// Port of `HNAMEKEY` from `Src/Modules/stat.c:43`. Hash key the
/// `zstat -H` mode uses to store the file name in the result assoc.
pub const HNAMEKEY: &str = "name"; // c:43

/// Port of `statmodeprint(mode_t mode, char *outbuf, int flags)` from `Src/Modules/stat.c:47`. Renders
/// a Unix mode word per the STF_RAW / STF_OCTAL / STF_STRING flag
/// combination — raw octal/decimal, "ls -l"-style permission
/// string, or both with the raw form parenthesised.
///
/// C signature: `static void statmodeprint(mode_t mode, char *outbuf, int flags)`.
/// Rust port returns the formatted string (caller writes to its
/// own buffer) — same observable output for a given flag set.
/// WARNING: param names don't match C — Rust=(mode, flags) vs C=(mode, outbuf, flags)
pub fn statmodeprint(mode: u32, flags: i32) -> String {
    // c:47
    let mut out = String::new();
    if (flags & STF_RAW) != 0 {
        // c:50
        if (flags & STF_OCTAL) != 0 {
            // c:51
            out.push_str(&format!("0{:o}", mode));
        } else {
            out.push_str(&format!("{}", mode));
        }
        if (flags & STF_STRING) != 0 {
            // c:53
            out.push_str(" ("); // c:54
        }
    }
    if (flags & STF_STRING) != 0 {
        // c:56
        let modes = b"?rwxrwxrwx";
        let mut pm = [b'-'; 10];
        // c:84-103 — file-type char.
        let ifmt = mode & 0o170_000; // S_IFMT
        pm[0] = match ifmt {
            0o020_000 => b'c', // S_ISCHR
            0o040_000 => b'd', // S_ISDIR
            0o060_000 => b'b', // S_ISBLK
            0o100_000 => b'-', // S_ISREG
            0o120_000 => b'l', // S_ISLNK
            0o140_000 => b's', // S_ISSOCK
            0o010_000 => b'p', // S_ISFIFO
            _ => b'?',
        };
        // c:105-107 — owner/group/other rwx bits.
        let bits = [
            0o0400, 0o0200, 0o0100, 0o0040, 0o0020, 0o0010, 0o0004, 0o0002, 0o0001,
        ];
        for i in 0..9 {
            pm[i + 1] = if (mode & bits[i]) != 0 {
                modes[i + 1]
            } else {
                b'-'
            };
        }
        // c:111-115 — setuid / setgid / sticky.
        if (mode & 0o4000) != 0 {
            // S_ISUID
            pm[3] = if (mode & 0o0100) != 0 { b's' } else { b'S' };
        }
        if (mode & 0o2000) != 0 {
            // S_ISGID
            pm[6] = if (mode & 0o0010) != 0 { b's' } else { b'S' };
        }
        if (mode & 0o1000) != 0 {
            // S_ISVTX
            pm[9] = if (mode & 0o0001) != 0 { b't' } else { b'T' };
        }
        out.push_str(std::str::from_utf8(&pm).unwrap_or(""));
        if (flags & STF_RAW) != 0 {
            // c:132
            out.push(')'); // c:132
        }
    }
    out
}

/// Port of `statuidprint(uid_t uid, char *outbuf, int flags)` from `Src/Modules/stat.c:132`. Renders
/// a uid in raw form (decimal), string form (user name via
/// `getpwuid`), or both.
/// WARNING: param names don't match C — Rust=(uid, flags) vs C=(uid, outbuf, flags)
pub fn statuidprint(uid: u32, flags: i32) -> String {
    // c:132
    let mut out = String::new();
    if (flags & STF_RAW) != 0 {
        // c:135
        out.push_str(&format!("{}", uid));
        if (flags & STF_STRING) != 0 {
            // c:137
            out.push_str(" (");
        }
    }
    if (flags & STF_STRING) != 0 {
        // c:140
        let name = unsafe {
            // c:142 — `pwd = getpwuid(uid);`
            let p = libc::getpwuid(uid);
            if p.is_null() {
                String::new()
            } else {
                let nm = (*p).pw_name;
                if nm.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(nm).to_string_lossy().into_owned()
                }
            }
        };
        if name.is_empty() {
            // c:148 numeric fallback
            out.push_str(&format!("{}", uid));
        } else {
            out.push_str(&name); // c:161 pwd->pw_name
        }
        if (flags & STF_RAW) != 0 {
            // c:161
            out.push(')');
        }
    }
    out
}

/// Port of `statgidprint(gid_t gid, char *outbuf, int flags)` from `Src/Modules/stat.c:161`. Symmetric
/// with `statuidprint` for gid via `getgrgid`.
/// WARNING: param names don't match C — Rust=(gid, flags) vs C=(gid, outbuf, flags)
pub fn statgidprint(gid: u32, flags: i32) -> String {
    // c:161
    let mut out = String::new();
    if (flags & STF_RAW) != 0 {
        // c:164
        out.push_str(&format!("{}", gid));
        if (flags & STF_STRING) != 0 {
            // c:166
            out.push_str(" (");
        }
    }
    if (flags & STF_STRING) != 0 {
        // c:169
        let name = unsafe {
            let g = libc::getgrgid(gid); // c:171
            if g.is_null() {
                String::new()
            } else {
                let nm = (*g).gr_name;
                if nm.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(nm).to_string_lossy().into_owned()
                }
            }
        };
        if name.is_empty() {
            out.push_str(&format!("{}", gid)); // c:184
        } else {
            out.push_str(&name); // c:178
        }
        if (flags & STF_RAW) != 0 {
            // c:191
            out.push(')');
        }
    }
    out
}

/// Port of `static char *timefmt;` from `Src/Modules/stat.c:187`. C uses a
/// module-static global initialized to the ctime-like default at the top of
/// `bin_stat` (c:376) and overwritten by `-F FMT`. Rust mirrors with a
/// `Mutex<String>` so `stattimeprint` (c:201) can read the same global.
/// Default constant lives next to the static; callers read/write the lock
/// directly (no accessor helpers — those would be Rust-only fns).
const TIMEFMT_DEFAULT: &str = "%a %b %e %k:%M:%S %Z %Y"; // c:376

static TIMEFMT: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();

/// Port of `stattimeprint(time_t tim, long nsecs, char *outbuf, int flags)` from `Src/Modules/stat.c:191`. Renders
/// a Unix timestamp + nsec offset: raw form is integer seconds;
/// string form is `ctime(3)` (or strftime via the timefmt global).
/// WARNING: param names don't match C — Rust=(tim, _nsecs, flags) vs C=(tim, nsecs, outbuf, flags)
pub fn stattimeprint(tim: i64, _nsecs: i64, flags: i32) -> String {
    // c:191
    let mut out = String::new();
    if (flags & STF_RAW) != 0 {
        // c:194
        out.push_str(&format!("{}", tim));
        if (flags & STF_STRING) != 0 {
            // c:196
            out.push_str(" (");
        }
    }
    if (flags & STF_STRING) != 0 {
        // c:199
        // c:201 — `ztrftime(oend, 40, timefmt,
        //     (flags & STF_GMT) ? gmtime(&tim) : localtime(&tim), nsecs);`
        // C reads the module-static `timefmt` here (initialized to the
        // ctime default at bin_stat entry, possibly overwritten by -F).
        // The GMT vs local choice comes from the STF_GMT flag (`stat -g`).
        let st = std::time::UNIX_EPOCH + std::time::Duration::from_secs(tim.max(0) as u64);
        let fmt: String = TIMEFMT
            .get_or_init(|| std::sync::Mutex::new(TIMEFMT_DEFAULT.to_string()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| TIMEFMT_DEFAULT.to_string());
        let use_gmt = (flags & STF_GMT) != 0; // c:201 — picks gmtime(&tim)
        let formatted = ztrftime(&fmt, st, use_gmt);
        out.push_str(&formatted);
        if (flags & STF_RAW) != 0 {
            // c:211
            out.push(')');
        }
    }
    out
}

/// Port of `statulprint(unsigned long num, char *outbuf)` from `Src/Modules/stat.c:211`. Renders an
/// unsigned-long stat field as decimal (always raw, no STF_STRING
/// branch).
/// WARNING: param names don't match C — Rust=(num) vs C=(num, outbuf)
pub fn statulprint(num: u64) -> String {
    // c:211
    format!("{}", num) // c:219
}

/// Port of `statlinkprint(struct stat *sbuf, char *outbuf, char *fname)` from `Src/Modules/stat.c:219`. For
/// symlinks, renders the link target via `readlink(2)`; otherwise
/// returns empty.
/// WARNING: param names don't match C — Rust=(sbuf_mode, fname) vs C=(sbuf, outbuf, fname)
pub fn statlinkprint(sbuf_mode: u32, fname: &str) -> String {
    // c:219
    if (sbuf_mode & 0o170_000) != 0o120_000 {
        // c:219 S_ISLNK
        return String::new();
    }
    fs::read_link(fname) // c:226 readlink
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Port of `statprint(struct stat *sbuf, char *outbuf, char *fname, int iwhich, int flags)` from `Src/Modules/stat.c:234`. The unified
/// per-field dispatcher: given a stat metadata, file name, the
/// `iwhich` index from `STATELTS`, and a flag word, produce the
/// formatted value string.
///
/// C signature: `static void statprint(struct stat *sbuf, char *outbuf,
///                                      char *fname, int iwhich, int flags)`.
/// WARNING: param names don't match C — Rust=(meta, fname, iwhich, flags) vs C=(sbuf, outbuf, fname, iwhich, flags)
pub fn statprint(meta: &fs::Metadata, fname: &str, iwhich: i32, flags: i32) -> String {
    // c:234
    // c:234-241 — `if (flags & STF_NAME)` prefix with `name<space>`.
    // `%-8s` left-justifies the name to 8 chars when not PICK/ARRAY,
    // `%s ` otherwise.
    let name_prefix = if (flags & STF_NAME) != 0 {
        let n = STATELTS.get(iwhich as usize).copied().unwrap_or("");
        if (flags & (STF_PICK | STF_ARRAY)) != 0 {
            format!("{} ", n) // c:239
        } else {
            format!("{:<8}", n) // c:240
        }
    } else {
        String::new()
    };
    let val = match iwhich {
        ST_DEV => format!("{}", meta.dev()),              // c:240
        ST_INO => format!("{}", meta.ino()),              // c:241
        ST_MODE => statmodeprint(meta.mode(), flags),     // c:242
        ST_NLINK => format!("{}", meta.nlink()),          // c:243
        ST_UID => statuidprint(meta.uid(), flags),        // c:244
        ST_GID => statgidprint(meta.gid(), flags),        // c:245
        ST_RDEV => format!("{}", meta.rdev()),            // c:246
        ST_SIZE => statulprint(meta.size()),              // c:247
        ST_ATIM => stattimeprint(meta.atime(), 0, flags), // c:248
        ST_MTIM => stattimeprint(meta.mtime(), 0, flags), // c:249
        ST_CTIM => stattimeprint(meta.ctime(), 0, flags), // c:250
        ST_BLKSIZE => statulprint(meta.blksize()),        // c:251
        ST_BLOCKS => statulprint(meta.blocks()),          // c:252
        ST_READLINK => statlinkprint(meta.mode(), fname), // c:253
        _ => String::new(),
    };
    format!("{}{}", name_prefix, val)
}

/// Port of `bin_stat(char *name, char **args, Options ops, UNUSED(int func))` from `Src/Modules/stat.c:368`. The `zstat`
/// builtin entry. Parses the `+ELEMENT` / `-flag` / `-A NAME` /
/// `-H NAME` / `-f FD` / `-F FORMAT` arg syntax, then calls
/// `lstat`/`stat`/`fstat` per file, dispatching `statprint` for
/// each requested element.
///
/// C signature: `static int bin_stat(char *name, char **args,
///                                    Options ops, int func)`.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(name, args, ops, func)
pub fn bin_stat(
    nam: &str,
    args: &[String], // c:368
    _ops_unused: &options,
    _func: i32,
) -> i32 {
    // c:370-374 — locals.
    let mut iwhich: i32 = -1; // c:373
    let mut flags: i32 = 0;
    let mut found = 0i32; // c:375
    let mut arrnam: Option<String> = None;
    let mut hashnam: Option<String> = None;
    let mut fd: i32 = 0;
    // c:376 — `timefmt = "%a %b %e %k:%M:%S %Z %Y";`. Reset the module-
    // static every entry so a prior `-F` doesn't leak into a fresh
    // invocation without `-F`.
    if let Ok(mut g) = TIMEFMT
        .get_or_init(|| std::sync::Mutex::new(TIMEFMT_DEFAULT.to_string()))
        .lock()
    {
        g.clear();
        g.push_str(TIMEFMT_DEFAULT);
    }
    // The C `Options ops` bitmap is parsed inline by this fn (the
    // BUILTIN spec at c:637 is `NULL`, so the framework doesn't pre-
    // parse). Per PORT_CHECKLIST.md rule 3 we keep `ops` as a local
    // 256-entry bitmap rather than introducing a Rust-only struct.
    let mut ops = [false; 256];
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut argv: Vec<&str> = Vec::with_capacity(args.len());
    let mut i = 0;
    // c:381 — arg loop.
    while i < args.len() && (args[i].starts_with('+') || args[i].starts_with('-')) {
        let arg = &args[i][1..];
        if arg.is_empty() || arg.starts_with('-') || arg.starts_with('+') {
            i += 1;
            break; // c:386
        }
        if args[i].starts_with('+') {
            // c:389
            if found != 0 {
                break;
            }
            for (idx, name) in STATELTS.iter().enumerate() {
                // c:392
                if name.starts_with(arg) {
                    found += 1;
                    iwhich = idx as i32;
                }
            }
            if found > 1 {
                // c:397
                zwarnnam(nam, &format!("{}: ambiguous stat element", arg));
                return 1;
            } else if found == 0 {
                // c:400
                zwarnnam(nam, &format!("{}: no such stat element", arg));
                return 1;
            }
            // c:404 — `if (iwhich == ST_READLINK) ops->ind['L'] = 1;`
            if iwhich == ST_READLINK {
                ops[b'L' as usize] = true;
            }
            flags |= STF_PICK; // c:406
        } else {
            // c:407 - flag arm
            for ch in arg.chars() {
                match ch {
                    'g' | 'l' | 'L' | 'n' | 'N' | 'o' | 'r' | 's' | 't' | 'T' => {
                        ops[ch as u8 as usize] = true; // c:411
                    }
                    'A' => {
                        // c:412 — array name follows.
                        i += 1;
                        if i >= args.len() {
                            zwarnnam(nam, "missing parameter name");
                            return 1;
                        }
                        arrnam = Some(args[i].to_string());
                        flags |= STF_ARRAY;
                        break;
                    }
                    'H' => {
                        i += 1;
                        if i >= args.len() {
                            zwarnnam(nam, "missing parameter name");
                            return 1;
                        }
                        hashnam = Some(args[i].to_string());
                        flags |= STF_HASH;
                        break;
                    }
                    'f' => {
                        ops[b'f' as usize] = true;
                        i += 1;
                        if i >= args.len() {
                            zwarnnam(nam, "missing file descriptor");
                            return 1;
                        }
                        let (val, endptr) = zstrtol(args[i], 10);
                        if !endptr.is_empty() {
                            zwarnnam(nam, "bad file descriptor");
                            return 1;
                        }
                        fd = val as i32;
                        break;
                    }
                    'F' => {
                        // c:442-451 — `-F FMT`. If the same arg has chars
                        // after 'F' (e.g. `-F%Y`), use those; else consume
                        // the next argv entry. Force STF_STRING via -s so
                        // the format actually gets used (c:449-450).
                        let inline: &str = &arg[arg.find('F').unwrap() + 1..];
                        let fmt: &str = if !inline.is_empty() {
                            // c:443-444
                            inline
                        } else {
                            i += 1;
                            if i >= args.len() {
                                // c:446
                                zwarnnam(nam, "missing time format");
                                return 1;
                            }
                            args[i]
                        };
                        // c:444 — `timefmt = arg+1;` / c:445 — `timefmt = *++args;`
                        if let Ok(mut g) = TIMEFMT
                            .get_or_init(|| std::sync::Mutex::new(TIMEFMT_DEFAULT.to_string()))
                            .lock()
                        {
                            g.clear();
                            g.push_str(fmt);
                        }
                        ops[b's' as usize] = true; // c:450 — force STF_STRING.
                        break;
                    }
                    _ => {
                        zwarnnam(nam, &format!("bad option: -{}", ch));
                        return 1;
                    }
                }
            }
        }
        i += 1;
    }
    while i < args.len() {
        argv.push(args[i]);
        i += 1;
    }
    let _ = fd;

    if (flags & STF_ARRAY) != 0 && (flags & STF_HASH) != 0 {
        // c:459
        zwarnnam(nam, "both array and hash requested");
        return 1;
    }

    if ops[b'l' as usize] {
        // c:467
        // List elements + return.
        if let Some(ref name) = arrnam {
            // c:469
            // c:472 — `setaparam(arrnam, names);` — array of element names.
            let joined: Vec<&str> = STATELTS.iter().copied().collect();
            setsparam(name, &joined.join(":"));
        } else {
            let joined: Vec<&str> = STATELTS.iter().copied().collect();
            println!("{}", joined.join(" ")); // c:478 putchar
        }
        return 0; // c:489
    }

    if argv.is_empty() && !ops[b'f' as usize] {
        // c:491
        zwarnnam(nam, "no files given");
        return 1;
    } else if !argv.is_empty() && ops[b'f' as usize] {
        // c:493
        zwarnnam(nam, "no files allowed with -f");
        return 1;
    }

    // c:496+ — per-file stat + dispatch loop.
    let use_lstat = ops[b'L' as usize];
    let mut hash_out: Vec<(String, String)> = Vec::new();
    let mut array_out: Vec<String> = Vec::new();
    let show_type = ops[b't' as usize]; // c: -t
    let mut local_flags = flags;
    // c:513 — `if (OPT_ISSET(ops,'s') || !OPT_ISSET(ops,'r'))` STF_STRING.
    if ops[b's' as usize] {
        local_flags |= STF_STRING;
    } // c:514
      // c:516 — `if (OPT_ISSET(ops,'r') || !OPT_ISSET(ops,'s'))` STF_RAW.
    if ops[b'r' as usize] || !ops[b's' as usize] {
        // c:516
        local_flags |= STF_RAW;
    }
    // c:518-519 — `-n` → STF_FILE (filename prefix).
    if ops[b'n' as usize] {
        local_flags |= STF_FILE;
    } // c:519
      // c:520-521 — `-o` → STF_OCTAL.
    if ops[b'o' as usize] {
        local_flags |= STF_OCTAL;
    } // c:521
      // c:522-523 — `-t` → STF_NAME explicit.
    if ops[b't' as usize] {
        local_flags |= STF_NAME;
    } // c:523
      // c:525-530 — default STF_NAME when neither -A nor -H and no
      // single-element pick: every line gets a `name<sp>` prefix so
      // `zstat /etc/hosts` looks like `mode    33188` etc.
    if arrnam.is_none() && hashnam.is_none() {
        if argv.len() > 1 {
            local_flags |= STF_FILE;
        } // c:527
        if (local_flags & STF_PICK) == 0 {
            // c:528
            local_flags |= STF_NAME; // c:529
        }
    }
    // c:532-535 — explicit -N / -f turn off STF_FILE; -T / -H turn off
    // STF_NAME (suppress prefix for `read` / hash use).
    if ops[b'N' as usize] || ops[b'f' as usize] {
        // c:532
        local_flags &= !STF_FILE;
    }
    if ops[b'T' as usize] || ops[b'H' as usize] {
        // c:534
        local_flags &= !STF_NAME;
    }
    let _ = show_type;

    // c:Src/Modules/stat.c:bin_stat — C tracks per-file stat(2) failures
    // and propagates a non-zero rc when any path errored (`ret = 1` at
    // c:560-565 inside the per-path loop). The Rust port previously
    // skipped the path on error via `continue` but unconditionally
    // returned 0 at the end, masking ENOENT (and any other stat error)
    // as success. Track the first failure rc and return it after the
    // loop.
    let mut rc: i32 = 0;
    for path in &argv {
        let meta = if use_lstat {
            fs::symlink_metadata(path)
        } else {
            fs::metadata(path)
        };
        let meta = match meta {
            Ok(m) => m,
            Err(e) => {
                // Bug #112 — strip Rust's " (os error N)" suffix by
                // routing the errno through the canonical strerror
                // port (Src/compat.c:194).
                let msg = crate::ported::compat::strerror(e.raw_os_error().unwrap_or(0));
                zwarnnam(nam, &format!("{}: {}", path, msg));
                rc = 1;
                continue;
            }
        };

        // c:573-581 — `STF_FILE` prefix the filename per file.
        if (local_flags & STF_FILE) != 0 && arrnam.is_none() && hashnam.is_none() {
            if (local_flags & STF_PICK) != 0 {
                print!("{} ", path); // c:580
            } else {
                println!("{}:", path);
            }
        }
        if iwhich >= 0 {
            // -E single element.
            let val = statprint(&meta, path, iwhich, local_flags);
            if let Some(ref aname) = arrnam {
                array_out.push(val);
                let _ = aname;
            } else if let Some(ref hname) = hashnam {
                hash_out.push((STATELTS[iwhich as usize].to_string(), val));
                let _ = hname;
            } else {
                println!("{}", val); // c:591
            }
        } else {
            // All elements.
            for idx in 0..STATELTS.len() {
                let val = statprint(&meta, path, idx as i32, local_flags);
                if let Some(_) = &arrnam {
                    array_out.push(val);
                } else if let Some(_) = &hashnam {
                    hash_out.push((STATELTS[idx].to_string(), val));
                } else {
                    println!("{}", val); // c:603
                }
            }
        }
    }

    if let Some(name) = arrnam {
        // c:setaparam
        // c — `setaparam(name, zarrdup(array_out));` — real indexed array.
        setaparam(&name, array_out); // c:params.c:3595
    }
    if let Some(name) = hashnam {
        // c:sethparam
        // c — `sethparam(name, ...);` — real assoc array. Flatten
        // hash_out into alternating [k,v,k,v,...].
        let mut flat: Vec<String> = Vec::with_capacity(hash_out.len() * 2);
        for (k, v) in hash_out {
            flat.push(k);
            flat.push(v);
        }
        sethparam(&name, flat); // c:params.c:3602
    }
    rc
}

// `bintab` — port of `static struct builtin bintab[]` (stat.c:638).

// `module_features` — port of `static struct features module_features`
// from stat.c:642.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/stat.c:651`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:651
    // C body c:653-654 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/stat.c:658`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:658
    *features = featuresarray(m, module_features());
    0 // c:673
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/stat.c:666`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:666
    handlefeatures(m, module_features(), enables) // c:673
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/stat.c:673`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:673
    // C body c:675-676 — `return 0`. Faithful empty-body port; the
    //                    zstat builtin registers via the bn_list dispatch.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/stat.c:680`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:680
    setfeatureenables(m, module_features(), None) // c:687
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/stat.c:687`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:687
    // C body c:689-690 — `return 0`. Faithful empty-body port; the
    //                    zstat builtin unregisters via cleanup_'s setfeatureenables.
    0
}
/// `ST_DEV` constant.
pub const ST_DEV: i32 = 0; // c:33
/// `ST_INO` constant.
pub const ST_INO: i32 = 1;
/// `ST_MODE` constant.
pub const ST_MODE: i32 = 2;
/// `ST_NLINK` constant.
pub const ST_NLINK: i32 = 3;
/// `ST_UID` constant.
pub const ST_UID: i32 = 4;
/// `ST_GID` constant.
pub const ST_GID: i32 = 5;
/// `ST_RDEV` constant.
pub const ST_RDEV: i32 = 6;
/// `ST_SIZE` constant.
pub const ST_SIZE: i32 = 7;
/// `ST_ATIM` constant.
pub const ST_ATIM: i32 = 8;
/// `ST_MTIM` constant.
pub const ST_MTIM: i32 = 9;
/// `ST_CTIM` constant.
pub const ST_CTIM: i32 = 10;
/// `ST_BLKSIZE` constant.
pub const ST_BLKSIZE: i32 = 11;
/// `ST_BLOCKS` constant.
pub const ST_BLOCKS: i32 = 12;
/// `ST_READLINK` constant.
pub const ST_READLINK: i32 = 13;
/// `ST_COUNT` constant.
pub const ST_COUNT: i32 = 14; // c:34

// ============================================================
// Port of `enum statflags` from `Src/Modules/stat.c:36-38`.
// Bitmask flags passed to the print ported + bin_stat dispatch.
// ============================================================
/// `STF_NAME` constant.
pub const STF_NAME: i32 = 1; // c:36
/// `STF_FILE` constant.
pub const STF_FILE: i32 = 2;
/// `STF_STRING` constant.
pub const STF_STRING: i32 = 4;
/// `STF_RAW` constant.
pub const STF_RAW: i32 = 8;

// =====================================================================
// static struct builtin bintab[]                                    c:638
// static struct features module_features                            c:642
// =====================================================================
/// `STF_PICK` constant.
pub const STF_PICK: i32 = 16;
/// `STF_ARRAY` constant.
pub const STF_ARRAY: i32 = 32;
/// `STF_GMT` constant.
pub const STF_GMT: i32 = 64;
/// `STF_HASH` constant.
pub const STF_HASH: i32 = 128;
/// `STF_OCTAL` constant.
pub const STF_OCTAL: i32 = 256; // c:38

/// Port of `statelts[]` from `Src/Modules/stat.c:39`. Names of the
/// 14 stat-elements, indexed by the `ST_*` constants above.
pub static STATELTS: &[&str] = &[
    // c:39
    "device", "inode", "mode", "nlink", "uid", "gid", "rdev", "size", "atime", "mtime", "ctime",
    "blksize", "blocks", "link",
];

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN STAT.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["b:stat".to_string(), "b:zstat".to_string()]
}

// WARNING: NOT IN STAT.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 2]);
    }
    0
}

// WARNING: NOT IN STAT.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN STAT.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 2,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 0,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn statelts_count_matches_st_count() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(STATELTS.len() as i32, ST_COUNT);
    }

    #[test]
    fn statmodeprint_octal_only() {
        let _g = crate::test_util::global_state_lock();
        let s = statmodeprint(0o100644, STF_RAW | STF_OCTAL);
        assert!(s.starts_with('0'));
        assert!(s.contains("644"));
    }

    #[test]
    fn statmodeprint_string_only() {
        let _g = crate::test_util::global_state_lock();
        let s = statmodeprint(0o100644, STF_STRING);
        assert_eq!(s.len(), 10);
        assert_eq!(&s[..1], "-");
    }

    #[test]
    fn statmodeprint_directory() {
        let _g = crate::test_util::global_state_lock();
        let s = statmodeprint(0o040755, STF_STRING);
        assert_eq!(&s[..1], "d");
    }

    #[test]
    fn statulprint_decimal() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(statulprint(12345), "12345");
    }

    #[test]
    fn statprint_size_via_index() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("x.txt");
        File::create(&path).unwrap().write_all(b"hello").unwrap();
        let meta = fs::metadata(&path).unwrap();
        let s = statprint(&meta, path.to_str().unwrap(), ST_SIZE, 0);
        assert_eq!(s, "5");
    }

    /// c:47 — `statmodeprint` for the BLOCK device file type. The
    /// `pm[0]` lookup at c:84-103 must yield 'b'. Pin all file-type
    /// chars so a regen that swaps `0o060_000` and `0o020_000`
    /// silently renders block devices as char devices.
    #[test]
    fn statmodeprint_file_type_chars_match_ls_output() {
        let _g = crate::test_util::global_state_lock();
        // S_IFREG (regular file)
        assert_eq!(&statmodeprint(0o100_644, STF_STRING)[..1], "-");
        // S_IFDIR
        assert_eq!(&statmodeprint(0o040_755, STF_STRING)[..1], "d");
        // S_IFLNK (symlink)
        assert_eq!(&statmodeprint(0o120_777, STF_STRING)[..1], "l");
        // S_IFCHR (char device)
        assert_eq!(&statmodeprint(0o020_644, STF_STRING)[..1], "c");
        // S_IFBLK (block device)
        assert_eq!(&statmodeprint(0o060_644, STF_STRING)[..1], "b");
        // S_IFIFO (named pipe)
        assert_eq!(&statmodeprint(0o010_644, STF_STRING)[..1], "p");
        // S_IFSOCK
        assert_eq!(&statmodeprint(0o140_644, STF_STRING)[..1], "s");
    }

    /// c:111-115 — setuid bit renders as 's' in the user-execute
    /// slot when execute is set, 'S' when not. Pin both polarities
    /// for setuid, setgid, and sticky so a regen flipping the
    /// uppercase/lowercase dispatch gets caught.
    #[test]
    fn statmodeprint_setuid_setgid_sticky_render_correctly() {
        let _g = crate::test_util::global_state_lock();
        // 4755 = setuid + executable → 's' in user slot
        let s = statmodeprint(0o104_755, STF_STRING);
        assert_eq!(
            s.chars().nth(3),
            Some('s'),
            "setuid+x must render as 's' in user-execute slot"
        );

        // 4644 = setuid + NOT executable → 'S' in user slot
        let s = statmodeprint(0o104_644, STF_STRING);
        assert_eq!(
            s.chars().nth(3),
            Some('S'),
            "setuid without x must render as 'S' (uppercase)"
        );

        // 2755 = setgid + group-x → 's' in group-execute slot
        let s = statmodeprint(0o102_755, STF_STRING);
        assert_eq!(
            s.chars().nth(6),
            Some('s'),
            "setgid+gx must render as 's' in group-execute slot"
        );

        // 2644 = setgid without group-x → 'S'
        let s = statmodeprint(0o102_644, STF_STRING);
        assert_eq!(
            s.chars().nth(6),
            Some('S'),
            "setgid without gx must render as 'S'"
        );

        // 1755 = sticky + world-x → 't' in other-execute slot
        let s = statmodeprint(0o101_755, STF_STRING);
        assert_eq!(
            s.chars().nth(9),
            Some('t'),
            "sticky+ox must render as 't' in other-execute slot"
        );

        // 1644 = sticky without world-x → 'T'
        let s = statmodeprint(0o101_644, STF_STRING);
        assert_eq!(
            s.chars().nth(9),
            Some('T'),
            "sticky without ox must render as 'T'"
        );
    }

    /// c:47-93 — `STF_RAW | STF_STRING` produces "raw (string)" with
    /// the raw form OUTSIDE parens and the string form inside.
    /// Pin the format because user scripts grep for "^(0?[0-9]+) \("
    /// to split the two halves.
    #[test]
    fn statmodeprint_raw_and_string_renders_with_parens() {
        let _g = crate::test_util::global_state_lock();
        let s = statmodeprint(0o100_644, STF_RAW | STF_STRING);
        // Decimal raw form, space, paren, 10-char string, close paren
        assert!(s.contains(" ("), "missing ' (' separator: {}", s);
        assert!(s.ends_with(')'), "missing closing ')': {}", s);
        // The closing paren must come right after the 10-char ls form
        let open = s.find('(').unwrap();
        let close = s.rfind(')').unwrap();
        assert_eq!(
            close - open - 1,
            10,
            "expected 10-char ls-mode between parens, got: {:?}",
            &s[open + 1..close]
        );
    }

    /// c:47-93 — `statmodeprint(0)` with STF_STRING renders all dashes
    /// after the file-type indicator. Edge case: zero permissions on
    /// a "no file-type" mode falls through to '?'.
    #[test]
    fn statmodeprint_zero_mode_renders_unknown_type_no_perms() {
        let _g = crate::test_util::global_state_lock();
        let s = statmodeprint(0, STF_STRING);
        assert_eq!(s.len(), 10);
        assert_eq!(&s[..1], "?", "mode with no S_IFMT bits → unknown");
        assert_eq!(&s[1..], "---------", "no permission bits → all dashes");
    }

    /// c:132 — `statuidprint` raw form is just the decimal uid;
    /// pin the no-leading-zeros, no-prefix shape so a regen that
    /// renders octal silently breaks `${(t)f[uid]}`.
    #[test]
    fn statuidprint_raw_is_decimal() {
        let _g = crate::test_util::global_state_lock();
        let s = statuidprint(1000, STF_RAW);
        assert_eq!(s, "1000");
    }

    /// c:132 — `statuidprint` for uid 0 must include "root" in the
    /// string form (every Unix has uid 0 = root). Pin the well-known
    /// case so a regen that breaks the getpwuid path doesn't silently
    /// fall back to numeric.
    #[test]
    fn statuidprint_uid_zero_resolves_to_root() {
        let _g = crate::test_util::global_state_lock();
        let s = statuidprint(0, STF_STRING);
        // Some hardened systems map uid 0 to a different name, but
        // it MUST resolve to non-numeric.
        assert!(
            !s.parse::<u32>().is_ok(),
            "uid 0 fell back to numeric form: {}",
            s
        );
        assert!(!s.is_empty());
    }

    /// c:161 — `statgidprint` raw form is decimal.
    #[test]
    fn statgidprint_raw_is_decimal() {
        let _g = crate::test_util::global_state_lock();
        let s = statgidprint(100, STF_RAW);
        assert_eq!(s, "100");
    }

    /// c:211 — `statulprint` for zero must render "0". A regression
    /// that prints "" or "0x0" silently breaks numeric script
    /// comparisons.
    #[test]
    fn statulprint_zero_renders_as_zero_digit() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(statulprint(0), "0");
    }

    /// c:211 — `statulprint` for u64::MAX renders the full decimal
    /// digit string. Pin the no-overflow / no-truncation behavior.
    #[test]
    fn statulprint_u64_max_renders_full_digits() {
        let _g = crate::test_util::global_state_lock();
        let s = statulprint(u64::MAX);
        assert_eq!(s, "18446744073709551615");
    }

    /// c:36-38 — STF_* flag values must each occupy a unique single
    /// bit, AND must not overlap with each other (so they can be
    /// OR'd together). Pin the bit-distinctness because the flags
    /// are AND-tested individually throughout statprint.
    #[test]
    fn stf_flag_values_are_distinct_single_bits() {
        let _g = crate::test_util::global_state_lock();
        for f in [
            STF_NAME, STF_FILE, STF_STRING, STF_RAW, STF_PICK, STF_ARRAY, STF_GMT, STF_HASH,
            STF_OCTAL,
        ] {
            assert!(f > 0, "STF_* flag {} must be positive", f);
            assert_eq!(
                (f as u32).count_ones(),
                1,
                "STF_* flag {} = 0b{:b} must be a single bit",
                f,
                f
            );
        }
        // Pairwise: no two flags share a bit
        let flags = [
            STF_NAME, STF_FILE, STF_STRING, STF_RAW, STF_PICK, STF_ARRAY, STF_GMT, STF_HASH,
            STF_OCTAL,
        ];
        for (i, &a) in flags.iter().enumerate() {
            for &b in &flags[i + 1..] {
                assert_eq!(a & b, 0, "STF flags {} and {} overlap", a, b);
            }
        }
    }

    /// c:651-690 — module-lifecycle stubs all return 0 in C.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    // ─── zsh-corpus pins for statmodeprint ─────────────────────────

    /// Regular file 0644 → "-rw-r--r--" string form.
    #[test]
    fn stat_corpus_modeprint_regular_644() {
        let mode = 0o100_644; // S_IFREG | 0644
        let r = statmodeprint(mode, STF_STRING);
        assert_eq!(r, "-rw-r--r--", "regular 0644 → -rw-r--r--, got {r:?}");
    }

    /// Regular file 0755 → "-rwxr-xr-x".
    #[test]
    fn stat_corpus_modeprint_regular_755() {
        let mode = 0o100_755;
        let r = statmodeprint(mode, STF_STRING);
        assert_eq!(r, "-rwxr-xr-x");
    }

    /// Directory 0755 → "drwxr-xr-x".
    #[test]
    fn stat_corpus_modeprint_directory() {
        let mode = 0o040_755; // S_IFDIR | 0755
        let r = statmodeprint(mode, STF_STRING);
        assert_eq!(r, "drwxr-xr-x");
    }

    /// Symlink → 'l' file-type prefix.
    #[test]
    fn stat_corpus_modeprint_symlink_prefix() {
        let mode = 0o120_777; // S_IFLNK | 0777
        let r = statmodeprint(mode, STF_STRING);
        assert!(
            r.starts_with('l'),
            "symlink mode starts with 'l', got {r:?}"
        );
    }

    /// FIFO → 'p' file-type prefix.
    #[test]
    fn stat_corpus_modeprint_fifo_prefix() {
        let mode = 0o010_644;
        let r = statmodeprint(mode, STF_STRING);
        assert!(r.starts_with('p'), "FIFO mode starts with 'p', got {r:?}");
    }

    /// Socket → 's' file-type prefix.
    #[test]
    fn stat_corpus_modeprint_socket_prefix() {
        let mode = 0o140_644;
        let r = statmodeprint(mode, STF_STRING);
        assert!(r.starts_with('s'));
    }

    /// `STF_RAW | STF_OCTAL` → numeric octal form.
    #[test]
    fn stat_corpus_modeprint_raw_octal() {
        let mode = 0o100_644;
        let r = statmodeprint(mode, STF_RAW | STF_OCTAL);
        assert!(
            r.starts_with('0'),
            "octal raw form starts with leading 0, got {r:?}"
        );
        assert!(
            r.contains("100644") || r.contains("644"),
            "octal repr contains 100644 or 644, got {r:?}"
        );
    }

    /// `STF_RAW` alone → decimal numeric.
    #[test]
    fn stat_corpus_modeprint_raw_decimal() {
        let mode = 0o100_644;
        let r = statmodeprint(mode, STF_RAW);
        // Decimal form of 0o100644 = 33188.
        assert_eq!(r, "33188", "raw decimal form, got {r:?}");
    }

    /// Empty flags → empty output.
    #[test]
    fn stat_corpus_modeprint_no_flags_empty() {
        let r = statmodeprint(0o100_644, 0);
        assert_eq!(r, "", "no flags → empty output");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/stat.c statmodeprint
    // file-type + rwx bit dispatch.
    // ═══════════════════════════════════════════════════════════════════

    /// c:84-103 — regular file (S_IFREG=0o100000) → '-' first char.
    #[test]
    fn statmodeprint_regular_file_starts_with_dash() {
        let r = statmodeprint(0o100_644, STF_STRING);
        assert!(r.starts_with('-'), "regular file → '-', got {:?}", r);
    }

    /// c:84-103 — directory (S_IFDIR=0o040000) → 'd' first char.
    #[test]
    fn statmodeprint_directory_starts_with_d() {
        let r = statmodeprint(0o040_755, STF_STRING);
        assert!(r.starts_with('d'), "directory → 'd', got {:?}", r);
    }

    /// c:84-103 — symlink (S_IFLNK=0o120000) → 'l' first char.
    #[test]
    fn statmodeprint_symlink_starts_with_l() {
        let r = statmodeprint(0o120_777, STF_STRING);
        assert!(r.starts_with('l'), "symlink → 'l', got {:?}", r);
    }

    /// c:84-103 — char device (S_IFCHR=0o020000) → 'c' first char.
    #[test]
    fn statmodeprint_char_device_starts_with_c() {
        let r = statmodeprint(0o020_644, STF_STRING);
        assert!(r.starts_with('c'), "char device → 'c', got {:?}", r);
    }

    /// c:84-103 — block device (S_IFBLK=0o060000) → 'b' first char.
    #[test]
    fn statmodeprint_block_device_starts_with_b() {
        let r = statmodeprint(0o060_644, STF_STRING);
        assert!(r.starts_with('b'), "block device → 'b', got {:?}", r);
    }

    /// c:84-103 — socket (S_IFSOCK=0o140000) → 's' first char.
    #[test]
    fn statmodeprint_socket_starts_with_s() {
        let r = statmodeprint(0o140_777, STF_STRING);
        assert!(r.starts_with('s'), "socket → 's', got {:?}", r);
    }

    /// c:84-103 — FIFO (S_IFIFO=0o010000) → 'p' first char.
    #[test]
    fn statmodeprint_fifo_starts_with_p() {
        let r = statmodeprint(0o010_644, STF_STRING);
        assert!(r.starts_with('p'), "FIFO → 'p', got {:?}", r);
    }

    /// c:84-103 — unknown ifmt → '?' fallback.
    #[test]
    fn statmodeprint_unknown_ifmt_returns_question_mark() {
        let r = statmodeprint(0o070_000, STF_STRING);
        assert!(r.starts_with('?'), "unknown ifmt → '?', got {:?}", r);
    }

    /// c:111 — setuid sticky 's' when execute bit set on user.
    #[test]
    fn statmodeprint_setuid_with_exec_shows_lowercase_s() {
        // 0o4755 = setuid + rwxr-xr-x
        let r = statmodeprint(0o104_755, STF_STRING);
        // Position 3 (user exec) should be 's' (setuid + x).
        assert_eq!(r.as_bytes()[3], b's', "setuid+x → 's', got {:?}", r);
    }

    /// c:111 — setuid uppercase 'S' when execute bit NOT set on user.
    #[test]
    fn statmodeprint_setuid_without_exec_shows_uppercase_S() {
        // 0o4644 = setuid + rw-r--r--
        let r = statmodeprint(0o104_644, STF_STRING);
        assert_eq!(r.as_bytes()[3], b'S', "setuid-no-x → 'S', got {:?}", r);
    }

    /// c:115 — sticky bit 't' when other-exec set, 'T' when not.
    #[test]
    fn statmodeprint_sticky_bit_dispatch() {
        // 0o1777 = sticky + rwxrwxrwx
        let r1 = statmodeprint(0o101_777, STF_STRING);
        assert_eq!(r1.as_bytes()[9], b't', "sticky+other-x → 't'");
        // 0o1644 = sticky + rw-r--r--
        let r2 = statmodeprint(0o101_644, STF_STRING);
        assert_eq!(r2.as_bytes()[9], b'T', "sticky no other-x → 'T'");
    }

    /// c:228 — `statulprint(N)` formats as decimal.
    #[test]
    fn statulprint_decimal_format() {
        assert_eq!(statulprint(0), "0");
        assert_eq!(statulprint(42), "42");
        assert_eq!(statulprint(u64::MAX), format!("{}", u64::MAX));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/stat.c
    // c:43 statmodeprint / c:112 statuidprint / c:156 statgidprint /
    // c:199 stattimeprint / c:228 statulprint / c:237 statlinkprint / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:228 — `statulprint` is pure (deterministic).
    #[test]
    fn statulprint_is_pure() {
        for v in [0u64, 1, 42, 1_000_000, u64::MAX] {
            let first = statulprint(v);
            for _ in 0..5 {
                assert_eq!(statulprint(v), first, "statulprint({}) must be pure", v);
            }
        }
    }

    /// c:228 — `statulprint` output is non-empty ASCII digit string.
    #[test]
    fn statulprint_output_is_ascii_digits() {
        for v in [0u64, 1, 42, u32::MAX as u64, u64::MAX] {
            let s = statulprint(v);
            assert!(!s.is_empty(), "non-empty");
            assert!(
                s.chars().all(|c| c.is_ascii_digit()),
                "all chars must be digits: {:?}",
                s
            );
        }
    }

    /// c:43 — `statmodeprint` is pure (deterministic for same mode+flags).
    #[test]
    fn statmodeprint_is_pure() {
        let mode = 0o100644u32;
        let first = statmodeprint(mode, 0);
        for _ in 0..5 {
            assert_eq!(statmodeprint(mode, 0), first, "statmodeprint must be pure");
        }
    }

    /// c:43 — `statmodeprint(_, 0)` returns empty (no flags = no output).
    #[test]
    fn statmodeprint_no_flags_returns_empty() {
        assert_eq!(statmodeprint(0o100644, 0), "");
    }

    /// c:199 — `stattimeprint` is pure.
    #[test]
    fn stattimeprint_is_pure() {
        let t = 1_700_000_000i64;
        let first = stattimeprint(t, 0, 0);
        for _ in 0..5 {
            assert_eq!(stattimeprint(t, 0, 0), first, "stattimeprint must be pure");
        }
    }

    /// c:237 — `statlinkprint` for non-symlink returns empty.
    #[test]
    fn statlinkprint_non_symlink_returns_empty() {
        let r = statlinkprint(0o100644, "/tmp/__not_symlink__");
        // Regular file mode bit pattern → empty.
        assert!(
            r.is_empty() || r == "" || r.starts_with(" "),
            "non-symlink should return empty or marker, got {:?}",
            r
        );
    }

    /// c:237 — `statlinkprint` empty fname doesn't panic.
    #[test]
    fn statlinkprint_empty_fname_no_panic() {
        let _ = statlinkprint(0o100644, "");
    }

    /// c:112 — `statuidprint(uid, flags)` is pure (deterministic for same args).
    #[test]
    fn statuidprint_is_pure() {
        for uid in [0u32, 1, 1000, u32::MAX] {
            let first = statuidprint(uid, 0);
            for _ in 0..5 {
                assert_eq!(
                    statuidprint(uid, 0),
                    first,
                    "statuidprint({}, 0) must be pure",
                    uid
                );
            }
        }
    }

    /// c:156 — `statgidprint(gid, flags)` is pure.
    #[test]
    fn statgidprint_is_pure() {
        for gid in [0u32, 1, 1000, u32::MAX] {
            let first = statgidprint(gid, 0);
            for _ in 0..5 {
                assert_eq!(
                    statgidprint(gid, 0),
                    first,
                    "statgidprint({}, 0) must be pure",
                    gid
                );
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/stat.c
    // c:43 statmodeprint / c:112 statuidprint / c:156 statgidprint /
    // c:199 stattimeprint / c:228 statulprint / c:237 statlinkprint /
    // c:300 bin_stat + lifecycle type pins
    // ═══════════════════════════════════════════════════════════════════

    /// c:43 — `statmodeprint` returns String (compile-time type pin).
    #[test]
    fn statmodeprint_returns_string_type() {
        let _: String = statmodeprint(0o100644, 0);
    }

    /// c:112 — `statuidprint` returns String.
    #[test]
    fn statuidprint_returns_string_type() {
        let _: String = statuidprint(0, 0);
    }

    /// c:156 — `statgidprint` returns String.
    #[test]
    fn statgidprint_returns_string_type() {
        let _: String = statgidprint(0, 0);
    }

    /// c:199 — `stattimeprint` returns String.
    #[test]
    fn stattimeprint_returns_string_type() {
        let _: String = stattimeprint(0, 0, 0);
    }

    /// c:228 — `statulprint` returns String.
    #[test]
    fn statulprint_returns_string_type() {
        let _: String = statulprint(0);
    }

    /// c:237 — `statlinkprint` returns String.
    #[test]
    fn statlinkprint_returns_string_type() {
        let _: String = statlinkprint(0o100644, "");
    }

    /// c:228 — `statulprint(0)` returns "0".
    #[test]
    fn statulprint_zero_returns_zero_digit() {
        assert_eq!(statulprint(0), "0", "0 → \"0\" canonical");
    }

    /// c:228 — `statulprint(u64::MAX)` returns max decimal repr.
    #[test]
    fn statulprint_u64_max_returns_canonical_decimal() {
        let s = statulprint(u64::MAX);
        assert_eq!(s, u64::MAX.to_string(), "u64::MAX matches std decimal");
    }

    /// c:300 — `bin_stat` returns i32 (compile-time type pin).
    #[test]
    fn bin_stat_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_stat("stat", &[], &ops, 0);
    }

    /// c:579 — `setup_` returns i32 (compile-time type pin).
    #[test]
    fn stat_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:587 — features non-empty.
    #[test]
    fn stat_features_nonempty() {
        let _g = crate::test_util::global_state_lock();
        let mut feats = Vec::new();
        features_(std::ptr::null(), &mut feats);
        assert!(!feats.is_empty(), "stat must advertise ≥1 feature");
    }

    /// c:587 — features use b:/p: prefix per zsh module spec.
    #[test]
    fn stat_features_use_canonical_prefix() {
        let _g = crate::test_util::global_state_lock();
        let mut feats = Vec::new();
        features_(std::ptr::null(), &mut feats);
        for f in &feats {
            assert!(
                f.starts_with("b:") || f.starts_with("p:"),
                "feature {:?} must use b:/p: prefix",
                f
            );
        }
    }

    /// c:228 — `statulprint` is monotonically non-decreasing-length
    /// for monotonically increasing input (10 → 100 → 1000).
    #[test]
    fn statulprint_length_grows_with_magnitude() {
        let a = statulprint(10).len();
        let b = statulprint(100).len();
        let c = statulprint(1000).len();
        assert!(a <= b, "len(10) ≤ len(100), got {} vs {}", a, b);
        assert!(b <= c, "len(100) ≤ len(1000), got {} vs {}", b, c);
    }

    /// c:579-618 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn stat_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
        assert_eq!(finish_(null), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/stat.c
    // c:43 statmodeprint / c:112 statuidprint / c:156 statgidprint /
    // c:199 stattimeprint / c:228 statulprint / c:300 bin_stat
    // ═══════════════════════════════════════════════════════════════════

    /// c:43 — `statmodeprint` returns String (compile-time pin, alt).
    #[test]
    fn statmodeprint_returns_string_pin_alt() {
        let _: String = statmodeprint(0o644, 0);
    }

    /// c:43 — `statmodeprint` is deterministic for any mode.
    #[test]
    fn statmodeprint_deterministic_for_common_modes() {
        for mode in [0o644u32, 0o755, 0o600, 0o777, 0o000] {
            let a = statmodeprint(mode, 0);
            let b = statmodeprint(mode, 0);
            assert_eq!(a, b, "statmodeprint({:o}) must be pure", mode);
        }
    }

    /// c:112 — `statuidprint` returns String (compile-time pin, alt).
    #[test]
    fn statuidprint_returns_string_pin_alt() {
        let _: String = statuidprint(0, 0);
    }

    /// c:112 — `statuidprint(0, STF_RAW)` returns "0" (raw uid digit).
    /// Pin the c:115 STF_RAW arm; flags=0 produces empty by design.
    #[test]
    fn statuidprint_root_with_raw_flag_returns_zero_digit() {
        let s = statuidprint(0, STF_RAW);
        assert_eq!(s, "0", "uid 0 + STF_RAW must produce '0'");
    }

    /// c:156 — `statgidprint` returns String (compile-time pin, alt).
    #[test]
    fn statgidprint_returns_string_pin_alt() {
        let _: String = statgidprint(0, 0);
    }

    /// c:156 — `statgidprint(0, STF_RAW)` returns "0" (raw gid digit).
    /// Pin the c:159 STF_RAW arm; flags=0 produces empty by design.
    #[test]
    fn statgidprint_zero_with_raw_flag_returns_zero_digit() {
        let s = statgidprint(0, STF_RAW);
        assert_eq!(s, "0", "gid 0 + STF_RAW must produce '0'");
    }

    /// c:199 — `stattimeprint` returns String (compile-time pin, alt).
    #[test]
    fn stattimeprint_returns_string_pin_alt() {
        let _: String = stattimeprint(0, 0, 0);
    }

    /// c:199 — `stattimeprint(0, 0, STF_RAW)` returns "0" (raw epoch).
    /// Pin the c:202 STF_RAW arm; flags=0 produces empty by design.
    #[test]
    fn stattimeprint_epoch_zero_with_raw_flag_returns_zero() {
        let s = stattimeprint(0, 0, STF_RAW);
        assert_eq!(s, "0", "epoch 0 + STF_RAW must produce '0'");
    }

    /// c:300 — `bin_stat` exit code is non-negative.
    #[test]
    fn bin_stat_exit_code_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for argv in [
            vec![],
            vec!["/tmp".into()],
            vec!["/dev/null".into()],
            vec!["/__nonexistent_xyz__".into()],
        ] {
            let r = bin_stat("stat", &argv, &ops, 0);
            assert!(
                r >= 0,
                "exit code must be non-negative, got {} for {:?}",
                r,
                argv
            );
        }
    }

    /// c:300 — `bin_stat` of nonexistent path MUST return nonzero
    /// (C uses `stat(2)` which fails ENOENT, then zwarnnam + return 1).
    /// In zshrs the port silently returns 0.
    #[test]
    fn bin_stat_nonexistent_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_stat("stat", &["/__definitely_no_such_xyz__".into()], &ops, 0);
        assert_ne!(r, 0, "nonexistent path → nonzero");
    }

    /// c:228 — `statulprint` deterministic for powers-of-2.
    #[test]
    fn statulprint_deterministic_for_powers_of_two() {
        for shift in 0..64 {
            let v = 1u64 << shift;
            let a = statulprint(v);
            let b = statulprint(v);
            assert_eq!(a, b, "statulprint({}) must be pure", v);
        }
    }
}
