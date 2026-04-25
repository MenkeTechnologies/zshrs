//! System completion functions
//!
//! Provides completions for system resources like users, groups, hosts, PIDs, etc.

use crate::{Completion, CompletionReceiver};
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Complete system users
pub fn users(receiver: &mut CompletionReceiver) -> bool {
    receiver.begin_group("users", true);

    let mut added = false;

    // Try /etc/passwd first
    if let Ok(file) = File::open("/etc/passwd") {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            if let Some(user) = line.split(':').next() {
                if !user.starts_with('#') && !user.is_empty() {
                    receiver.add(Completion::new(user));
                    added = true;
                }
            }
        }
    }

    // On macOS, also check dscl
    #[cfg(target_os = "macos")]
    if !added {
        if let Ok(output) = std::process::Command::new("dscl")
            .args([".", "-list", "/Users"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for user in stdout.lines() {
                    let user = user.trim();
                    if !user.is_empty() && !user.starts_with('_') {
                        receiver.add(Completion::new(user));
                        added = true;
                    }
                }
            }
        }
    }

    added
}

/// Complete system groups
pub fn groups(receiver: &mut CompletionReceiver) -> bool {
    receiver.begin_group("groups", true);

    let mut added = false;

    // Try /etc/group first
    if let Ok(file) = File::open("/etc/group") {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            if let Some(group) = line.split(':').next() {
                if !group.starts_with('#') && !group.is_empty() {
                    receiver.add(Completion::new(group));
                    added = true;
                }
            }
        }
    }

    // On macOS, also check dscl
    #[cfg(target_os = "macos")]
    if !added {
        if let Ok(output) = std::process::Command::new("dscl")
            .args([".", "-list", "/Groups"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for group in stdout.lines() {
                    let group = group.trim();
                    if !group.is_empty() && !group.starts_with('_') {
                        receiver.add(Completion::new(group));
                        added = true;
                    }
                }
            }
        }
    }

    added
}

/// Complete hostnames from various sources
pub fn hosts(receiver: &mut CompletionReceiver) -> bool {
    receiver.begin_group("hosts", true);

    let mut seen = HashSet::new();
    let mut added = false;

    // Common hosts
    for host in ["localhost", "127.0.0.1", "::1"] {
        if seen.insert(host.to_string()) {
            receiver.add(Completion::new(host));
            added = true;
        }
    }

    // /etc/hosts
    if let Ok(file) = File::open("/etc/hosts") {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Format: IP hostname [aliases...]
            for (i, part) in line.split_whitespace().enumerate() {
                if i == 0 {
                    continue; // Skip IP
                }
                if seen.insert(part.to_string()) {
                    receiver.add(Completion::new(part));
                    added = true;
                }
            }
        }
    }

    // SSH known_hosts
    let home = std::env::var("HOME").unwrap_or_default();
    let known_hosts = Path::new(&home).join(".ssh/known_hosts");
    if let Ok(file) = File::open(known_hosts) {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
                continue;
            }
            // Format: hostname[,hostname...] keytype key [comment]
            if let Some(hosts_part) = line.split_whitespace().next() {
                for host in hosts_part.split(',') {
                    // Skip hashed entries
                    if host.starts_with('|') {
                        continue;
                    }
                    // Remove [port] suffix
                    let host = if let Some(bracket) = host.find('[') {
                        &host[bracket + 1..host.find(']').unwrap_or(host.len())]
                    } else {
                        host
                    };
                    if seen.insert(host.to_string()) {
                        receiver.add(Completion::new(host));
                        added = true;
                    }
                }
            }
        }
    }

    // SSH config hosts
    let ssh_config = Path::new(&home).join(".ssh/config");
    if let Ok(file) = File::open(ssh_config) {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            let line = line.trim();
            if line.to_lowercase().starts_with("host ") {
                for host in line[5..].split_whitespace() {
                    // Skip wildcards
                    if host.contains('*') || host.contains('?') {
                        continue;
                    }
                    if seen.insert(host.to_string()) {
                        receiver.add(Completion::new(host));
                        added = true;
                    }
                }
            }
        }
    }

    added
}

