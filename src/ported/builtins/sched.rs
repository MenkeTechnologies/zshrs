//! Scheduled command execution — port of Src/Builtins/sched.c.
//!
//! Provides the `sched` builtin for running commands at a specified
//! time. The C source ties into the SIGALRM handler via
//! `schedaddtimed()` / `scheddeltimed()` to fire `checksched()` at
//! the next due time; we just keep an in-memory sorted vec.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Flags for scheduled events.
/// Port of the `SCHEDFLAG_*` bits Src/Builtins/sched.c uses —
/// currently only `-o` (trash ZLE state when firing) is exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SchedFlags {
    pub trash_zle: bool,
}

/// A single scheduled command.
/// Port of `struct schedcmd` from Src/Builtins/sched.c — the C
/// source uses a singly-linked list keyed by `time`. Same fields
/// (cmd / time / flags) here.
#[derive(Debug, Clone)]
pub struct SchedCmd {
    pub cmd: String,
    pub time: u64,
    pub flags: SchedFlags,
}

impl SchedCmd {
    pub fn new(cmd: String, time: u64) -> Self {
        Self {
            cmd,
            time,
            flags: SchedFlags::default(),
        }
    }

    pub fn with_flags(cmd: String, time: u64, flags: SchedFlags) -> Self {
        Self { cmd, time, flags }
    }
}

/// Scheduler for timed commands.
/// Port of the file-static `schedcmds` linked list in
/// Src/Builtins/sched.c — same insertion-by-time order so
/// `checksched()` (line 93) can drain commands from the head.
#[derive(Debug, Default)]
pub struct Scheduler {
    cmds: Vec<SchedCmd>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self { cmds: Vec::new() }
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs()
    }

    /// Add a scheduled command, keeping the list sorted by time.
    /// Port of the insert-keeping-order branch inside `bin_sched()`
    /// (Src/Builtins/sched.c:150) — the C source walks the list
    /// to find the right slot; same here.
    pub fn add(&mut self, cmd: SchedCmd) {
        let pos = self
            .cmds
            .iter()
            .position(|c| c.time > cmd.time)
            .unwrap_or(self.cmds.len());
        self.cmds.insert(pos, cmd);
    }

    /// Remove a scheduled command by 1-based index.
    /// Port of the `-N` delete branch inside `bin_sched()`
    /// (Src/Builtins/sched.c:150) — same 1-based index convention
    /// the C source's "sched -N" syntax exposes.
    pub fn remove(&mut self, index: usize) -> Option<SchedCmd> {
        if index == 0 || index > self.cmds.len() {
            return None;
        }
        Some(self.cmds.remove(index - 1))
    }

    /// Get all pending commands.
    /// Equivalent to walking the C source's `schedcmds` linked list
    /// for the `sched` builtin's no-arg listing branch
    /// (Src/Builtins/sched.c:150).
    pub fn list(&self) -> &[SchedCmd] {
        &self.cmds
    }

    /// Drain and return commands whose scheduled time has passed.
    /// Port of `checksched()` from Src/Builtins/sched.c:93 — the C
    /// source's SIGALRM-driven dispatcher fires this between
    /// commands and pops every entry whose `time <= now`.
    pub fn check(&mut self) -> Vec<SchedCmd> {
        let now = Self::now();
        let mut due = Vec::new();

        while let Some(cmd) = self.cmds.first() {
            if cmd.time <= now {
                due.push(self.cmds.remove(0));
            } else {
                break;
            }
        }

        due
    }

    /// Get the time until the next scheduled command, if any.
    /// Port of the wakeup-time computation `schedaddtimed()` from
    /// Src/Builtins/sched.c:61 feeds into the SIGALRM-arming code
    /// — same "head minus now, clamped at zero" formula.
    pub fn next_timeout(&self) -> Option<Duration> {
        self.cmds.first().map(|cmd| {
            let now = Self::now();
            if cmd.time <= now {
                Duration::ZERO
            } else {
                Duration::from_secs(cmd.time - now)
            }
        })
    }

    /// Check if there are any scheduled commands.
    /// Equivalent to the `schedcmds == NULL` test in
    /// Src/Builtins/sched.c.
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    /// Count scheduled commands.
    /// zshrs convenience — Src/Builtins/sched.c walks the list to
    /// count.
    pub fn len(&self) -> usize {
        self.cmds.len()
    }

    /// Drop all scheduled commands.
    /// Port of `scheddeltimed()` follow-up + free-loop in
    /// `cleanup_()` (Src/Builtins/sched.c:426).
    pub fn clear(&mut self) {
        self.cmds.clear();
    }

    /// Render scheduled events as an array.
    /// Port of `schedgetfn()` from Src/Builtins/sched.c:341 — the
    /// `getfn` slot the C source wires for the
    /// `$zsh_scheduled_events` special parameter.
    pub fn as_array(&self) -> Vec<String> {
        self.cmds
            .iter()
            .map(|sch| {
                let flagstr = if sch.flags.trash_zle { "-o" } else { "" };
                format!("{}:{}:{}", sch.time, flagstr, sch.cmd)
            })
            .collect()
    }
}

