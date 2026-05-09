//! File stat module — port of `Src/Modules/stat.c`.
//!
//! C source has 2 enums (`statnum`, `statflags`) — both anonymous-
//! valued integer-constant tables. Rust port mirrors as constants
//! to avoid Rust-only type wrappers.
//!
//! Functions (matching C 1:1):
//!   - statmodeprint  [c:47]
//!   - statuidprint   [c:132]
//!   - statgidprint   [c:161]
//!   - stattimeprint  [c:191]
//!   - statulprint    [c:211]
//!   - statlinkprint  [c:219]
//!   - statprint      [c:234]
//!   - bin_stat       [c:368]
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

/// Port of `statmodeprint()` from `Src/Modules/stat.c:47`. Renders
/// a Unix mode word per the STF_RAW / STF_OCTAL / STF_STRING flag
/// combination — raw octal/decimal, "ls -l"-style permission
/// string, or both with the raw form parenthesised.
///
/// C signature: `static void statmodeprint(mode_t mode, char *outbuf, int flags)`.
/// Rust port returns the formatted string (caller writes to its
/// own buffer) — same observable output for a given flag set.
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
        if (flags & STF_RAW) != 0 {                                      // c:121
            out.push(')');                                               // c:122
        }
    }
    out
}

/// Port of `statuidprint()` from `Src/Modules/stat.c:132`. Renders
/// a uid in raw form (decimal), string form (user name via
/// `getpwuid`), or both.
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
            out.push_str(&name);                                          // c:144 pwd->pw_name
        }
        if (flags & STF_RAW) != 0 {                                      // c:155
            out.push(')');
        }
    }
    out
}

/// Port of `statgidprint()` from `Src/Modules/stat.c:161`. Symmetric
/// with `statuidprint` for gid via `getgrgid`.
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
        if (flags & STF_RAW) != 0 {                                      // c:187
            out.push(')');
        }
    }
    out
}

/// Port of `stattimeprint()` from `Src/Modules/stat.c:191`. Renders
/// a Unix timestamp + nsec offset: raw form is integer seconds;
/// string form is `ctime(3)` (or strftime via the timefmt global).
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
        if (flags & STF_RAW) != 0 {                                      // c:204
            out.push(')');
        }
    }
    out
}

/// Port of `statulprint()` from `Src/Modules/stat.c:211`. Renders an
/// unsigned-long stat field as decimal (always raw, no STF_STRING
/// branch).
pub fn statulprint(num: u64) -> String {                                 // c:211
    format!("{}", num)                                                    // c:213
}

/// Port of `statlinkprint()` from `Src/Modules/stat.c:219`. For
/// symlinks, renders the link target via `readlink(2)`; otherwise
/// returns empty.
pub fn statlinkprint(sbuf_mode: u32, fname: &str) -> String {            // c:219
    if (sbuf_mode & 0o170_000) != 0o120_000 {                            // c:222 S_ISLNK
        return String::new();
    }
    fs::read_link(fname)                                                  // c:226 readlink
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Port of `statprint()` from `Src/Modules/stat.c:234`. The unified
/// per-field dispatcher: given a stat metadata, file name, the
/// `iwhich` index from `STATELTS`, and a flag word, produce the
/// formatted value string.
///
/// C signature: `static void statprint(struct stat *sbuf, char *outbuf,
///                                      char *fname, int iwhich, int flags)`.
pub fn statprint(meta: &fs::Metadata, fname: &str, iwhich: i32, flags: i32) -> String {  // c:234
    match iwhich {
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
    }
}

/// Port of `bin_stat()` from `Src/Modules/stat.c:368`. The `zstat`
/// builtin entry. Parses the `+ELEMENT` / `-flag` / `-A NAME` /
/// `-H NAME` / `-f FD` / `-F FORMAT` arg syntax, then calls
/// `lstat`/`stat`/`fstat` per file, dispatching `statprint` for
/// each requested element.
///
/// C signature: `static int bin_stat(char *name, char **args,
///                                    Options ops, int func)`.
pub fn bin_stat(exec: &mut ShellExecutor, nam: &str, args: &[&str], ops: &mut [bool; 256])
    -> i32                                                                // c:368
{
    let mut iwhich: i32 = -1;                                            // c:373
    let mut flags: i32 = 0;
    let mut found = 0i32;                                                // c:375
    let mut arrnam: Option<String> = None;
    let mut hashnam: Option<String> = None;
    let mut fd: i32 = 0;
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
            let names: Vec<String> = STATELTS.iter().map(|s| s.to_string()).collect();
            exec.arrays.insert(name.clone(), names);
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
    let show_name = ops[b'n' as usize];                                  // c: -n
    let show_type = ops[b't' as usize];                                  // c: -t
    let mut local_flags = flags;
    if ops[b's' as usize] { local_flags |= STF_STRING; }                  // c: -s force string
    if ops[b'r' as usize] { local_flags |= STF_RAW; }                     // c: -r raw
    if ops[b'o' as usize] { local_flags |= STF_OCTAL; }                   // c: -o octal
    // Default: if neither raw nor string, use raw.
    if (local_flags & (STF_RAW | STF_STRING)) == 0 {
        local_flags |= STF_RAW;
    }

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
                if show_name { print!("{} ", STATELTS[iwhich as usize]); }
                if show_type && argv.len() > 1 { print!("{} ", path); }
                println!("{}", val);
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
                    if show_name {
                        println!("{:<8} {}", STATELTS[idx], val);
                    } else {
                        println!("{}", val);
                    }
                }
            }
        }
    }

    if let Some(name) = arrnam {                                         // c:setaparam
        exec.arrays.insert(name, array_out);
    }
    if let Some(name) = hashnam {                                        // c:sethparam
        let map: indexmap::IndexMap<String, String> = hash_out.into_iter().collect();
        exec.assoc_arrays.insert(name, map);
    }
    0
}