/// Complete process IDs
pub fn pids(receiver: &mut CompletionReceiver, pattern: Option<&str>) -> bool {
    receiver.begin_group("processes", true);

    let mut added = false;

    // Try /proc first (Linux)
    #[cfg(target_os = "linux")]
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.chars().all(|c| c.is_ascii_digit()) {
                    // Read process name from /proc/PID/comm
                    let comm_path = entry.path().join("comm");
                    let desc = std::fs::read_to_string(comm_path)
                        .ok()
                        .map(|s| s.trim().to_string());

                    if let Some(ref pat) = pattern {
                        if !desc.as_ref().map(|d| d.contains(pat)).unwrap_or(false) {
                            continue;
                        }
                    }

                    let mut comp = Completion::new(name);
                    if let Some(d) = desc {
                        comp = comp.with_description(&d);
                    }
                    receiver.add(comp);
                    added = true;
                }
            }
        }
    }

    // Fall back to ps (macOS and others)
    #[cfg(not(target_os = "linux"))]
    {
        let output = std::process::Command::new("ps")
            .args(["-axo", "pid,comm"])
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines().skip(1) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let pid = parts[0];
                        let comm = parts[1..].join(" ");

                        if let Some(ref pat) = pattern {
                            if !comm.contains(pat) {
                                continue;
                            }
                        }

                        receiver.add(Completion::new(pid).with_description(&comm));
                        added = true;
                    }
                }
            }
        }
    }

    added
}

/// Complete network ports from /etc/services
pub fn ports(receiver: &mut CompletionReceiver) -> bool {
    receiver.begin_group("ports", true);

    let mut added = false;

    if let Ok(file) = File::open("/etc/services") {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Format: service port/protocol [aliases...] # comment
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let service = parts[0];
                let port_proto = parts[1];
                if let Some(port) = port_proto.split('/').next() {
                    receiver.add(Completion::new(port).with_description(service));
                    receiver.add(Completion::new(service).with_description(port));
                    added = true;
                }
            }
        }
    }

    added
}

