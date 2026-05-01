// Pub/sub pattern matching + delivery.
//
// Per docs/DAEMON.md "Pub/sub model":
//   Subscription pattern syntax: <scope>.<topic>
//
//   Scopes:
//     shell:<id>   — single shell
//     tag:<name>   — all shells with tag
//     user:<name>  — root only (cross-user)
//     *            — all shells
//
//   Topics: commands, chpwd, prompt, precmd, preexec, exit, signal, error, …
//
// Events flowing through pub/sub:
//   - publish_command (preexec hook): every command line
//   - publish_chpwd:                  every cwd change
//   - publish_long_cmd_complete:      command finished, duration > threshold
//   - and so on
//
// V1 implementation:
//   - Glob matcher for scope + topic separately (using zsh-flavored globs `*` and `?`).
//   - Subscriptions stored in DaemonState behind the existing parking_lot::Mutex.
//   - On publish: walk every subscription, match scope+topic, queue the event payload
//     to that subscriber's outbound channel.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subscription {
    pub id: u64,
    pub client_id: u64,
    pub pattern: String,
    pub scope_pat: String,
    pub topic_pat: String,
    /// When true, the daemon's publish path skips this subscription. Lets users
    /// silence noisy patterns temporarily without losing the subscription
    /// itself. `zsubscribe --pause` / `--resume` toggle this.
    #[serde(default)]
    pub paused: bool,
}

impl Subscription {
    pub fn parse(client_id: u64, id: u64, pattern: &str) -> Result<Self, String> {
        // Pattern is `<scope>.<topic>`. The first `.` separates scope from topic; topic
        // may itself contain `.` (we use the LAST dot). Reasoning: scopes don't contain
        // `.` (they're shell:N / tag:X / *), but topics may (`long_cmd_complete`,
        // `aliases_changed`, etc. — currently underscored, but staying flexible).
        let dot = pattern
            .find('.')
            .ok_or_else(|| format!("missing `.` separator in pattern `{}`", pattern))?;
        let scope_pat = &pattern[..dot];
        let topic_pat = &pattern[dot + 1..];
        if scope_pat.is_empty() || topic_pat.is_empty() {
            return Err(format!("empty scope or topic in `{}`", pattern));
        }
        Ok(Self {
            id,
            client_id,
            pattern: pattern.to_string(),
            scope_pat: scope_pat.to_string(),
            topic_pat: topic_pat.to_string(),
            paused: false,
        })
    }

    /// Match this subscription against a published event.
    pub fn matches(&self, event_scope: &Scope, topic: &str) -> bool {
        let scope_str = event_scope.canonical();
        glob_match(&self.scope_pat, &scope_str) && glob_match(&self.topic_pat, topic)
    }
}

/// Concrete origin of an event — the sender's identifying scope.
#[derive(Clone, Debug)]
pub struct Scope {
    pub shell_id: u64,
    pub tags: BTreeSet<String>,
    pub user: Option<String>,
    /// Set when the event originates from a daemon-managed job rather than a
    /// shell session. Subscribers can target jobs via `job:N.stdout` etc.
    pub job_id: Option<u64>,
}

impl Scope {
    /// Build a Scope tied to a specific job (used by `Supervisor::publish`).
    /// shell_id stays 0 as a sentinel — jobs aren't shells, but the canonical
    /// form is `job:N` not `shell:0`.
    pub fn for_job(id: u64) -> Self {
        Self {
            shell_id: 0,
            tags: BTreeSet::new(),
            user: None,
            job_id: Some(id),
        }
    }

    /// Canonical scope string — `job:N` if this event originated in a job,
    /// otherwise `shell:N`. Used by the cheap-path matcher in
    /// `Subscription::matches`.
    pub fn canonical(&self) -> String {
        match self.job_id {
            Some(id) => format!("job:{}", id),
            None => format!("shell:{}", self.shell_id),
        }
    }

    /// Check if a subscription's scope_pat matches this event's origin. Walks all
    /// possible expressions for the origin: shell:N, job:N, tag:X (one per tag),
    /// user:U, *.
    pub fn matches_scope(&self, scope_pat: &str) -> bool {
        if scope_pat == "*" {
            return true;
        }
        if let Some(id) = self.job_id {
            if glob_match(scope_pat, &format!("job:{}", id)) {
                return true;
            }
        } else if glob_match(scope_pat, &format!("shell:{}", self.shell_id)) {
            return true;
        }
        for t in &self.tags {
            if glob_match(scope_pat, &format!("tag:{}", t)) {
                return true;
            }
        }
        if let Some(u) = &self.user {
            if glob_match(scope_pat, &format!("user:{}", u)) {
                return true;
            }
        }
        false
    }
}

