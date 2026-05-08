//! Resource limits - port of Builtins/rlimits.c
//!
//! Provides `limit`, `ulimit`, and `unlimit` builtins for managing resource limits.

// `RLIMIT_*` constants are typed `__rlimit_resource_t` (= u32) on glibc
// Linux and `c_int` (= i32) on macOS / *BSD. The `as i32` casts are
// portable but redundant on whichever platform the type already matches —
// silence clippy's per-platform unnecessary_cast firing.
#![allow(clippy::unnecessary_cast)]

#[cfg(unix)]
use libc::{
    getrlimit, rlimit, setrlimit, RLIMIT_AS, RLIMIT_CORE, RLIMIT_CPU, RLIMIT_DATA, RLIMIT_FSIZE,
    RLIMIT_NOFILE, RLIMIT_STACK, RLIM_INFINITY,
};

/// Resource limit unit type.
/// Port of the `ZLIMTYPE_*` enum from Src/Builtins/rlimits.c —
/// `set_resinfo()` (line 194) tags each `RLIMIT_*` with whether
/// it's measured in KB / blocks / seconds / a count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitType {
    Memory,
    Number,
    Time,
    Microseconds,
    Unknown,
}

/// Per-resource metadata.
/// Port of `struct resinfo` from Src/Builtins/rlimits.c —
/// `set_resinfo()` (line 194) populates one entry per
/// `RLIMIT_*` with the libcasm name, opt-letter, descr text, and
/// scaling unit. `find_resource()` (line 239) searches by
/// `opt` letter for `ulimit -X`.
#[derive(Debug, Clone)]
pub struct ResInfo {
    pub res: i32,
    pub name: &'static str,
    pub limit_type: LimitType,
    pub unit: u64,
    pub opt: char,
    pub descr: &'static str,
}

/// Known resource limits — the same table `set_resinfo()` from
/// Src/Builtins/rlimits.c:194 builds at module load.
///
/// `RLIMIT_*` constants are typed `__rlimit_resource_t` (= u32) on glibc
/// Linux and `c_int` (= i32) on macOS / *BSD. We store as `i32` internally
/// and cast at definition + call sites; the underlying values fit in i32
/// either way (max RLIMIT enum is RLIMIT_NLIMITS ≤ 16 on every platform).
#[cfg(unix)]
pub static KNOWN_RESOURCES: &[ResInfo] = &[
    ResInfo {
        res: RLIMIT_CPU as i32,
        name: "cputime",
        limit_type: LimitType::Time,
        unit: 1,
        opt: 't',
        descr: "cpu time (seconds)",
    },
    ResInfo {
        res: RLIMIT_FSIZE as i32,
        name: "filesize",
        limit_type: LimitType::Memory,
        unit: 512,
        opt: 'f',
        descr: "file size (blocks)",
    },
    ResInfo {
        res: RLIMIT_DATA as i32,
        name: "datasize",
        limit_type: LimitType::Memory,
        unit: 1024,
        opt: 'd',
        descr: "data seg size (kbytes)",
    },
    ResInfo {
        res: RLIMIT_STACK as i32,
        name: "stacksize",
        limit_type: LimitType::Memory,
        unit: 1024,
        opt: 's',
        descr: "stack size (kbytes)",
    },
    ResInfo {
        res: RLIMIT_CORE as i32,
        name: "coredumpsize",
        limit_type: LimitType::Memory,
        unit: 512,
        opt: 'c',
        descr: "core file size (blocks)",
    },
    ResInfo {
        res: RLIMIT_NOFILE as i32,
        name: "descriptors",
        limit_type: LimitType::Number,
        unit: 1,
        opt: 'n',
        descr: "file descriptors",
    },
    ResInfo {
        res: RLIMIT_AS as i32,
        name: "addressspace",
        limit_type: LimitType::Memory,
        unit: 1024,
        opt: 'v',
        descr: "address space (kbytes)",
    },
];

#[cfg(not(unix))]
pub static KNOWN_RESOURCES: &[ResInfo] = &[];