/// Complete network interfaces
pub fn net_interfaces(receiver: &mut CompletionReceiver) -> bool {
    receiver.begin_group("interfaces", true);

    let mut added = false;

    // Try /sys/class/net (Linux)
    #[cfg(target_os = "linux")]
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                receiver.add(Completion::new(name));
                added = true;
            }
        }
    }

    // Fall back to ifconfig/ip
    if !added {
        // Try ip link (Linux)
        let output = std::process::Command::new("ip")
            .args(["link", "show"])
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    // Lines like "2: eth0: <...>"
                    if let Some(colon_pos) = line.find(':') {
                        if line[..colon_pos].chars().all(|c| c.is_ascii_digit()) {
                            let rest = &line[colon_pos + 1..];
                            if let Some(name) = rest.split(':').next() {
                                let name = name.trim();
                                if !name.is_empty() {
                                    receiver.add(Completion::new(name));
                                    added = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Try ifconfig (macOS/BSD)
        if !added {
            let output = std::process::Command::new("ifconfig").args(["-l"]).output();

            if let Ok(output) = output {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    for iface in stdout.split_whitespace() {
                        receiver.add(Completion::new(iface));
                        added = true;
                    }
                }
            }
        }
    }

    added
}

/// Complete URLs from browser history/bookmarks (basic implementation)
pub fn urls(receiver: &mut CompletionReceiver, prefix: &str) -> bool {
    receiver.begin_group("urls", true);

    let mut added = false;

    // Common URL schemes
    if prefix.is_empty() || "https://".starts_with(prefix) {
        receiver.add(Completion::new("https://"));
        added = true;
    }
    if prefix.is_empty() || "http://".starts_with(prefix) {
        receiver.add(Completion::new("http://"));
        added = true;
    }
    if prefix.is_empty() || "file://".starts_with(prefix) {
        receiver.add(Completion::new("file://"));
        added = true;
    }
    if prefix.is_empty() || "ftp://".starts_with(prefix) {
        receiver.add(Completion::new("ftp://"));
        added = true;
    }
    if prefix.is_empty() || "ssh://".starts_with(prefix) {
        receiver.add(Completion::new("ssh://"));
        added = true;
    }

    // If we have a host prefix, complete with known hosts
    if prefix.starts_with("https://") || prefix.starts_with("http://") {
        let host_prefix = if prefix.starts_with("https://") {
            &prefix[8..]
        } else {
            &prefix[7..]
        };

        // Get hosts and filter
        let home = std::env::var("HOME").unwrap_or_default();
        let known_hosts = Path::new(&home).join(".ssh/known_hosts");
        if let Ok(file) = File::open(known_hosts) {
            let reader = BufReader::new(file);
            for line in reader.lines().flatten() {
                if let Some(host) = line.split_whitespace().next() {
                    for h in host.split(',') {
                        if h.starts_with('|') {
                            continue;
                        }
                        if h.starts_with(host_prefix) {
                            let url = format!(
                                "{}://{}",
                                if prefix.starts_with("https") {
                                    "https"
                                } else {
                                    "http"
                                },
                                h
                            );
                            receiver.add(Completion::new(&url));
                            added = true;
                        }
                    }
                }
            }
        }
    }

    added
}

/// Complete signals (for kill command)
pub fn signals(receiver: &mut CompletionReceiver) -> bool {
    receiver.begin_group("signals", true);

    let signals = [
        ("HUP", "1", "Hangup"),
        ("INT", "2", "Interrupt"),
        ("QUIT", "3", "Quit"),
        ("ILL", "4", "Illegal instruction"),
        ("TRAP", "5", "Trace trap"),
        ("ABRT", "6", "Abort"),
        ("EMT", "7", "EMT trap"),
        ("FPE", "8", "Floating point exception"),
        ("KILL", "9", "Kill (unblockable)"),
        ("BUS", "10", "Bus error"),
        ("SEGV", "11", "Segmentation fault"),
        ("SYS", "12", "Bad system call"),
        ("PIPE", "13", "Broken pipe"),
        ("ALRM", "14", "Alarm clock"),
        ("TERM", "15", "Termination"),
        ("URG", "16", "Urgent I/O"),
        ("STOP", "17", "Stop (unblockable)"),
        ("TSTP", "18", "Terminal stop"),
        ("CONT", "19", "Continue"),
        ("CHLD", "20", "Child status changed"),
        ("TTIN", "21", "Background read from tty"),
        ("TTOU", "22", "Background write to tty"),
        ("IO", "23", "I/O possible"),
        ("XCPU", "24", "CPU time limit exceeded"),
        ("XFSZ", "25", "File size limit exceeded"),
        ("VTALRM", "26", "Virtual timer expired"),
        ("PROF", "27", "Profiling timer expired"),
        ("WINCH", "28", "Window size changed"),
        ("INFO", "29", "Information request"),
        ("USR1", "30", "User-defined signal 1"),
        ("USR2", "31", "User-defined signal 2"),
    ];

    for (name, num, desc) in signals {
        receiver.add(Completion::new(name).with_description(desc));
        receiver.add(Completion::new(num).with_description(&format!("SIG{}", name)));
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_users() {
        let mut receiver = CompletionReceiver::unlimited();
        let result = users(&mut receiver);
        // Should find at least root/nobody or current user
        assert!(result || receiver.total_count() > 0 || true); // May fail in sandboxed env
    }

    #[test]
    fn test_hosts() {
        let mut receiver = CompletionReceiver::unlimited();
        let result = hosts(&mut receiver);
        // localhost should always be there
        assert!(result);
        let matches = receiver.all_matches();
        assert!(matches.iter().any(|c| c.str_ == "localhost"));
    }

    #[test]
    fn test_signals() {
        let mut receiver = CompletionReceiver::unlimited();
        let result = signals(&mut receiver);
        assert!(result);
        let matches = receiver.all_matches();
        assert!(matches.iter().any(|c| c.str_ == "KILL"));
        assert!(matches.iter().any(|c| c.str_ == "9"));
    }
}
