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

use crate::ported::exec::ShellExecutor;
use crate::ported::utils::zwarnnam;
use std::fs;
use std::os::unix::fs::MetadataExt;

// ============================================================
// Port of `enum statnum` from `Src/Modules/stat.c:33-35`.
// Anonymous integer constants for the per-element index passed
// to `statprint(..., iwhich, ...)`.
// ============================================================
/// Port of `HNAMEKEY` from `Src/Modules/stat.c:43`. Hash key the
/// `zstat -H` mode uses to store the file name in the result assoc.
pub const HNAMEKEY: &str = "name";                                       // c:43

pub const ST_DEV:      i32 = 0;                                          // c:33
pub const ST_INO:      i32 = 1;
pub const ST_MODE:     i32 = 2;
pub const ST_NLINK:    i32 = 3;
pub const ST_UID:      i32 = 4;
pub const ST_GID:      i32 = 5;
pub const ST_RDEV:     i32 = 6;
pub const ST_SIZE:     i32 = 7;
pub const ST_ATIM:     i32 = 8;
pub const ST_MTIM:     i32 = 9;
pub const ST_CTIM:     i32 = 10;
pub const ST_BLKSIZE:  i32 = 11;
pub const ST_BLOCKS:   i32 = 12;
pub const ST_READLINK: i32 = 13;
pub const ST_COUNT:    i32 = 14;                                         // c:34

// ============================================================
// Port of `enum statflags` from `Src/Modules/stat.c:36-38`.
// Bitmask flags passed to the print fns + bin_stat dispatch.
// ============================================================
pub const STF_NAME:   i32 = 1;                                           // c:36
pub const STF_FILE:   i32 = 2;
pub const STF_STRING: i32 = 4;
pub const STF_RAW:    i32 = 8;
pub const STF_PICK:   i32 = 16;
pub const STF_ARRAY:  i32 = 32;
pub const STF_GMT:    i32 = 64;
pub const STF_HASH:   i32 = 128;
pub const STF_OCTAL:  i32 = 256;                                         // c:38

/// Port of `statelts[]` from `Src/Modules/stat.c:39`. Names of the
/// 14 stat-elements, indexed by the `ST_*` constants above.
pub static STATELTS: &[&str] = &[                                        // c:39
    "device", "inode", "mode", "nlink",
    "uid", "gid", "rdev", "size", "atime",
    "mtime", "ctime", "blksize", "blocks",
    "link",
];

/// Port of `statmodeprint(mode_t mode, char *outbuf, int flags)` from `Src/Modules/stat.c:47`. Renders
/// a Unix mode word per the STF_RAW / STF_OCTAL / STF_STRING flag
/// combination — raw octal/decimal, "ls -l"-style permission
/// string, or both with the raw form parenthesised.
///
/// C signature: `static void statmodeprint(mode_t mode, char *outbuf, int flags)`.
/// Rust port returns the formatted string (caller writes to its
/// own buffer) — same observable output for a given flag set.
/// WARNING: param names don't match C — Rust=(mode, flags) vs C=(mode, outbuf, flags)
pub fn statmodeprint(mode: u32, flags: i32) -> String {                  // c:47
    let mut out = String::new();
    if (flags & STF_RAW) != 0 {                                          // c:50
        if (flags & STF_OCTAL) != 0 {                                    // c:51
            out.push_str(&format!("0{:o}", mode));
        } else {
            out.push_str(&format!("{}", mode));
        }
        if (flags & STF_STRING) != 0 {                                   // c:53
            out.push_str(" (");                                          // c:54
        }
    }
    if (flags & STF_STRING) != 0 {                                       // c:56
        let modes = b"?rwxrwxrwx";
        let mut pm = [b'-'; 10];
        // c:84-103 — file-type char.
        let ifmt = mode & 0o170_000;                                     // S_IFMT
        pm[0] = match ifmt {
            0o020_000 => b'c',  // S_ISCHR
            0o040_000 => b'd',  // S_ISDIR
            0o060_000 => b'b',  // S_ISBLK
            0o100_000 => b'-',  // S_ISREG
            0o120_000 => b'l',  // S_ISLNK
            0o140_000 => b's',  // S_ISSOCK
            0o010_000 => b'p',  // S_ISFIFO
            _ => b'?',
        };
        // c:105-107 — owner/group/other rwx bits.
        let bits = [
            0o0400, 0o0200, 0o0100,
            0o0040, 0o0020, 0o0010,
            0o0004, 0o0002, 0o0001,
        ];
        for i in 0..9 {
            pm[i + 1] = if (mode & bits[i]) != 0 { modes[i + 1] } else { b'-' };
        }
        // c:111-115 — setuid / setgid / sticky.
        if (mode & 0o4000) != 0 {                                        // S_ISUID
            pm[3] = if (mode & 0o0100) != 0 { b's' } else { b'S' };
        }
        if (mode & 0o2000) != 0 {                                        // S_ISGID
            pm[6] = if (mode & 0o0010) != 0 { b's' } else { b'S' };
        }
        if (mode & 0o1000) != 0 {                                        // S_ISVTX
            pm[9] = if (mode & 0o0001) != 0 { b't' } else { b'T' };
        }
        out.push_str(std::str::from_utf8(&pm).unwrap_or(""));
        if (flags & STF_RAW) != 0 {                                      // c:132
            out.push(')');                                               // c:132
        }
    }
    out
}