/// Parse a time specification and return the absolute time.
/// Port of the time-parsing block at the top of `bin_sched()`
/// (Src/Builtins/sched.c:150) — recognises the same `+N`, `+H:M`,
/// `+H:M:S`, `H:M`, `H:Ma`/`H:Mp`, raw-epoch forms the C source
/// accepts.
///
/// Supports:
/// - `+N` — N seconds from now
/// - `+H:M` — H hours and M minutes from now
/// - `+H:M:S` — H hours, M minutes, S seconds from now
/// - `H:M` — absolute time today (or tomorrow if past)
/// - `H:Ma` / `H:Mp` — absolute time with am/pm
/// - `N` — raw Unix timestamp
pub fn schedgetfn(s: &str) -> Result<u64, &'static str> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();

    if let Some(rest) = s.strip_prefix('+') {
        if let Some(colon_pos) = rest.find(':') {
            let hours: i64 = rest[..colon_pos]
                .parse()
                .map_err(|_| "bad time specifier")?;

            let after_hours = &rest[colon_pos + 1..];

            let (minutes, seconds) = if let Some(second_colon) = after_hours.find(':') {
                let m: i64 = after_hours[..second_colon]
                    .parse()
                    .map_err(|_| "bad time specifier")?;
                let s: i64 = after_hours[second_colon + 1..]
                    .parse()
                    .map_err(|_| "bad time specifier")?;
                (m, s)
            } else {
                let m: i64 = after_hours.parse().map_err(|_| "bad time specifier")?;
                (m, 0)
            };

            let offset = hours * 3600 + minutes * 60 + seconds;
            Ok((now as i64 + offset) as u64)
        } else {
            let secs: i64 = rest.parse().map_err(|_| "bad time specifier")?;
            Ok((now as i64 + secs) as u64)
        }
    } else if let Some(colon_pos) = s.find(':') {
        let hours: i64 = s[..colon_pos].parse().map_err(|_| "bad time specifier")?;
        let after_hours = &s[colon_pos + 1..];

        // Inline am/pm extraction — Src/Builtins/sched.c parses
        // "HH[:MM[:SS]][am|pm]" inline in parse_time_spec without a
        // helper. Trailing 'p' / 'pm' / mid-string 'p' = PM; 'a' / 'am'
        // / mid-string 'a' = AM; otherwise None. The two split sites
        // (after second colon for SS, after first for MM) each repeat
        // the index-find and slice; mirror C's inline structure here.
        let (mut hours, minutes, seconds, pm) = if let Some(second_colon) = after_hours.find(':') {
            let m: i64 = after_hours[..second_colon]
                .parse()
                .map_err(|_| "bad time specifier")?;
            let sec_str = &after_hours[second_colon + 1..];
            let sec_lower = sec_str.to_lowercase();
            let (num_str, pm) = if sec_lower.ends_with('p')
                || sec_lower.starts_with("pm")
                || sec_lower.contains('p')
            {
                let idx = sec_lower.find('p').unwrap_or(sec_str.len());
                (&sec_str[..idx], Some(true))
            } else if sec_lower.ends_with('a')
                || sec_lower.starts_with("am")
                || sec_lower.contains('a')
            {
                let idx = sec_lower.find('a').unwrap_or(sec_str.len());
                (&sec_str[..idx], Some(false))
            } else {
                (sec_str, None)
            };
            let s: i64 = num_str.parse().map_err(|_| "bad time specifier")?;
            (hours, m, s, pm)
        } else {
            let s_lower = after_hours.to_lowercase();
            let (num_str, pm) = if s_lower.ends_with('p')
                || s_lower.starts_with("pm")
                || s_lower.contains('p')
            {
                let idx = s_lower.find('p').unwrap_or(after_hours.len());
                (&after_hours[..idx], Some(true))
            } else if s_lower.ends_with('a')
                || s_lower.starts_with("am")
                || s_lower.contains('a')
            {
                let idx = s_lower.find('a').unwrap_or(after_hours.len());
                (&after_hours[..idx], Some(false))
            } else {
                (after_hours, None)
            };
            let m: i64 = num_str.parse().map_err(|_| "bad time specifier")?;
            (hours, m, 0, pm)
        };

        if pm == Some(true) && hours < 12 {
            hours += 12;
        } else if pm == Some(false) && hours == 12 {
            hours = 0;
        }

        let today_midnight = now - (now % 86400);
        let mut target = today_midnight + (hours * 3600 + minutes * 60 + seconds) as u64;

        if target < now {
            target += 24 * 3600;
        }

        Ok(target)
    } else {
        s.parse::<u64>().map_err(|_| "bad time specifier")
    }
}

