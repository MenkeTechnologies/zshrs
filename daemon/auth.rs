//! HTTP bearer-token + scope-based authorization. Per
//! docs/DAEMON_AS_SERVICE.md §"Authentication / authorization".
//!
//! Backward compat is the design constraint: `[http.tokens]` configs
//! that predate this module (`name = "secret"` flat strings) keep
//! working unchanged and grant full access. New configs can opt in
//! to scoped tokens via the table form
//! (`name = { token = "...", scopes = [...] }`); the daemon then
//! enforces the scope set per op.
//!
//! Scope namespaces are flat `<area>.<verb>` strings (e.g. `cache.put`,
//! `defs.read`, `meta.admin`). Wildcards in patterns: `*` (everything),
//! `<area>.*` (every verb in an area), `*.read` (every read across
//! areas). `op_scope()` is the authoritative op→required-scope table;
//! ops not listed default to `meta.admin` (deny-by-default for any new
//! op until it's explicitly mapped).
//!
//! The daemon is single-user-by-default — the typical config has
//! ZERO tokens and the HTTP listener is loopback-only. Scopes only
//! exist for the multi-client deployment scenarios in
//! docs/DAEMON_AS_SERVICE.md (CI pipelines, editor LSPs, dashboard
//! tokens). For solo use, leave `[http.tokens]` empty.

use std::sync::Arc;

/// One configured bearer token.
#[derive(Debug, Clone)]
pub struct Token {
    /// The label from `[http.tokens]` (e.g. "vim-lsp", "ci-pipeline").
    /// Used in error messages + access logs only; not checked against
    /// the wire.
    pub label: String,
    /// The bearer secret sent in `Authorization: Bearer <secret>`.
    pub secret: String,
    /// Allowed scopes. Empty = full access (legacy / unscoped token).
    pub scopes: ScopeMatcher,
}

impl Token {
    /// Does this token grant access to `op_scope`? Empty pattern set
    /// = unscoped legacy token = always true.
    pub fn allows(&self, op_scope: &str) -> bool {
        if self.scopes.patterns.is_empty() {
            return true;
        }
        self.scopes.matches(op_scope)
    }

    /// Render the granted scopes as a JSON-friendly list (used in the
    /// 403 response body when a scope check fails so the caller can
    /// see what's available).
    pub fn granted_scopes(&self) -> Vec<String> {
        if self.scopes.patterns.is_empty() {
            vec!["*".to_string()]
        } else {
            self.scopes.patterns.clone()
        }
    }
}

/// Patterns the token grants. Empty list = full access.
///
/// Pattern shapes (lowest-precedence-first; first match wins):
///   `*`            → every op
///   `<area>.*`     → every verb in `<area>` (`cache.*`, `job.*`, …)
///   `*.<verb>`     → every area's `<verb>` (`*.read`, `*.write`)
///   `<area>.<verb>` → exact match
///
/// No nested globs, no regex — keeping the matcher predictable + the
/// config readable is more important than catching every possible
/// access pattern.
#[derive(Debug, Clone, Default)]
pub struct ScopeMatcher {
    pub patterns: Vec<String>,
}