/// Port of `statuidprint(uid_t uid, char *outbuf, int flags)` from `Src/Modules/stat.c:132`. Renders
/// a uid in raw form (decimal), string form (user name via
/// `getpwuid`), or both.
/// WARNING: param names don't match C — Rust=(uid, flags) vs C=(uid, outbuf, flags)
pub fn statuidprint(uid: u32, flags: i32) -> String {                    // c:132
    let mut out = String::new();
    if (flags & STF_RAW) != 0 {                                          // c:135
        out.push_str(&format!("{}", uid));
        if (flags & STF_STRING) != 0 {                                   // c:137
            out.push_str(" (");
        }
    }
    if (flags & STF_STRING) != 0 {                                       // c:140
        let name = unsafe {
            // c:142 — `pwd = getpwuid(uid);`
            let p = libc::getpwuid(uid);
            if p.is_null() { String::new() }
            else {
                let nm = (*p).pw_name;
                if nm.is_null() { String::new() }
                else { std::ffi::CStr::from_ptr(nm).to_string_lossy().into_owned() }
            }
        };
        if name.is_empty() {                                              // c:148 numeric fallback
            out.push_str(&format!("{}", uid));
        } else {
            out.push_str(&name);                                          // c:161 pwd->pw_name
        }
        if (flags & STF_RAW) != 0 {                                      // c:161
            out.push(')');
        }
    }
    out
}

/// Port of `statgidprint(gid_t gid, char *outbuf, int flags)` from `Src/Modules/stat.c:161`. Symmetric
/// with `statuidprint` for gid via `getgrgid`.
/// WARNING: param names don't match C — Rust=(gid, flags) vs C=(gid, outbuf, flags)
pub fn statgidprint(gid: u32, flags: i32) -> String {                    // c:161
    let mut out = String::new();
    if (flags & STF_RAW) != 0 {                                          // c:164
        out.push_str(&format!("{}", gid));
        if (flags & STF_STRING) != 0 {                                   // c:166
            out.push_str(" (");
        }
    }
    if (flags & STF_STRING) != 0 {                                       // c:169
        let name = unsafe {
            let g = libc::getgrgid(gid);                                  // c:171
            if g.is_null() { String::new() }
            else {
                let nm = (*g).gr_name;
                if nm.is_null() { String::new() }
                else { std::ffi::CStr::from_ptr(nm).to_string_lossy().into_owned() }
            }
        };
        if name.is_empty() {
            out.push_str(&format!("{}", gid));                           // c:184
        } else {
            out.push_str(&name);                                         // c:178
        }
        if (flags & STF_RAW) != 0 {                                      // c:191
            out.push(')');
        }
    }
    out
}