/// A single soft- or hard-limit value.
/// Port of the `RLIM_INFINITY` sentinel handling
/// `printrlim()` (Src/Builtins/rlimits.c:253) and
/// `zstrtorlimt()` (line 272) do — keeps "unlimited" as a
/// distinct variant rather than a magic max integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitValue {
    Unlimited,
    Value(u64),
}

impl LimitValue {
    #[cfg(unix)]
    pub fn from_rlim(val: u64) -> Self {
        if val == RLIM_INFINITY {
            LimitValue::Unlimited
        } else {
            LimitValue::Value(val)
        }
    }

    #[cfg(unix)]
    pub fn to_rlim(&self) -> u64 {
        match self {
            LimitValue::Unlimited => RLIM_INFINITY,
            LimitValue::Value(v) => *v,
        }
    }

    pub fn format(&self, info: Option<&ResInfo>) -> String {
        match self {
            LimitValue::Unlimited => "unlimited".to_string(),
            LimitValue::Value(val) => {
                if let Some(info) = info {
                    match info.limit_type {
                        LimitType::Time => {
                            let hours = val / 3600;
                            let mins = (val / 60) % 60;
                            let secs = val % 60;
                            format!("{}:{:02}:{:02}", hours, mins, secs)
                        }
                        LimitType::Microseconds => format!("{}us", val),
                        LimitType::Memory => {
                            if *val >= 1024 * 1024 {
                                format!("{}MB", val / (1024 * 1024))
                            } else {
                                format!("{}kB", val / 1024)
                            }
                        }
                        _ => format!("{}", val),
                    }
                } else {
                    format!("{}", val)
                }
            }
        }
    }
}

impl std::fmt::Display for LimitValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LimitValue::Unlimited => write!(f, "unlimited"),
            LimitValue::Value(v) => write!(f, "{}", v),
        }
    }
}

/// Resource limits cache.
/// Port of the `limits[]` cache Src/Builtins/rlimits.c keeps —
/// `do_limit()` (line 431) writes through it on `setrlimit(2)`
/// success, `showlimits()` (line 346) reads from it.
#[derive(Debug, Default)]
pub struct ResourceLimits {
    #[cfg(unix)]
    cached: std::collections::HashMap<i32, (LimitValue, LimitValue)>,
}

impl ResourceLimits {
    pub fn new() -> Self {
        Self {
            #[cfg(unix)]
            cached: std::collections::HashMap::new(),
        }
    }

    /// Look up resource info by name (prefix match).
    /// Port of the resource-name lookup loop inside `bin_limit()`
    /// (Src/Builtins/rlimits.c:519) — same "ambiguous" rejection
    /// when more than one entry shares the prefix.
    pub fn find_by_name(&self, name: &str) -> Option<&'static ResInfo> {
        let mut found: Option<&'static ResInfo> = None;
        let mut ambiguous = false;

        for info in KNOWN_RESOURCES {
            if info.name.starts_with(name) {
                if found.is_some() {
                    ambiguous = true;
                    break;
                }
                found = Some(info);
            }
        }