impl ScopeMatcher {
    pub fn from_strings<I, S>(iter: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            patterns: iter.into_iter().map(Into::into).collect(),
        }
    }

    pub fn matches(&self, op_scope: &str) -> bool {
        for p in &self.patterns {
            if p == "*" || p == op_scope {
                return true;
            }
            if let Some(prefix) = p.strip_suffix(".*") {
                // `<area>.*` matches `<area>.<verb>` (NOT `<area>` alone).
                if let Some(rest) = op_scope.strip_prefix(prefix) {
                    if rest.starts_with('.') {
                        return true;
                    }
                }
            }
            if let Some(suffix) = p.strip_prefix("*.") {
                // `*.<verb>` matches `<area>.<verb>`.
                if let Some(rest) = op_scope.rsplit_once('.') {
                    if rest.1 == suffix {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Set of configured tokens, indexed by secret for O(1) lookup on the
/// hot path. Wraps `Arc<Vec<Token>>` so the http handler can clone the
/// state without cloning every token.
#[derive(Debug, Clone, Default)]
pub struct TokenRegistry {
    tokens: Arc<Vec<Token>>,
}

impl TokenRegistry {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens: Arc::new(tokens),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Look up a token by its secret. Linear scan — typical N ≤ 10.
    pub fn lookup(&self, secret: &str) -> Option<&Token> {
        self.tokens.iter().find(|t| t.secret == secret)
    }
}

/// Authoritative op → required-scope mapping. Ops not listed return
/// `"meta.admin"` so any new op is denied to scoped tokens until
/// explicitly mapped. When adding an op, add a line here at the same
/// time.
pub fn op_scope(op: &str) -> &'static str {
    match op {
        // ---- cache.* ----
        "cache_get" | "cache_list" | "cache_stats" => "cache.read",
        "cache_put" | "cache_del" => "cache.write",

        // ---- job.* ----
        "job_list" | "job_output" | "job_status" | "job_wait" => "job.read",
        "job_submit" => "job.write",
        "job_cancel" | "job_kill" => "job.control",

        // ---- lock.* ----
        "lock_list" => "lock.read",
        "lock_acquire" | "lock_release" | "lock_try_acquire" => "lock.write",

        // ---- defs.* (federated catalog) ----
        "definitions_query"
        | "definitions_kinds"
        | "definitions_diff"
        | "definitions_subscribe"
        | "definitions_unsubscribe" => "defs.read",
        "definitions_emit" => "defs.write",

        // ---- snapshot.* ----
        "snapshot_list" | "snapshot_diff" => "snapshot.read",
        "snapshot_save" | "snapshot_load" => "snapshot.write",

        // ---- artifact.* ----
        "artifact_get" | "artifact_get_by_digest" | "artifact_list" => "artifact.read",
        "artifact_put" => "artifact.write",
        "artifact_gc" => "artifact.admin",

        // ---- schedule.* ----
        "schedule_list" => "schedule.read",
        "schedule_add" | "schedule_add_once" | "schedule_remove" => "schedule.write",

        // ---- event.* ----
        "subscribe" | "unsubscribe" | "subscription_set_paused" => "event.read",
        "publish" => "event.write",

        // ---- watch.* ----
        "watch_list" | "watcher_stats" => "watch.read",
        "watch_subscribe" | "watch_unsubscribe" | "fpath_changed" => "watch.write",

        // ---- shell.* (cross-shell coordination) ----
        "list_shells" => "shell.read",
        "send" | "notify" | "tag" | "untag" | "register" => "shell.write",

        // ---- recorder.* ----
        "recorder_ingest" => "recorder.write",

        // ---- history.* ----
        "history_query" => "history.read",
        "history_append" => "history.write",

        // ---- ask.* ----
        "ask_pending" | "ask_take" => "ask.read",
        "ask_ask" | "ask_response" | "ask_dismiss" => "ask.write",

        // ---- export.* ----
        "export" | "export_all" | "export_catalog" | "export_shard" | "export_zcompdump"
        | "view" => "export.read",

        // ---- import.* ----
        "import_all"
        | "import_catalog"
        | "import_history"
        | "import_shard"
        | "import_zcompdump"
        | "import_zwc"
        | "load_script"
        | "push_canonical"
        | "pull_canonical"
        | "canonical_hydrate_view"
        | "diff_canonical" => "import.write",

        // ---- meta.read (introspection / status) ----
        "info" | "ping" | "daemon" | "doctor" | "verify" | "metrics" | "log_stats"
        | "config_get" | "config_list" | "keys" | "complete" | "suggest" | "highlight"
        | "source_resolve" | "replay_log" | "cmd_result" | "cmd_started"
        | "subscribe_shard" => "meta.read",

        // ---- meta.admin (mutating admin ops) ----
        "clean" | "compact" | "stats_flush" | "log_level" | "log_rotate" | "config_set" => {
            "meta.admin"
        }

        // Unknown / future op — deny by default for scoped tokens.
        // Unscoped tokens still pass through Token::allows because the
        // empty-patterns shortcut grants full access regardless.
        _ => "meta.admin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_scopes_grant_full_access() {
        let t = Token {
            label: "legacy".into(),
            secret: "x".into(),
            scopes: ScopeMatcher::default(),
        };
        assert!(t.allows("cache.read"));
        assert!(t.allows("snapshot.write"));
        assert!(t.allows("meta.admin"));
        assert!(t.allows("brand.new.scope.never.seen"));
    }

    #[test]
    fn exact_pattern_matches_only_that_scope() {
        let m = ScopeMatcher::from_strings(["cache.put"]);
        assert!(m.matches("cache.put"));
        assert!(!m.matches("cache.get"));
        assert!(!m.matches("snapshot.save"));
    }

    #[test]
    fn area_wildcard_matches_every_verb_in_area() {
        let m = ScopeMatcher::from_strings(["cache.*"]);
        assert!(m.matches("cache.read"));
        assert!(m.matches("cache.write"));
        assert!(m.matches("cache.admin"));
        assert!(!m.matches("snapshot.write"));
        // `cache.*` should NOT match `cache` (no verb).
        assert!(!m.matches("cache"));
    }

    #[test]
    fn verb_wildcard_matches_every_area_with_that_verb() {
        let m = ScopeMatcher::from_strings(["*.read"]);
        assert!(m.matches("cache.read"));
        assert!(m.matches("snapshot.read"));
        assert!(m.matches("defs.read"));
        assert!(!m.matches("cache.write"));
        assert!(!m.matches("snapshot.save"));
    }

    #[test]
    fn star_matches_everything() {
        let m = ScopeMatcher::from_strings(["*"]);
        assert!(m.matches("cache.read"));
        assert!(m.matches("snapshot.write"));
        assert!(m.matches("meta.admin"));
    }

    #[test]
    fn multiple_patterns_or_together() {
        let m = ScopeMatcher::from_strings(["cache.*", "snapshot.read"]);
        assert!(m.matches("cache.write"));
        assert!(m.matches("snapshot.read"));
        assert!(!m.matches("snapshot.write"));
        assert!(!m.matches("job.submit"));
    }

    #[test]
    fn op_scope_table_covers_known_ops() {
        // Sample a few from each area; the full table is enforced by
        // the dispatcher at runtime.
        assert_eq!(op_scope("cache_put"), "cache.write");
        assert_eq!(op_scope("cache_get"), "cache.read");
        assert_eq!(op_scope("definitions_emit"), "defs.write");
        assert_eq!(op_scope("definitions_query"), "defs.read");
        assert_eq!(op_scope("snapshot_save"), "snapshot.write");
        assert_eq!(op_scope("watch_subscribe"), "watch.write");
        assert_eq!(op_scope("recorder_ingest"), "recorder.write");
        assert_eq!(op_scope("info"), "meta.read");
        assert_eq!(op_scope("clean"), "meta.admin");
        // Unmapped op falls through to the deny-by-default scope.
        assert_eq!(op_scope("zzz_brand_new_op"), "meta.admin");
    }

    #[test]
    fn registry_lookup_finds_by_secret() {
        let reg = TokenRegistry::new(vec![
            Token {
                label: "ci".into(),
                secret: "abc".into(),
                scopes: ScopeMatcher::from_strings(["job.*"]),
            },
            Token {
                label: "lsp".into(),
                secret: "def".into(),
                scopes: ScopeMatcher::from_strings(["defs.read"]),
            },
        ]);
        assert!(reg.lookup("abc").is_some());
        assert_eq!(reg.lookup("abc").unwrap().label, "ci");
        assert!(reg.lookup("def").is_some());
        assert!(reg.lookup("missing").is_none());
    }
}