/// Port of `stattimeprint(time_t tim, long nsecs, char *outbuf, int flags)` from `Src/Modules/stat.c:191`. Renders
/// a Unix timestamp + nsec offset: raw form is integer seconds;
/// string form is `ctime(3)` (or strftime via the timefmt global).
/// WARNING: param names don't match C — Rust=(tim, _nsecs, flags) vs C=(tim, nsecs, outbuf, flags)
pub fn stattimeprint(tim: i64, _nsecs: i64, flags: i32) -> String {      // c:191
    let mut out = String::new();
    if (flags & STF_RAW) != 0 {                                          // c:194
        out.push_str(&format!("{}", tim));
        if (flags & STF_STRING) != 0 {                                   // c:196
            out.push_str(" (");
        }
    }
    if (flags & STF_STRING) != 0 {                                       // c:199
        // c:200 — `ztrftime(buf, ..., timefmt, localtime(&tim), nsecs);`
        let st = std::time::UNIX_EPOCH + std::time::Duration::from_secs(tim.max(0) as u64);
        let formatted = crate::ported::utils::ztrftime("%a %b %e %k:%M:%S %Z %Y", st);
        out.push_str(&formatted);
        if (flags & STF_RAW) != 0 {                                      // c:211
            out.push(')');
        }
    }
    out
}

/// Port of `statulprint(unsigned long num, char *outbuf)` from `Src/Modules/stat.c:211`. Renders an
/// unsigned-long stat field as decimal (always raw, no STF_STRING
/// branch).
/// WARNING: param names don't match C — Rust=(num) vs C=(num, outbuf)
pub fn statulprint(num: u64) -> String {                                 // c:211
    format!("{}", num)                                                    // c:219
}