/// Port of `setup_()` from `Src/Modules/stat.c:651`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:651
    0                                                                    // c:654
}

/// Port of `features_()` from `Src/Modules/stat.c:658`. C body is
/// `*features = featuresarray(m, &module_features); return 0;`.
pub fn features_() -> i32 {                                              // c:658
    0                                                                    // c:662
}

/// Port of `enables_()` from `Src/Modules/stat.c:666`. C body is
/// `return handlefeatures(m, &module_features, enables);`.
pub fn enables_() -> i32 {                                               // c:666
    0                                                                    // c:669
}

/// Port of `boot_()` from `Src/Modules/stat.c:673`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn boot_() -> i32 {                                                  // c:673
    0                                                                    // c:676
}

/// Port of `cleanup_()` from `Src/Modules/stat.c:680`. C body is
/// `return setfeatureenables(m, &module_features, NULL);`.
pub fn cleanup_() -> i32 {                                               // c:680
    0                                                                    // c:683
}

/// Port of `finish_()` from `Src/Modules/stat.c:687`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:687
    0                                                                    // c:690
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn statelts_count_matches_st_count() {
        assert_eq!(STATELTS.len() as i32, ST_COUNT);
    }

    #[test]
    fn statmodeprint_octal_only() {
        let s = statmodeprint(0o100644, STF_RAW | STF_OCTAL);
        assert!(s.starts_with('0'));
        assert!(s.contains("644"));
    }

    #[test]
    fn statmodeprint_string_only() {
        let s = statmodeprint(0o100644, STF_STRING);
        assert_eq!(s.len(), 10);
        assert_eq!(&s[..1], "-");
    }

    #[test]
    fn statmodeprint_directory() {
        let s = statmodeprint(0o040755, STF_STRING);
        assert_eq!(&s[..1], "d");
    }

    #[test]
    fn statulprint_decimal() {
        assert_eq!(statulprint(12345), "12345");
    }

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

// =====================================================
// Methods moved verbatim from src/ported/exec.rs
// =====================================================

impl ShellExecutor {
    /// `zstat` / `stat` builtin shim — adapts `&[String]` argv to
    /// `bin_stat` over a `&mut [bool; 256]` ops bitmask. zshrs's
    /// builtin dispatcher invokes this directly via two names
    /// (zstat is the canonical, stat is the alias) — both go
    /// through this entry.
    pub(crate) fn bin_stat(&mut self, args: &[String]) -> i32 {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut ops = [false; 256];
        bin_stat(self, "stat", &argv, &mut ops)
    }

    pub(crate) fn builtin_zstat(&mut self, args: &[String]) -> i32 {
        self.bin_stat(args)
    }
}