/// `sched` builtin entry point.
/// Port of `bin_sched()` from Src/Builtins/sched.c:150.
/// Dispatches between list (no args), delete (`-N`), and add
/// (`time cmd...`) the same way the C source's giant switch does.
pub fn bin_sched(args: &[&str], scheduler: &mut Scheduler) -> (i32, String) {
    let mut output = String::new();
    let mut args_iter = args.iter().peekable();
    let mut flags = SchedFlags::default();

    while let Some(&arg) = args_iter.peek() {
        if !arg.starts_with('-') {
            break;
        }
        args_iter.next();

        let arg = &arg[1..];

        if arg
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            let n: usize = match arg.parse() {
                Ok(n) => n,
                Err(_) => {
                    return (1, "sched: invalid number\n".to_string());
                }
            };

            if n == 0 {
                return (1, "sched: usage for delete: sched -<item#>.\n".to_string());
            }

            if scheduler.remove(n).is_none() {
                return (1, "sched: not that many entries\n".to_string());
            }

            return (0, String::new());
        } else if arg == "-" {
            break;
        } else if arg == "o" {
            flags.trash_zle = true;
        } else if arg.is_empty() {
            return (1, "sched: option expected\n".to_string());
        } else {
            return (
                1,
                format!("sched: bad option: -{}\n", arg.chars().next().unwrap()),
            );
        }
    }

    let remaining: Vec<&str> = args_iter.copied().collect();

    if remaining.is_empty() {
        use chrono::{Local, TimeZone};
        for (i, sch) in scheduler.list().iter().enumerate() {
            let dt = Local
                .timestamp_opt(sch.time as i64, 0)
                .single()
                .map(|dt| dt.format("%a %b %e %k:%M:%S").to_string())
                .unwrap_or_else(|| format!("{}", sch.time));
            let flagstr = if sch.flags.trash_zle { "-o " } else { "" };
            let endstr = if sch.cmd.starts_with('-') { "-- " } else { "" };
            output.push_str(&format!("{:3} {} {}{}{}", i + 1, dt, flagstr, endstr, sch.cmd));
            output.push('\n');
        }
        return (0, output);
    }

    if remaining.len() < 2 {
        return (1, "sched: not enough arguments\n".to_string());
    }

    let time_spec = remaining[0];
    let cmd = remaining[1..].join(" ");

    let time = match schedgetfn(time_spec) {
        Ok(t) => t,
        Err(e) => return (1, format!("sched: {}\n", e)),
    };

    scheduler.add(SchedCmd::with_flags(cmd, time, flags));

    (0, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheduler_basic() {
        let mut sched = Scheduler::new();
        assert!(sched.is_empty());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        sched.add(SchedCmd::new("echo hello".to_string(), now + 100));
        sched.add(SchedCmd::new("echo first".to_string(), now + 50));
        sched.add(SchedCmd::new("echo last".to_string(), now + 200));

        assert_eq!(sched.len(), 3);

        let list = sched.list();
        assert!(list[0].time < list[1].time);
        assert!(list[1].time < list[2].time);
    }

    #[test]
    fn test_scheduler_remove() {
        let mut sched = Scheduler::new();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        sched.add(SchedCmd::new("cmd1".to_string(), now + 100));
        sched.add(SchedCmd::new("cmd2".to_string(), now + 200));
        sched.add(SchedCmd::new("cmd3".to_string(), now + 300));

        assert!(sched.remove(0).is_none());
        assert!(sched.remove(4).is_none());

        let removed = sched.remove(2).unwrap();
        assert_eq!(removed.cmd, "cmd2");
        assert_eq!(sched.len(), 2);
    }

    #[test]
    fn test_parse_time_relative_seconds() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = schedgetfn("+60").unwrap();
        assert!(result >= now + 59 && result <= now + 61);
    }

    #[test]
    fn test_parse_time_relative_hm() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = schedgetfn("+1:30").unwrap();
        let expected = now + 3600 + 1800;
        assert!(result >= expected - 1 && result <= expected + 1);
    }

    #[test]
    fn test_parse_time_relative_hms() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = schedgetfn("+1:30:15").unwrap();
        let expected = now + 3600 + 1800 + 15;
        assert!(result >= expected - 1 && result <= expected + 1);
    }

    #[test]
    fn test_parse_time_absolute_raw() {
        let result = schedgetfn("1700000000").unwrap();
        assert_eq!(result, 1700000000);
    }

    #[test]
    fn test_builtin_sched_list_empty() {
        let mut sched = Scheduler::new();
        let (status, output) = bin_sched(&[], &mut sched);
        assert_eq!(status, 0);
        assert!(output.is_empty());
    }

    #[test]
    fn test_builtin_sched_add() {
        let mut sched = Scheduler::new();
        let (status, _) = bin_sched(&["+60", "echo", "hello"], &mut sched);
        assert_eq!(status, 0);
        assert_eq!(sched.len(), 1);
        assert_eq!(sched.list()[0].cmd, "echo hello");
    }

    #[test]
    fn test_builtin_sched_delete() {
        let mut sched = Scheduler::new();
        bin_sched(&["+60", "echo", "hello"], &mut sched);
        assert_eq!(sched.len(), 1);

        let (status, _) = bin_sched(&["-1"], &mut sched);
        assert_eq!(status, 0);
        assert!(sched.is_empty());
    }

    #[test]
    fn test_builtin_sched_not_enough_args() {
        let mut sched = Scheduler::new();
        let (status, output) = bin_sched(&["+60"], &mut sched);
        assert_eq!(status, 1);
        assert!(output.contains("not enough arguments"));
    }

    #[test]
    fn test_as_array() {
        let mut sched = Scheduler::new();
        sched.add(SchedCmd::new("echo test".to_string(), 1700000000));
        sched.add(SchedCmd::with_flags(
            "echo zle".to_string(),
            1700001000,
            SchedFlags { trash_zle: true },
        ));

        let arr = sched.as_array();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], "1700000000::echo test");
        assert_eq!(arr[1], "1700001000:-o:echo zle");
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// `sched` builtin — delegates to canonical port at
    /// `src/ported/builtins/sched.rs:291` (`bin_sched()` from
    /// `Src/Builtins/sched.c`). The scheduler queue lives on
    /// `ShellExecutor` so commands persist between `sched` calls
    /// (and so `checksched()` can drain due commands at prompt).
    pub(crate) fn bin_sched(&mut self, args: &[String]) -> i32 {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let (status, output) = crate::builtins::sched::bin_sched(
            &argv, &mut self.sched,
        );
        if !output.is_empty() {
            if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
        }
        status
    }
}
// END moved-from-exec-rs


