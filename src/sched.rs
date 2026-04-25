//! Scheduled command execution - port of Builtins/sched.c
//!
//! Provides the `sched` builtin for scheduling commands to run at specified times.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Flags for scheduled events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SchedFlags {
    pub trash_zle: bool,
}

/// A scheduled command
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

/// Scheduler for timed commands
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

    /// Add a scheduled command, maintaining time order
    pub fn add(&mut self, cmd: SchedCmd) {
        let pos = self
            .cmds
            .iter()
            .position(|c| c.time > cmd.time)
            .unwrap_or(self.cmds.len());
        self.cmds.insert(pos, cmd);
    }

    /// Remove a scheduled command by 1-based index
    pub fn remove(&mut self, index: usize) -> Option<SchedCmd> {
        if index == 0 || index > self.cmds.len() {
            return None;
        }
        Some(self.cmds.remove(index - 1))
    }

    /// Get all pending commands
    pub fn list(&self) -> &[SchedCmd] {
        &self.cmds
    }

    /// Check and return any commands due for execution
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

    /// Get the time until the next scheduled command (if any)
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

    /// Check if there are any scheduled commands
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    /// Get the number of scheduled commands
    pub fn len(&self) -> usize {
        self.cmds.len()
    }

    /// Clear all scheduled commands
    pub fn clear(&mut self) {
        self.cmds.clear();
    }

    /// Get scheduled events as array (for zsh_scheduled_events parameter)
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

/// Parse a time specification and return the absolute time
/// Supports:
/// - `+N` - N seconds from now
/// - `+H:M` - H hours and M minutes from now
/// - `+H:M:S` - H hours, M minutes, S seconds from now
/// - `H:M` - absolute time today (or tomorrow if past)
/// - `H:Ma` / `H:Mp` - absolute time with am/pm
/// - `N` - raw Unix timestamp
pub fn parse_time_spec(s: &str) -> Result<u64, &'static str> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();

    if s.starts_with('+') {
        let rest = &s[1..];

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

        let (mut hours, minutes, seconds, pm) = if let Some(second_colon) = after_hours.find(':') {
            let m: i64 = after_hours[..second_colon]
                .parse()
                .map_err(|_| "bad time specifier")?;
            let sec_str = &after_hours[second_colon + 1..];

            let (s_str, pm) = extract_ampm(sec_str);
            let s: i64 = s_str.parse().map_err(|_| "bad time specifier")?;
            (hours, m, s, pm)
        } else {
            let (m_str, pm) = extract_ampm(after_hours);
            let m: i64 = m_str.parse().map_err(|_| "bad time specifier")?;
            (hours, m, 0, pm)
        };

        if pm == Some(true) && hours < 12 {
            hours += 12;
        } else if pm == Some(false) && hours == 12 {
            hours = 0;
        }

        let today_midnight = get_today_midnight(now);
        let mut target = today_midnight + (hours * 3600 + minutes * 60 + seconds) as u64;

        if target < now {
            target += 24 * 3600;
        }

        Ok(target)
    } else {
        s.parse::<u64>().map_err(|_| "bad time specifier")
    }
}

fn extract_ampm(s: &str) -> (&str, Option<bool>) {
    let s_lower = s.to_lowercase();
    if s_lower.ends_with('p') || s_lower.starts_with("pm") || s_lower.contains('p') {
        let idx = s.to_lowercase().find('p').unwrap_or(s.len());
        (&s[..idx], Some(true))
    } else if s_lower.ends_with('a') || s_lower.starts_with("am") || s_lower.contains('a') {
        let idx = s.to_lowercase().find('a').unwrap_or(s.len());
        (&s[..idx], Some(false))
    } else {
        (s, None)
    }
}

fn get_today_midnight(now: u64) -> u64 {
    let secs_since_midnight = now % 86400;
    now - secs_since_midnight
}

/// Format a scheduled command for display
pub fn format_sched(index: usize, sch: &SchedCmd) -> String {
    use chrono::{Local, TimeZone};

    let dt = Local
        .timestamp_opt(sch.time as i64, 0)
        .single()
        .map(|dt| dt.format("%a %b %e %k:%M:%S").to_string())
        .unwrap_or_else(|| format!("{}", sch.time));

    let flagstr = if sch.flags.trash_zle { "-o " } else { "" };
    let endstr = if sch.cmd.starts_with('-') { "-- " } else { "" };

    format!("{:3} {} {}{}{}", index, dt, flagstr, endstr, sch.cmd)
}

/// Execute the sched builtin
/// Returns (exit_status, output)
pub fn builtin_sched(args: &[&str], scheduler: &mut Scheduler) -> (i32, String) {
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
        for (i, sch) in scheduler.list().iter().enumerate() {
            output.push_str(&format_sched(i + 1, sch));
            output.push('\n');
        }
        return (0, output);
    }

    if remaining.len() < 2 {
        return (1, "sched: not enough arguments\n".to_string());
    }

    let time_spec = remaining[0];
    let cmd = remaining[1..].join(" ");

    let time = match parse_time_spec(time_spec) {
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

        let result = parse_time_spec("+60").unwrap();
        assert!(result >= now + 59 && result <= now + 61);
    }

    #[test]
    fn test_parse_time_relative_hm() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = parse_time_spec("+1:30").unwrap();
        let expected = now + 3600 + 1800;
        assert!(result >= expected - 1 && result <= expected + 1);
    }

    #[test]
    fn test_parse_time_relative_hms() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = parse_time_spec("+1:30:15").unwrap();
        let expected = now + 3600 + 1800 + 15;
        assert!(result >= expected - 1 && result <= expected + 1);
    }

    #[test]
    fn test_parse_time_absolute_raw() {
        let result = parse_time_spec("1700000000").unwrap();
        assert_eq!(result, 1700000000);
    }

    #[test]
    fn test_builtin_sched_list_empty() {
        let mut sched = Scheduler::new();
        let (status, output) = builtin_sched(&[], &mut sched);
        assert_eq!(status, 0);
        assert!(output.is_empty());
    }

    #[test]
    fn test_builtin_sched_add() {
        let mut sched = Scheduler::new();
        let (status, _) = builtin_sched(&["+60", "echo", "hello"], &mut sched);
        assert_eq!(status, 0);
        assert_eq!(sched.len(), 1);
        assert_eq!(sched.list()[0].cmd, "echo hello");
    }

    #[test]
    fn test_builtin_sched_delete() {
        let mut sched = Scheduler::new();
        builtin_sched(&["+60", "echo", "hello"], &mut sched);
        assert_eq!(sched.len(), 1);

        let (status, _) = builtin_sched(&["-1"], &mut sched);
        assert_eq!(status, 0);
        assert!(sched.is_empty());
    }

    #[test]
    fn test_builtin_sched_not_enough_args() {
        let mut sched = Scheduler::new();
        let (status, output) = builtin_sched(&["+60"], &mut sched);
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