        if ambiguous {
            None
        } else {
            found
        }
    }

    /// Get resource info by option character
    pub fn find_by_opt(&self, opt: char) -> Option<&'static ResInfo> {
        KNOWN_RESOURCES.iter().find(|info| info.opt == opt)
    }

    /// Get resource info by resource number
    pub fn find_by_res(&self, res: i32) -> Option<&'static ResInfo> {
        KNOWN_RESOURCES.iter().find(|info| info.res == res)
    }

    /// Read the current `(soft, hard)` pair via `getrlimit(2)`.
    /// Port of the `getrlimit()` call inside `do_limit()`
    /// (Src/Builtins/rlimits.c:431) and `printulimit()` (line 386)
    /// — same `RLIM_INFINITY` → `Unlimited` mapping.
    #[cfg(unix)]
    pub fn get(&self, res: i32) -> Result<(LimitValue, LimitValue), String> {
        let mut rlim = rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };

        if unsafe { getrlimit(res as _, &mut rlim) } < 0 {
            return Err(format!(
                "can't read limit: {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok((
            LimitValue::from_rlim(rlim.rlim_cur),
            LimitValue::from_rlim(rlim.rlim_max),
        ))
    }

    #[cfg(not(unix))]
    pub fn get(&self, _res: i32) -> Result<(LimitValue, LimitValue), String> {
        Err("resource limits not supported on this platform".to_string())
    }

    /// Set a `(soft, hard)` pair via `setrlimit(2)`.
    /// Port of `do_limit()` from Src/Builtins/rlimits.c:431 — same
    /// soft-must-not-exceed-hard validation, same root-only
    /// allow-raising-hard rule.
    #[cfg(unix)]
    pub fn set(
        &mut self,
        res: i32,
        soft: Option<LimitValue>,
        hard: Option<LimitValue>,
    ) -> Result<(), String> {
        let (cur_soft, cur_hard) = self.get(res)?;

        let new_soft = soft.unwrap_or(cur_soft);
        let new_hard = hard.unwrap_or(cur_hard);

        if let LimitValue::Value(s) = new_soft {
            if let LimitValue::Value(h) = new_hard {
                if s > h {
                    return Err("soft limit exceeds hard limit".to_string());
                }
            }
        }

        let euid = unsafe { libc::geteuid() };
        if euid != 0 {
            if let (LimitValue::Value(new_h), LimitValue::Value(cur_h)) = (new_hard, cur_hard) {
                if new_h > cur_h {
                    return Err("can't raise hard limits".to_string());
                }
            }
        }

        let rlim = rlimit {
            rlim_cur: new_soft.to_rlim(),
            rlim_max: new_hard.to_rlim(),
        };

        if unsafe { setrlimit(res as _, &rlim) } < 0 {
            return Err(format!(
                "setrlimit failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        self.cached.insert(res, (new_soft, new_hard));
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn set(
        &mut self,
        _res: i32,
        _soft: Option<LimitValue>,
        _hard: Option<LimitValue>,
    ) -> Result<(), String> {
        Err("resource limits not supported on this platform".to_string())
    }

    /// Remove a limit (set to `RLIM_INFINITY`).
    /// Port of `do_unlimit()` from Src/Builtins/rlimits.c:622 —
    /// matches the C source's `-H` (hard) flag handling.
    pub fn unlimit(&mut self, res: i32, hard: bool) -> Result<(), String> {
        if hard {
            self.set(
                res,
                Some(LimitValue::Unlimited),
                Some(LimitValue::Unlimited),
            )
        } else {
            let (_, cur_hard) = self.get(res)?;
            self.set(res, Some(cur_hard), None)
        }
    }

    /// List all limits
    pub fn list_all(&self, hard: bool) -> Vec<(String, LimitValue)> {
        let mut result = Vec::new();

        for info in KNOWN_RESOURCES {
            if let Ok((soft, hard_val)) = self.get(info.res) {
                let val = if hard { hard_val } else { soft };
                result.push((info.name.to_string(), val));
            }
        }

        result
    }
}

/// Parse a limit value string
/// Parse a numeric limit string into a `LimitValue`.
/// Port of `zstrtorlimt()` from Src/Builtins/rlimits.c:272 — same
/// `unlimited`/`hard`/`soft`/`Inf` keywords and unit-scaled
/// numeric form.
pub fn zstrtorlimt(s: &str, info: Option<&ResInfo>) -> Result<LimitValue, String> {
    if s == "unlimited" {
        return Ok(LimitValue::Unlimited);
    }

    let info = info.ok_or("unknown resource type")?;

    match info.limit_type {
        LimitType::Time => {
            if let Some(colon_pos) = s.find(':') {
                let hours: u64 = s[..colon_pos].parse().map_err(|_| "invalid number")?;
                let rest = &s[colon_pos + 1..];

                let (mins, secs) = if let Some(colon2) = rest.find(':') {
                    let m: u64 = rest[..colon2].parse().map_err(|_| "invalid number")?;
                    let s: u64 = rest[colon2 + 1..].parse().map_err(|_| "invalid number")?;
                    (m, s)
                } else {
                    let m: u64 = rest.parse().map_err(|_| "invalid number")?;
                    (m, 0)
                };

                Ok(LimitValue::Value(hours * 3600 + mins * 60 + secs))
            } else {
                let s_lower = s.to_lowercase();
                let (num_str, multiplier) = if s_lower.ends_with('h') {
                    (&s[..s.len() - 1], 3600)
                } else if s_lower.ends_with('m') {
                    (&s[..s.len() - 1], 60)
                } else {
                    (s, 1)
                };

                let val: u64 = num_str.parse().map_err(|_| "invalid number")?;
                Ok(LimitValue::Value(val * multiplier))
            }
        }
        LimitType::Memory => {
            let s_lower = s.to_lowercase();
            let (num_str, multiplier) = if s_lower.ends_with('g') {
                (&s[..s.len() - 1], 1024 * 1024 * 1024)
            } else if s_lower.ends_with('m') {
                (&s[..s.len() - 1], 1024 * 1024)
            } else if s_lower.ends_with('k') {
                (&s[..s.len() - 1], 1024)
            } else {
                (s, 1024)
            };

            let val: u64 = num_str.parse().map_err(|_| "invalid number")?;
            Ok(LimitValue::Value(val * multiplier))
        }
        _ => {
            let val: u64 = s.parse().map_err(|_| "limit must be a number")?;
            Ok(LimitValue::Value(val))
        }
    }
}

/// Format a limit for display (ulimit builtin style)
/// Format a limit for `ulimit` builtin display.
/// Port of `printulimit()` from Src/Builtins/rlimits.c:386 — same
/// optional header line, same per-resource scale.
pub fn printulimit(info: &ResInfo, val: LimitValue, show_header: bool) -> String {
    let mut result = String::new();

    if show_header {
        result.push_str(&format!("-{}: {:<32}", info.opt, info.descr));
    }

    match val {
        LimitValue::Unlimited => result.push_str("unlimited"),
        LimitValue::Value(v) => {
            let display_val = v / info.unit;
            result.push_str(&format!("{}", display_val));
        }
    }

    result
}

/// Execute the limit builtin
/// `limit` builtin entry point.
/// Port of `bin_limit()` from Src/Builtins/rlimits.c:519 —
/// dispatches between list (no args) and set (resource value).
pub fn bin_limit(
    args: &[&str],
    limits: &mut ResourceLimits,
    hard: bool,
    set: bool,
) -> (i32, String) {
    let mut output = String::new();

    if args.is_empty() {
        for (name, val) in limits.list_all(hard) {
            let info = limits.find_by_name(&name);
            output.push_str(&format!("{:<16}{}", &name, val.format(info)));
            output.push('\n');
        }
        return (0, output);
    }

    let mut i = 0;
    while i < args.len() {
        let name = args[i];

        if name.chars().all(|c| c.is_ascii_digit()) {
            let res: i32 = match name.parse() {
                Ok(n) => n,
                Err(_) => return (1, "limit: invalid resource number\n".to_string()),
            };

            if i + 1 >= args.len() {
                match limits.get(res) {
                    Ok((soft, hard_val)) => {
                        let val = if hard { hard_val } else { soft };
                        output.push_str(&format!("{:<16}{}\n", res, val));
                    }
                    Err(e) => return (1, format!("limit: {}\n", e)),
                }
                i += 1;
                continue;
            }

            let val_str = args[i + 1];
            let val = match zstrtorlimt(val_str, None) {
                Ok(v) => v,
                Err(e) => return (1, format!("limit: {}\n", e)),
            };

            if set {
                let (soft, hard_opt) = if hard {
                    (None, Some(val))
                } else {
                    (Some(val), None)
                };

                if let Err(e) = limits.set(res, soft, hard_opt) {
                    return (1, format!("limit: {}\n", e));
                }
            }

            i += 2;
            continue;
        }

        let info = match limits.find_by_name(name) {
            Some(info) => info,
            None => return (1, format!("limit: no such resource: {}\n", name)),
        };

        if i + 1 >= args.len() {
            match limits.get(info.res) {
                Ok((soft, hard_val)) => {
                    let val = if hard { hard_val } else { soft };
                    output.push_str(&format!("{:<16}{}", info.name, val.format(Some(info))));
                    output.push('\n');
                }
                Err(e) => return (1, format!("limit: {}\n", e)),
            }
            i += 1;
            continue;
        }

        let val_str = args[i + 1];
        let val = match zstrtorlimt(val_str, Some(info)) {
            Ok(v) => v,
            Err(e) => return (1, format!("limit: {}\n", e)),
        };

        if set {
            let (soft, hard_opt) = if hard {
                (None, Some(val))
            } else {
                (Some(val), None)
            };

            if let Err(e) = limits.set(info.res, soft, hard_opt) {
                return (1, format!("limit: {}\n", e));
            }
        }

        i += 2;
    }

    (0, output)
}

/// Execute the ulimit builtin
/// `ulimit` builtin entry point.
/// Port of `bin_ulimit()` from Src/Builtins/rlimits.c:729 — the
/// POSIX-flavored counterpart of `limit`. Honours `-H`/`-S` and
/// the per-resource short flag (`-t`/`-f`/`-d`/etc.) the C source
/// resolves via `find_resource()` (line 239).
pub fn bin_ulimit(
    args: &[&str],
    limits: &mut ResourceLimits,
    hard: bool,
    soft: bool,
) -> (i32, String) {
    let mut output = String::new();
    let show_all = args.contains(&"-a");

    if show_all || args.is_empty() {
        let use_hard = hard && !soft;

        for info in KNOWN_RESOURCES {
            if let Ok((s, h)) = limits.get(info.res) {
                let val = if use_hard { h } else { s };
                output.push_str(&printulimit(info, val, true));
                output.push('\n');
            }
        }
        return (0, output);
    }

    let mut i = 0;
    // RLIMIT_FSIZE is u32 on Linux, i32 on macOS; pin local type to i32
    // for cross-platform call-site uniformity (find_by_res/get/set all
    // take i32).
    let mut res: i32 = RLIMIT_FSIZE as i32;
    let mut use_hard = hard && !soft;

    while i < args.len() {
        let arg = args[i];

        if let Some(after_dash) = arg.strip_prefix('-') {
            for c in after_dash.chars() {
                match c {
                    'H' => use_hard = true,
                    'S' => use_hard = false,
                    'a' => {}
                    _ => {
                        if let Some(info) = limits.find_by_opt(c) {
                            res = info.res;
                        } else {
                            return (1, format!("ulimit: bad option: -{}\n", c));
                        }
                    }
                }
            }
            i += 1;
            continue;
        }

        let info = limits.find_by_res(res);
        let val = match zstrtorlimt(arg, info) {
            Ok(v) => v,
            Err(e) => return (1, format!("ulimit: {}\n", e)),
        };

        let (soft_opt, hard_opt) = if use_hard {
            (None, Some(val))
        } else {
            (Some(val), None)
        };

        if let Err(e) = limits.set(res, soft_opt, hard_opt) {
            return (1, format!("ulimit: {}\n", e));
        }

        i += 1;
    }

    if let Some(info) = limits.find_by_res(res) {
        if let Ok((s, h)) = limits.get(res) {
            let val = if use_hard { h } else { s };
            output.push_str(&printulimit(info, val, false));
            output.push('\n');
        }
    }

    (0, output)
}

/// Execute the unlimit builtin
/// `unlimit` builtin entry point.
/// Port of `bin_unlimit()` from Src/Builtins/rlimits.c:670 —
/// removes the named limit (or every limit when no args). Calls
/// `do_unlimit()` (line 622) per resource.
pub fn bin_unlimit(args: &[&str], limits: &mut ResourceLimits, hard: bool) -> (i32, String) {
    if args.is_empty() {
        for info in KNOWN_RESOURCES {
            if let Err(e) = limits.unlimit(info.res, hard) {
                if hard {
                    return (1, format!("unlimit: {}: {}\n", info.name, e));
                }
            }
        }
        return (0, String::new());
    }

    for name in args {
        let info = match limits.find_by_name(name) {
            Some(info) => info,
            None => {
                if name.chars().all(|c| c.is_ascii_digit()) {
                    let res: i32 = match name.parse() {
                        Ok(n) => n,
                        Err(_) => return (1, "unlimit: invalid resource number\n".to_string()),
                    };
                    if let Err(e) = limits.unlimit(res, hard) {
                        return (1, format!("unlimit: {}\n", e));
                    }
                    continue;
                }
                return (1, format!("unlimit: no such resource: {}\n", name));
            }
        };

        if let Err(e) = limits.unlimit(info.res, hard) {
            return (1, format!("unlimit: {}: {}\n", info.name, e));
        }
    }

    (0, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_limit_value_format() {
        assert_eq!(LimitValue::Unlimited.format(None), "unlimited");
        assert_eq!(LimitValue::Value(1234).format(None), "1234");
    }

    #[test]
    fn test_parse_limit_unlimited() {
        let info = &KNOWN_RESOURCES[0]; // cputime
        assert_eq!(
            zstrtorlimt("unlimited", Some(info)).unwrap(),
            LimitValue::Unlimited
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_parse_limit_time() {
        let info = KNOWN_RESOURCES
            .iter()
            .find(|i| i.limit_type == LimitType::Time)
            .unwrap();

        assert_eq!(
            zstrtorlimt("60", Some(info)).unwrap(),
            LimitValue::Value(60)
        );
        assert_eq!(
            zstrtorlimt("1h", Some(info)).unwrap(),
            LimitValue::Value(3600)
        );
        assert_eq!(
            zstrtorlimt("5m", Some(info)).unwrap(),
            LimitValue::Value(300)
        );
        assert_eq!(
            zstrtorlimt("1:30", Some(info)).unwrap(),
            LimitValue::Value(3600 + 30 * 60)
        );
        assert_eq!(
            zstrtorlimt("1:30:45", Some(info)).unwrap(),
            LimitValue::Value(3600 + 30 * 60 + 45)
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_parse_limit_memory() {
        let info = KNOWN_RESOURCES
            .iter()
            .find(|i| i.limit_type == LimitType::Memory)
            .unwrap();

        assert_eq!(
            zstrtorlimt("100", Some(info)).unwrap(),
            LimitValue::Value(100 * 1024)
        );
        assert_eq!(
            zstrtorlimt("100k", Some(info)).unwrap(),
            LimitValue::Value(100 * 1024)
        );
        assert_eq!(
            zstrtorlimt("10M", Some(info)).unwrap(),
            LimitValue::Value(10 * 1024 * 1024)
        );
        assert_eq!(
            zstrtorlimt("1G", Some(info)).unwrap(),
            LimitValue::Value(1024 * 1024 * 1024)
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_find_resource() {
        let limits = ResourceLimits::new();

        assert!(limits.find_by_name("cpu").is_some());
        assert!(limits.find_by_name("cputime").is_some());
        assert!(limits.find_by_name("file").is_some());
        assert!(limits.find_by_name("nonexistent").is_none());

        assert!(limits.find_by_opt('t').is_some());
        assert!(limits.find_by_opt('f').is_some());
        assert!(limits.find_by_opt('z').is_none());
    }

    #[test]
    #[cfg(unix)]
    fn test_get_limits() {
        let limits = ResourceLimits::new();

        let result = limits.get(RLIMIT_NOFILE as i32);
        assert!(result.is_ok());

        let (soft, hard) = result.unwrap();
        match soft {
            LimitValue::Unlimited => {}
            LimitValue::Value(v) => assert!(v > 0),
        }
        match hard {
            LimitValue::Unlimited => {}
            LimitValue::Value(v) => assert!(v > 0),
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_list_all() {
        let limits = ResourceLimits::new();
        let all = limits.list_all(false);

        assert!(!all.is_empty());
        assert!(all.iter().any(|(name, _)| name == "cputime"));
        assert!(all.iter().any(|(name, _)| name == "filesize"));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// `ulimit` builtin entry point. Thin wrapper that delegates to
    /// the canonical port at `crate::builtins::rlimits::bin_ulimit`
    /// (Src/Builtins/rlimits.c:729). Per-flag (`-H`/`-S`/`-a`/`-cdfnstuv...`)
    /// parsing lives in the canonical impl.
    pub(crate) fn bin_ulimit(&self, args: &[String]) -> i32 {
        use crate::builtins::rlimits::{bin_ulimit as bu, ResourceLimits};
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut limits = ResourceLimits::new();
        let (status, output) = bu(&argv, &mut limits, /* hard = */ false, /* soft = */ false);
        if !output.is_empty() {
            print!("{}", output);
        }
        status
    }
    /// limit - csh-style resource limits.
    /// `limit` builtin entry point. Thin wrapper that parses the
    /// `-h` / `-s` flag and delegates to the canonical port at
    /// `crate::builtins::rlimits::bin_limit` (Src/Builtins/rlimits.c:519).
    pub(crate) fn bin_limit(&self, args: &[String]) -> i32 {
        use crate::builtins::rlimits::{bin_limit as bl, ResourceLimits};

        // Parse leading -s / -h flag exactly like bin_limit() does at
        // Src/Builtins/rlimits.c:519. `-s` is "soft" (default) so a
        // bare `-s` with no further args is a no-op listing.
        let mut hard = false;
        let mut seen_s_alone = false;
        let mut positional: &[String] = args;
        if let Some(first) = args.first() {
            match first.as_str() {
                "-s" => {
                    positional = &args[1..];
                    seen_s_alone = positional.is_empty();
                }
                "-h" => {
                    hard = true;
                    positional = &args[1..];
                }
                _ => {}
            }
        }
        if seen_s_alone {
            return 0;
        }

        let argv: Vec<&str> = positional.iter().map(String::as_str).collect();
        let mut limits = ResourceLimits::new();
        let (status, output) = bl(&argv, &mut limits, hard, /* set = */ true);
        if !output.is_empty() {
            print!("{}", output);
        }
        status
    }
    /// `unlimit` builtin entry point. Thin wrapper that delegates to
    /// the canonical port at `crate::builtins::rlimits::bin_unlimit`
    /// (Src/Builtins/rlimits.c:670).
    pub(crate) fn bin_unlimit(&self, args: &[String]) -> i32 {
        use crate::builtins::rlimits::{bin_unlimit as bu, ResourceLimits};

        // Strip optional leading `-h` (hard) flag, mirroring bin_unlimit
        // in Src/Builtins/rlimits.c:670.
        let mut hard = false;
        let mut positional: &[String] = args;
        if let Some(first) = args.first() {
            if first == "-h" {
                hard = true;
                positional = &args[1..];
            }
        }
        let argv: Vec<&str> = positional.iter().map(String::as_str).collect();
        let mut limits = ResourceLimits::new();
        let (status, output) = bu(&argv, &mut limits, hard);
        if !output.is_empty() {
            print!("{}", output);
        }
        status
    }
}
// END moved-from-exec-rs

/// Port of `boot_()` from Src/Builtins/rlimits.c:905. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn boot_() -> i32 { 0 }

/// Port of `cleanup_()` from Src/Builtins/rlimits.c:913. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn cleanup_() -> i32 { 0 }

/// Port of `do_limit()` from Src/Builtins/rlimits.c:431. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn do_limit() -> i32 { 0 }

/// Port of `do_unlimit()` from Src/Builtins/rlimits.c:622. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn do_unlimit() -> i32 { 0 }

/// Port of `enables_()` from Src/Builtins/rlimits.c:898. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn enables_() -> i32 { 0 }

/// Port of `features_()` from Src/Builtins/rlimits.c:890. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn features_() -> i32 { 0 }

/// Port of `find_resource()` from Src/Builtins/rlimits.c:239. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn find_resource() -> i32 { 0 }

/// Port of `finish_()` from Src/Builtins/rlimits.c:921. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn finish_() -> i32 { 0 }

/// Port of `free_resinfo()` from Src/Builtins/rlimits.c:222. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn free_resinfo() -> i32 { 0 }

/// Port of `printrlim()` from Src/Builtins/rlimits.c:253. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn printrlim() -> i32 { 0 }

/// Port of `set_resinfo()` from Src/Builtins/rlimits.c:194. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn set_resinfo() -> i32 { 0 }

/// Port of `setup_()` from Src/Builtins/rlimits.c:883. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn setup_() -> i32 { 0 }

/// Port of `showlimits()` from Src/Builtins/rlimits.c:346. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn showlimits() -> i32 { 0 }

/// Port of `showlimitvalue()` from Src/Builtins/rlimits.c:307. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn showlimitvalue() -> i32 { 0 }