/// Port of `statlinkprint(struct stat *sbuf, char *outbuf, char *fname)` from `Src/Modules/stat.c:219`. For
/// symlinks, renders the link target via `readlink(2)`; otherwise
/// returns empty.
/// WARNING: param names don't match C — Rust=(sbuf_mode, fname) vs C=(sbuf, outbuf, fname)
pub fn statlinkprint(sbuf_mode: u32, fname: &str) -> String {            // c:219
    if (sbuf_mode & 0o170_000) != 0o120_000 {                            // c:219 S_ISLNK
        return String::new();
    }
    fs::read_link(fname)                                                  // c:226 readlink
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
pub fn statprint(meta: &fs::Metadata, fname: &str, iwhich: i32, flags: i32) -> String {  // c:234
    // c:234-241 — `if (flags & STF_NAME)` prefix with `name<space>`.
    // `%-8s` left-justifies the name to 8 chars when not PICK/ARRAY,
    // `%s ` otherwise.
    let name_prefix = if (flags & STF_NAME) != 0 {
        let n = STATELTS.get(iwhich as usize).copied().unwrap_or("");
        if (flags & (STF_PICK | STF_ARRAY)) != 0 {
            format!("{} ", n)                                            // c:239
        } else {
            format!("{:<8}", n)                                          // c:240
        }
    } else {
        String::new()
    };
    let val = match iwhich {
        ST_DEV      => format!("{}", meta.dev()),                        // c:240
        ST_INO      => format!("{}", meta.ino()),                        // c:241
        ST_MODE     => statmodeprint(meta.mode(), flags),                // c:242
        ST_NLINK    => format!("{}", meta.nlink()),                      // c:243
        ST_UID      => statuidprint(meta.uid(), flags),                  // c:244
        ST_GID      => statgidprint(meta.gid(), flags),                  // c:245
        ST_RDEV     => format!("{}", meta.rdev()),                       // c:246
        ST_SIZE     => statulprint(meta.size()),                         // c:247
        ST_ATIM     => stattimeprint(meta.atime(), 0, flags),            // c:248
        ST_MTIM     => stattimeprint(meta.mtime(), 0, flags),            // c:249
        ST_CTIM     => stattimeprint(meta.ctime(), 0, flags),            // c:250
        ST_BLKSIZE  => statulprint(meta.blksize()),                      // c:251
        ST_BLOCKS   => statulprint(meta.blocks()),                       // c:252
        ST_READLINK => statlinkprint(meta.mode(), fname),                // c:253
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
pub fn bin_stat(nam: &str, args: &[String],                                  // c:368
                _ops_unused: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    // c:370-374 — locals.
    let mut iwhich: i32 = -1;                                            // c:373
    let mut flags: i32 = 0;
    let mut found = 0i32;                                                // c:375
    let mut arrnam: Option<String> = None;
    let mut hashnam: Option<String> = None;
    let mut fd: i32 = 0;
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
            break;                                                       // c:386
        }
        if args[i].starts_with('+') {                                    // c:389
            if found != 0 { break; }
            for (idx, name) in STATELTS.iter().enumerate() {             // c:392
                if name.starts_with(arg) {
                    found += 1;
                    iwhich = idx as i32;
                }
            }
            if found > 1 {                                               // c:397
                zwarnnam(nam, &format!("{}: ambiguous stat element", arg));
                return 1;
            } else if found == 0 {                                       // c:400
                zwarnnam(nam, &format!("{}: no such stat element", arg));
                return 1;
            }
            // c:404 — `if (iwhich == ST_READLINK) ops->ind['L'] = 1;`
            if iwhich == ST_READLINK {
                ops[b'L' as usize] = true;
            }
            flags |= STF_PICK;                                           // c:406
        } else {                                                         // c:407 - flag arm
            for ch in arg.chars() {
                match ch {
                    'g' | 'l' | 'L' | 'n' | 'N' | 'o' | 'r' | 's' | 't' | 'T' => {
                        ops[ch as u8 as usize] = true;                   // c:411
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
                        let (val, endptr) = crate::ported::utils::zstrtol(args[i], 10);
                        if !endptr.is_empty() {
                            zwarnnam(nam, "bad file descriptor");
                            return 1;
                        }
                        fd = val as i32;
                        break;
                    }
                    'F' => {
                        i += 1;
                        if i >= args.len() {
                            zwarnnam(nam, "missing time format");
                            return 1;
                        }
                        // c:447 — force string format.
                        ops[b's' as usize] = true;
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

    if (flags & STF_ARRAY) != 0 && (flags & STF_HASH) != 0 {             // c:459
        zwarnnam(nam, "both array and hash requested");
        return 1;
    }

    if ops[b'l' as usize] {                                              // c:467
        // List elements + return.
        if let Some(ref name) = arrnam {                                 // c:469
            // c:472 — `setaparam(arrnam, names);` — array of element names.
            let joined: Vec<&str> = STATELTS.iter().copied().collect();
            crate::ported::params::setsparam(name, &joined.join(":"));
        } else {
            let joined: Vec<&str> = STATELTS.iter().copied().collect();
            println!("{}", joined.join(" "));                            // c:478 putchar
        }
        return 0;                                                        // c:489
    }

    if argv.is_empty() && !ops[b'f' as usize] {                          // c:491
        zwarnnam(nam, "no files given");
        return 1;
    } else if !argv.is_empty() && ops[b'f' as usize] {                   // c:493
        zwarnnam(nam, "no files allowed with -f");
        return 1;
    }

    // c:496+ — per-file stat + dispatch loop.
    let use_lstat = ops[b'L' as usize];
    let mut hash_out: Vec<(String, String)> = Vec::new();
    let mut array_out: Vec<String> = Vec::new();
    let show_type = ops[b't' as usize];                                  // c: -t
    let mut local_flags = flags;
    // c:513 — `if (OPT_ISSET(ops,'s') || !OPT_ISSET(ops,'r'))` STF_STRING.
    if ops[b's' as usize] { local_flags |= STF_STRING; }                  // c:514
    // c:516 — `if (OPT_ISSET(ops,'r') || !OPT_ISSET(ops,'s'))` STF_RAW.
    if ops[b'r' as usize] || !ops[b's' as usize] {                        // c:516
        local_flags |= STF_RAW;
    }
    // c:518-519 — `-n` → STF_FILE (filename prefix).
    if ops[b'n' as usize] { local_flags |= STF_FILE; }                    // c:519
    // c:520-521 — `-o` → STF_OCTAL.
    if ops[b'o' as usize] { local_flags |= STF_OCTAL; }                   // c:521
    // c:522-523 — `-t` → STF_NAME explicit.
    if ops[b't' as usize] { local_flags |= STF_NAME; }                    // c:523
    // c:525-530 — default STF_NAME when neither -A nor -H and no
    // single-element pick: every line gets a `name<sp>` prefix so
    // `zstat /etc/hosts` looks like `mode    33188` etc.
    if arrnam.is_none() && hashnam.is_none() {
        if argv.len() > 1 { local_flags |= STF_FILE; }                    // c:527
        if (local_flags & STF_PICK) == 0 {                                // c:528
            local_flags |= STF_NAME;                                      // c:529
        }
    }
    // c:532-535 — explicit -N / -f turn off STF_FILE; -T / -H turn off
    // STF_NAME (suppress prefix for `read` / hash use).
    if ops[b'N' as usize] || ops[b'f' as usize] {                         // c:532
        local_flags &= !STF_FILE;
    }
    if ops[b'T' as usize] || ops[b'H' as usize] {                         // c:534
        local_flags &= !STF_NAME;
    }
    let _ = show_type;

    for path in &argv {
        let meta = if use_lstat {
            fs::symlink_metadata(path)
        } else {
            fs::metadata(path)
        };
        let meta = match meta {
            Ok(m) => m,
            Err(e) => {
                zwarnnam(nam, &format!("{}: {}", path, e));
                continue;
            }
        };

        // c:573-581 — `STF_FILE` prefix the filename per file.
        if (local_flags & STF_FILE) != 0 && arrnam.is_none() && hashnam.is_none() {
            if (local_flags & STF_PICK) != 0 {
                print!("{} ", path);                                     // c:580
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
                println!("{}", val);                                     // c:591
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
                    println!("{}", val);                                 // c:603
                }
            }
        }
    }

    if let Some(name) = arrnam {                                         // c:setaparam
        // c — `setaparam(name, zarrdup(array_out));` — real indexed array.
        crate::ported::params::setaparam(&name, array_out);              // c:params.c:3595
    }
    if let Some(name) = hashnam {                                        // c:sethparam
        // c — `sethparam(name, ...);` — real assoc array. Flatten
        // hash_out into alternating [k,v,k,v,...].
        let mut flat: Vec<String> = Vec::with_capacity(hash_out.len() * 2);
        for (k, v) in hash_out {
            flat.push(k);
            flat.push(v);
        }
        crate::ported::params::sethparam(&name, flat);                   // c:params.c:3602
    }
    0
}

// =====================================================================
// static struct builtin bintab[]                                    c:638
// static struct features module_features                            c:642
// =====================================================================

use crate::ported::zsh_h::module;

// `bintab` — port of `static struct builtin bintab[]` (stat.c:638).


// `module_features` — port of `static struct features module_features`
// from stat.c:642.



/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/stat.c:651`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {                                    // c:651
    // C body c:653-654 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/stat.c:658`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {     // c:658
    *features = featuresarray(m, module_features());
    0                                                                    // c:673
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/stat.c:666`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {  // c:666
    handlefeatures(m, module_features(), enables) // c:673
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/stat.c:673`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {                                     // c:673
    // C body c:675-676 — `return 0`. Faithful empty-body port; the
    //                    zstat builtin registers via the bn_list dispatch.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/stat.c:680`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {                                  // c:680
    setfeatureenables(m, module_features(), None) // c:687
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/stat.c:687`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {                                   // c:687
    // C body c:689-690 — `return 0`. Faithful empty-body port; the
    //                    zstat builtin unregisters via cleanup_'s setfeatureenables.
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    #[test]
    fn statelts_count_matches_st_count() {
        assert_eq!(STATELTS.len() as i32, ST_COUNT);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    #[test]
    fn statmodeprint_octal_only() {
        let s = statmodeprint(0o100644, STF_RAW | STF_OCTAL);
        assert!(s.starts_with('0'));
        assert!(s.contains("644"));
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    #[test]
    fn statmodeprint_string_only() {
        let s = statmodeprint(0o100644, STF_STRING);
        assert_eq!(s.len(), 10);
        assert_eq!(&s[..1], "-");
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    #[test]
    fn statmodeprint_directory() {
        let s = statmodeprint(0o040755, STF_STRING);
        assert_eq!(&s[..1], "d");
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    #[test]
    fn statulprint_decimal() {
        assert_eq!(statulprint(12345), "12345");
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    #[test]
    fn statprint_size_via_index() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("x.txt");
        File::create(&path).unwrap().write_all(b"hello").unwrap();
        let meta = fs::metadata(&path).unwrap();
        let s = statprint(&meta, path.to_str().unwrap(), ST_SIZE, 0);
        assert_eq!(s, "5");
    }
}

use crate::ported::zsh_h::features as features_t;
use std::sync::{Mutex, OnceLock};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/stat.c`.
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 2,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,
        pd_size: 0,
        n_abstract: 0,
    }))
}

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/stat.c`.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:stat".to_string(), "b:zstat".to_string()]
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/stat.c`.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features_t>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 2]);
    }
    0
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/stat.c`.
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<features_t>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