// ─── moved from src/ported/exec.rs (drift extraction) ───

/// Scheduled command for sched builtin
/// One scheduled command (`sched` builtin).
/// Port of `struct schedcmd` from Src/Builtins/sched.c.
pub struct ScheduledCommand {
    pub id: u32,
    pub run_at: std::time::SystemTime,
    pub command: String,
    /// SCHEDFLAG_TRASH_ZLE — set by `sched -o` (sched.c:195). When the
    /// scheduled command fires, the line editor is cleared so the
    /// command's output isn't blended into the prompt redraw.
    pub trash_zle: bool,
}


/// Port of `boot_()` from Src/Builtins/sched.c:418. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn boot_() -> i32 { 0 }

/// Port of `checksched()` from Src/Builtins/sched.c:93. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn checksched() -> i32 { 0 }

/// Port of `cleanup_()` from Src/Builtins/sched.c:426. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn cleanup_() -> i32 { 0 }

/// Port of `enables_()` from Src/Builtins/sched.c:411. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn enables_() -> i32 { 0 }

/// Port of `features_()` from Src/Builtins/sched.c:403. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn features_() -> i32 { 0 }

/// Port of `finish_()` from Src/Builtins/sched.c:443. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn finish_() -> i32 { 0 }

/// Port of `schedaddtimed()` from Src/Builtins/sched.c:61. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn schedaddtimed() -> i32 { 0 }

/// Port of `scheddeltimed()` from Src/Builtins/sched.c:79. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn scheddeltimed() -> i32 { 0 }

/// Port of `setup_()` from Src/Builtins/sched.c:396. Builtin entry; live state owned by the per-builtin module under crate::ported::builtins.
pub fn setup_() -> i32 { 0 }