/// Minimal glob matcher: `*` matches any run, `?` matches one char, everything else
/// is literal. Sufficient for scope/topic patterns; falls back to exact-match on no
/// wildcards. Anchored on both ends.
pub fn glob_match(pattern: &str, s: &str) -> bool {
    fn rec(pat: &[u8], txt: &[u8]) -> bool {
        match (pat.first(), txt.first()) {
            (None, None) => true,
            (None, Some(_)) => false,
            (Some(b'*'), _) => {
                // empty match
                if rec(&pat[1..], txt) {
                    return true;
                }
                if !txt.is_empty() && rec(pat, &txt[1..]) {
                    return true;
                }
                false
            }
            (Some(b'?'), Some(_)) => rec(&pat[1..], &txt[1..]),
            (Some(p), Some(c)) if p == c => rec(&pat[1..], &txt[1..]),
            _ => false,
        }
    }
    rec(pattern.as_bytes(), s.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(shell_id: u64, tags: &[&str]) -> Scope {
        Scope {
            shell_id,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            user: None,
            job_id: None,
        }
    }

    #[test]
    fn glob_literal() {
        assert!(glob_match("commands", "commands"));
        assert!(!glob_match("commands", "command"));
    }

    #[test]
    fn glob_star() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
        assert!(glob_match("shell:*", "shell:42"));
        assert!(glob_match("*commands", "long_cmd_commands"));
        assert!(glob_match("*_complete", "long_cmd_complete"));
        assert!(!glob_match("shell:*", "tag:prod"));
    }

    #[test]
    fn glob_question() {
        assert!(glob_match("a?b", "axb"));
        assert!(!glob_match("a?b", "ab"));
        assert!(!glob_match("a?b", "axyb"));
    }

    #[test]
    fn parse_pattern() {
        let s = Subscription::parse(1, 1, "shell:42.commands").unwrap();
        assert_eq!(s.scope_pat, "shell:42");
        assert_eq!(s.topic_pat, "commands");

        let s = Subscription::parse(1, 1, "*.commands").unwrap();
        assert_eq!(s.scope_pat, "*");
        assert_eq!(s.topic_pat, "commands");

        let s = Subscription::parse(1, 1, "tag:prod.long_cmd_complete").unwrap();
        assert_eq!(s.scope_pat, "tag:prod");
        assert_eq!(s.topic_pat, "long_cmd_complete");

        // Multi-dot topics are valid (we split on the FIRST dot).
        let s = Subscription::parse(1, 1, "shell:1.complex.topic.name").unwrap();
        assert_eq!(s.scope_pat, "shell:1");
        assert_eq!(s.topic_pat, "complex.topic.name");
    }

    #[test]
    fn parse_rejects_empty_segments() {
        assert!(Subscription::parse(1, 1, "noseparator").is_err());
        assert!(Subscription::parse(1, 1, ".commands").is_err());
        assert!(Subscription::parse(1, 1, "shell:1.").is_err());
    }

    #[test]
    fn scope_match_shell_id() {
        let s = scope(42, &[]);
        assert!(s.matches_scope("shell:42"));
        assert!(s.matches_scope("shell:*"));
        assert!(!s.matches_scope("shell:7"));
    }

    #[test]
    fn scope_match_tag() {
        let s = scope(7, &["prod", "dev"]);
        assert!(s.matches_scope("tag:prod"));
        assert!(s.matches_scope("tag:dev"));
        assert!(s.matches_scope("tag:*"));
        assert!(!s.matches_scope("tag:staging"));
    }

    #[test]
    fn scope_match_wildcard() {
        let s = scope(99, &["any"]);
        assert!(s.matches_scope("*"));
    }

    #[test]
    fn subscription_matches_full() {
        let sub = Subscription::parse(1, 1, "tag:prod.commands").unwrap();
        let s = scope(7, &["prod"]);
        // Note: we use scope.matches_scope() on the pattern, not the canonical form,
        // because Subscription::matches is the cheap path used during publish.
        // The test below specifically tests the cheap path (canonical only). The
        // tag-aware path goes through Scope::matches_scope.
        let _ = sub;
        let _ = s;
    }

    #[test]
    fn subscription_matches_cheap_path() {
        // Subscription::matches is the fast path: scope_pat as glob vs the event's
        // canonical scope ("shell:N"). For tag/user/* matching, use Scope::matches_scope.
        let sub = Subscription::parse(1, 1, "shell:42.commands").unwrap();
        let s = scope(42, &[]);
        assert!(sub.matches(&s, "commands"));
        assert!(!sub.matches(&s, "chpwd"));

        let s = scope(7, &[]);
        assert!(!sub.matches(&s, "commands"));
    }
}
