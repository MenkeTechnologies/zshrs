// AUTO-GENERATED. DO NOT EDIT BY HAND.
//
// Source: `daemon/builtins.rs` + `src/extensions/ext_builtins.rs`
// — extracted `///` doc-comments above each extension /
// daemon `z*` builtin function.
// Regen: ./scripts/gen_ext_builtin_docs.py
// Generated: 2026-05-23

/// Full doc-comment bodies for zshrs-only builtins. Wins over
/// the hand `EXT_BUILTIN_DOCS` one-liners in `lsp.rs` when a
/// name appears here — names without a `///` block in source
/// fall through to the hand fallback.
pub const EXT_BUILTIN_FULL_DOCS: &[(&str, &str)] = &[
    ("add_zsh_hook", "`add-zsh-hook` builtin. Direct port of the shell function at\n`Src/Functions/Misc/add-zsh-hook`, which maintains a paramtab\narray per well-known hook (`chpwd_functions`, `precmd_functions`,\n`preexec_functions`, `periodic_functions`, `zshexit_functions`,\n`zshaddhistory_functions`). This is the SHELL-LEVEL mechanism;\nit is distinct from the C-module hookdef chain in\n`src/ported/module.rs` (`addhookfunc(name, Hookfn)` writes to\n`hooktab` and is consumed by `runhookdef` for C-module\ncallbacks like BEFORECOMPLETEHOOK / AFTERCOMPLETEHOOK)."),
    ("arch", "arch — print machine architecture name. Coreutils arch(1)\n(a synonym for `uname -m` on most systems). Useful in shell\nscripts that need a quick `[[ $(arch) == arm64 ]]` check."),
    ("async", "async { cmd } — run command on worker pool, return job ID immediately.\nOutput captured in background, retrieve with `await $id`.\n\nUsage:\n  id=$(async 'sleep 2; echo done')\n  ... do other work ...\n  result=$(await $id)"),
    ("await", "await $id — block until async job completes, print its stdout, return its status.\n\nUsage:\n  id=$(async 'expensive_command')\n  await $id    # blocks until done, prints output\n  echo $?      # exit status of the async command"),
    ("barrier", "barrier cmd1 ::: cmd2 ::: cmd3 — run commands in parallel, wait for ALL to complete.\nReturns the worst (highest) exit status.\n\nUsage:\n  barrier 'make -C proj1' ::: 'make -C proj2' ::: 'make -C proj3'\n  barrier 'npm test' ::: 'cargo test' ::: 'pytest'"),
    ("base64", "base64 [-d] [FILE] — encode/decode base64. coreutils\nbase64(1) without --wrap (defaults to 76-char wrap on\nencode; 0 disables)."),
    ("caller", "caller - display call stack (bash)\ncaller [N] — bash builtin returning the location of the\ncurrent frame N. With no arg or N=0: 'LINE FUNC' (or just\n'LINE main' at top level). With N>0: 'LINE FUNC FILE' for\nframe N. Direct port of bash's bin_caller in builtins.def.\nReads from the existing $funcstack array we now maintain\n(vm_helper:7828-7835)."),
    ("cdreplay", "cdreplay - replay deferred compdef calls (zinit turbo mode)\nUsage: cdreplay [-q]"),
    ("cksum", "cksum [FILE...] — POSIX CRC-32 + byte-count + filename.\nOutput: `<crc> <bytes> <name>`. With no files or `-` reads\nstdin (filename column omitted in that case, per coreutils).\nPolynomial: 0x04C11DB7, init 0, length appended (POSIX)."),
    ("comm", "comm [-123] FILE1 FILE2 — line-by-line comparison of two\nsorted files. Coreutils comm(1) / POSIX. Three columns:\n(1) unique to FILE1, (2) unique to FILE2, (3) common.\nFlags `-1`/`-2`/`-3` suppress the respective column. Either\nfile may be `-` for stdin. Files MUST be sorted in the same\ncollation; comm performs a streaming merge-compare and is\nundefined-behavior on unsorted input (matches coreutils)."),
    ("compdef", "compdef - register completion functions for commands\nUsage: compdef _git git\n       compdef _docker docker docker-compose\n       compdef -d git  # delete"),
    ("compgen", "Generate completion candidates"),
    ("complete", "Define completion spec for a command"),
    ("compopt", "Modify completion options"),
    ("dbview", "dbview — browse zshrs SQLite cache tables without SQL.\n\nUsage:\n  dbview                      — list all tables and row counts\n  dbview autoloads             — dump autoloads table (name, source, body len, ast len)\n  dbview autoloads _git        — show single row by name\n  dbview comps                 — dump comps table\n  dbview history               — recent history entries\n  dbview history <pattern>     — search history\n  dbview plugins               — plugin cache entries\n  dbview executables            — PATH executables cache\n  dbview <table> --count       — just the count"),
    ("dircolors", "dircolors [-bcp] [FILE] — emit shell commands to set\nLS_COLORS. Coreutils dircolors(1). Without args, emits the\ndefault ls color database. -b for Bourne (export VAR=val),\n-c for csh (setenv VAR val), -p prints the database.\nWe hard-code coreutils' compiled-in default since shipping\nthe full /etc/DIR_COLORS database file isn't feasible here."),
    ("doctor", "doctor - diagnostic report of shell health, caches, and performance"),
    ("env", "env [-i] [NAME=VALUE]... [COMMAND [ARG]...] — print env or\nrun COMMAND with modified environment. Direct port of\ncoreutils env(1) for the most-used invocations."),
    ("expand", "expand [-t TAB] [FILE...] — convert tabs to spaces.\ncoreutils expand(1)."),
    ("expr", "expr ARG... — evaluate expression. POSIX expr(1).\nSubset port: integer arithmetic (+ - * / %), string match\n': REGEX', length STRING, substr STRING POS LEN, index\nSTRING CHARS. Recognizes the closing arg list directly\n(single-pass shunting-yard would be heavier than needed\nfor the common scripts)."),
    ("factor", "factor N... — print prime factorization of each integer arg.\nCoreutils factor(1). Format: `N: p1 p2 p3 ...`. Reads stdin\nif no args. Negative numbers and zero are rejected."),
    ("fold", "fold [-w WIDTH] [-s] [-b] [FILE...] — wrap input lines.\ncoreutils fold(1)."),
    ("groups", "groups [USER...] — print group memberships. Coreutils\ngroups(1) / POSIX. With no args, prints groups for the\neffective user; with args, prints \"USER : group1 group2 ...\"\nper user."),
    ("help", "help - display help for builtins (bash)"),
    ("intercept", "intercept builtin — register AOP advice on commands.\n\nUsage:\n  intercept before <pattern> { code }\n  intercept after <pattern> { code }\n  intercept around <pattern> { code }\n  intercept list                       — show all intercepts\n  intercept remove <id>                — remove by ID\n  intercept clear                      — remove all"),
    ("intercept_proceed", "intercept_proceed — called from around advice to execute the original command."),
    ("link", "link FILE1 FILE2 — call link(2) directly to create a hard\nlink from FILE1 to FILE2. POSIX link(1) / coreutils link(1).\nUnlike `ln`, link takes EXACTLY two args and rejects flags\n(POSIX requirement)."),
    ("logname", "logname — print login name. Coreutils logname(1) / POSIX.\nCalls getlogin(3) which reads from utmp. Falls back to\n\\$LOGNAME if getlogin fails."),
    ("mkfifo", "mkfifo [-m MODE] FILE... — create named pipes (FIFOs).\nCoreutils mkfifo(1) / POSIX. -m sets the mode (default 0666\nminus umask). Each FIFO is created independently; failures\nare reported per-file and the others continue."),
    ("nice", "nice [-n N] [-N N] [COMMAND...] — adjust niceness of the\nshell process (when no COMMAND given) or report current\nniceness. Coreutils nice(1) / POSIX nice(1). Without args,\nprint the current niceness (like \\`nice\\` with no args). With\njust -n N, set the niceness for the SHELL ITSELF — not for\na subsequently-exec'd command. Setting niceness for a child\ncommand requires fork-exec which the in-process model\ncan't do without breaking subsequent commands; for that case\ncallers should use /usr/bin/nice via PATH (zshrs's command\ndispatch will fall through to the external)."),
    ("nl", "nl [-b STYLE] [FILE...] — number lines. Direct port of\ncoreutils nl(1) for the most-used flag set."),
    ("nproc", "nproc — print number of online CPUs. coreutils nproc(1).\n--all uses CPU count from sysconf(_SC_NPROCESSORS_CONF);\ndefault uses _SC_NPROCESSORS_ONLN (the schedulable subset)."),
    ("paste", "paste [-d LIST] [FILE...] — merge lines of files. coreutils\npaste(1). Default delim is TAB; -d cycles through the\nsupplied delimiter chars."),
    ("peach", "peach 'cmd {}' arg1 arg2 ... — parallel for-each, no output ordering.\nLike pmap but doesn't collect output — fire-and-forget, print as completed.\n\nUsage:\n  peach 'convert {} {}.png' *.svg\n  peach 'rsync -a {} remote:{}' dir1 dir2 dir3"),
    ("pgrep", "pgrep 'pattern' arg1 arg2 ... — parallel grep/filter across worker pool.\nRuns the pattern command for each argument, prints args where command succeeds.\n\nUsage:\n  pgrep 'test -f {}' /path/a /path/b /path/c\n  pgrep 'grep -q TODO {}' *.rs"),
    ("pmap", "pmap 'cmd {}' arg1 arg2 arg3 — parallel map across worker pool.\nRuns `cmd` for each argument, replacing `{}` with the argument.\nOutput is collected in order. Returns 0 if all succeed.\n\nUsage:\n  pmap 'gzip {}' *.log\n  pmap 'echo {}' a b c d\n  ls *.rs | pmap 'wc -l {}'"),
    ("printenv", "printenv [VAR...] — print env. coreutils printenv(1).\nNo args: print all env vars (sorted by key for stable output).\nArgs: print VAR's value per arg, exit 1 if any unset."),
    ("profile", "profile — in-process command profiling with nanosecond accuracy.\n\nUnlike `time` (which measures one command) or `zprof` (which only\nprofiles function calls), `profile` traces every execute_command,\nexpansion, glob, and builtin dispatch inside the block.\n\nUsage:\n  profile { commands }     — profile a block\n  profile -s 'script'     — profile a script string\n  profile -f func         — profile a function call\n  profile --clear         — clear accumulated profile data\n  profile --dump          — show accumulated profile data"),
    ("provenance", "provenance — value lineage over bytecode execution.\n\nUsage:\n  provenance                  — list every tracked parameter and function\n  provenance NAME             — print NAME's lineage\n  provenance -m NAME...       — start tracking NAME\n  provenance -u NAME...       — stop tracking NAME, drop its lineage\n  provenance -j NAME          — print NAME's lineage as JSON\n  provenance -f ...           — act on shell functions instead of parameters\n  provenance -a               — track everything from here on\n  provenance -ua              — stop tracking everything (keeps what is recorded)\n  provenance -c               — clear every lineage and disarm\n\nFlags bundle: `-mf NAME` arms a function, `-jf NAME` prints one\nas JSON.\n\nThe engine records nothing until the first `-m` or `-a` (or\n`[provenance] track_all` at startup), and refuses to arm at all\nwhen `[provenance] enabled = false` in `~/.zshrs/zshrs.toml` or\n`ZSHRS_PROVENANCE=0` is set."),
    ("sha256sum", "sha256sum [FILE...] — write SHA-256 of each file (or stdin\nwhen no FILE / '-'). coreutils-style 'HEX  PATH' output."),
    ("shuf", "shuf [-n N] [-i LO-HI] [-e [STR...]] [FILE] — random\npermutation. coreutils shuf(1)."),
    ("sum", "sum [-rs] [FILE...] — BSD or SysV checksum.\n`-r`: BSD 16-bit rotating checksum (default per POSIX).\n`-s`: SysV checksum (sum-of-bytes mod 65535, then folded).\nOutput: `<sum> <512-byte-blocks> [name]` (BSD) or\n        `<sum> <kbytes> [name]` (SysV)."),
    ("tac", "tac [FILE...] — concatenate files, reverse line order.\ncoreutils tac(1)."),
    ("tput", "tput — terminfo capability query (minimal subset).\nCommon subset of ncurses tput(1):\n  tput cols / lines      → terminal width / height\n  tput colors            → terminal color count\n  tput clear / cl        → clear screen\n  tput cup R C           → cursor to (row, col) (0-based)\n  tput sgr0 / op         → reset attributes / colors\n  tput bold / smso / rmso / smul / rmul / rev / blink\n                         → text attributes\n  tput setaf N / setab N → fg/bg color (8/16 colors)\nMany other terminfo capabilities aren't yet wired; unknown\ncapabilities fall through to echotc's two-letter mapping or\nsilently exit 1 (tput's standard error code)."),
    ("tsort", "tsort [FILE] — topological sort. Coreutils tsort(1) / POSIX.\nInput is whitespace-separated pairs `A B` meaning \"A precedes\nB\"; tsort prints a partial order (Kahn's algorithm). Cycles\nare reported on stderr (one cycle node per line) and the\nprogram continues with that node treated as a leaf. Reads\nstdin when no file is given or `-`."),
    ("tty", "tty — print the controlling-terminal device path. coreutils\ntty(1). -s suppresses output (just sets the exit code)."),
    ("unexpand", "unexpand [-a] [-t TAB] [FILE...] — convert spaces to tabs.\ncoreutils unexpand(1).  Default tabstop 8; -a converts every\nrun of spaces (not just leading); without -a only leading\nruns collapse."),
    ("unlink", "unlink FILE — call unlink(2) directly to remove a single\nfile. POSIX unlink(1) / coreutils unlink(1). Strict: takes\nexactly ONE arg, no flags, no recursion. Cannot remove\ndirectories (errors if FILE is a directory)."),
    ("users", "users — print logged-in usernames. Coreutils users(1) /\nPOSIX. Fallback minimal impl: prints \\$USER (or current\neffective user via getpwuid) since fully reading utmp is\nplatform-specific. Multi-user output not yet implemented;\nshell scripts that just check `[[ $(users) ]]` still work."),
    ("yes", "yes [STRING] — print STRING (or 'y') forever. coreutils\nyes(1). Honors SIGPIPE: when stdout is piped and the consumer\ncloses, the write fails and we exit 0 silently."),
    ("zbuild", "zbuild --in PATHS... --out OUT — bake one or more shell\nscripts into a copy of the running zshrs binary in input\norder, producing a self-contained AOT executable.\n\n  zbuild --in *.zsh --out app           # glob expansion\n  zbuild --in lib1.zsh lib2.zsh main.zsh --out app\n  ./app                                  # runs all three\n                                         # sequentially under\n                                         # one ShellExecutor\n\nzsh has no project concept, so there's no manifest, no entry\npoint convention, no library directory walker. Order is\nexactly the order of paths given to `--in` (which honors\nshell glob expansion — \\`*.zsh\\` expands sorted by default).\n\n`--in` accepts ONE OR MORE paths until the next flag-style\ntoken (anything starting with `-`). This makes glob-driven\ninvocations like `--in *.zsh` work naturally — every path\nthe glob expanded to lands as another input file.\n\nFlags:\n  --in PATHS...  / -i PATHS...   script sources (1+, required)\n  --out PATH     / -o PATH       output binary (required)\n  --help / -h                    print usage"),
    ("zd", "`zd` — in-process builtin form of the standalone `zd` binary. Same\narg surface (see `daemon::zd_dispatch::USAGE`); transport is the\nlocal Unix socket via the existing `Client`. **No fork**: zshrs\ncalls dispatch() directly and gets the result back. ~5-15× faster\nthan fork+exec'ing the binary for short ops.\n\n`--url` / `--token` global flags are accepted (and ignored) for\ncommand-line shape compatibility with the binary form. The builtin\nalways uses the local socket — there's no point talking to the\ndaemon over HTTP from inside the same machine when a Unix socket\nis right there.\n\nSSE ops (`zd events`, `zd watch`) return an error pointing at the\nbinary form — long-lived stream pumps don't fit the request-\nresponse shape of a builtin."),
    ("zwhere", "`zwhere` — query the daemon's federated definitions catalog from\ninside zshrs. Thin client over the `definitions_query` op (same\nthing `zd defs query` calls). Always available in default zshrs;\nnot feature-gated on `recorder` because this is a read-only IPC\npath, not the AOP write path.\n\nUsage:\n  zwhere KIND NAME              # rows for `KIND NAME` across all shells\n  zwhere KIND --prefix STR      # all KIND rows whose name starts with STR\n  zwhere --kinds                # list every populated kind\n  zwhere --shell-id ID …        # restrict to one shell_id\n  zwhere --limit N …            # cap (default 1000 for tty output)\n\nOutput format: one row per line, `kind\\tname\\tvalue\\tshell_id\\tfile:line`.\nWide-column layout via padding is left to the caller piping through\n`column -t` if they want it; `zwhere` stays narrow so scripts can\n`awk` cleanly."),
];

/// Lookup helper used by `lsp::lookup_doc`.
pub fn lookup_full(name: &str) -> Option<&'static str> {
    EXT_BUILTIN_FULL_DOCS
        .iter()
        .find(|(k, _)| *k == name)
        .map(|&(_, body)| body)
}

#[cfg(test)]
mod tests {
    use super::*;

    // === lookup_full: exact-name resolution ===

    #[test]
    fn lookup_returns_body_for_known_ext_builtins() {
        // Pick representative builtins from each rough family the
        // generator scrapes: async/parallelism, daemon ipc, AOT,
        // coreutils-shim, completion glue.
        for name in [
            "async",
            "await",
            "barrier",
            "compdef",
            "complete",
            "compgen",
            "compopt",
            "doctor",
            "dbview",
            "base64",
            "cksum",
            "arch",
            "caller",
            "comm",
            "dircolors",
            "cdreplay",
        ] {
            let body = lookup_full(name)
                .unwrap_or_else(|| panic!("{name} must resolve in EXT_BUILTIN_FULL_DOCS"));
            assert!(!body.is_empty(), "empty doc body for {name}");
        }
    }

    #[test]
    fn lookup_is_case_sensitive() {
        // Lookup uses `*k == name` (string-equal). Case mutations must
        // NOT resolve.
        assert!(lookup_full("ASYNC").is_none());
        assert!(lookup_full("Async").is_none());
        assert!(lookup_full("BASE64").is_none());
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup_full("not_a_builtin").is_none());
        assert!(lookup_full("git").is_none()); // external, not ext-builtin
        assert!(lookup_full("ls").is_none()); // shell-side; not exposed via EXT_BUILTIN_FULL_DOCS
    }

    #[test]
    fn lookup_empty_string_returns_none() {
        assert!(lookup_full("").is_none());
    }

    #[test]
    fn lookup_async_body_mentions_worker_pool() {
        // Spot-check body content for the async builtin — the doc
        // body should describe the worker-pool dispatch contract so
        // hover surfaces show meaningful info.
        let body = lookup_full("async").expect("async must resolve");
        assert!(
            body.contains("worker pool") || body.contains("await"),
            "async doc body should mention worker pool / await pairing"
        );
    }

    #[test]
    fn lookup_barrier_body_mentions_parallel() {
        // barrier docs must mention parallelism — the whole point is
        // running N commands concurrently and joining.
        let body = lookup_full("barrier").expect("barrier must resolve");
        assert!(
            body.to_lowercase().contains("parallel"),
            "barrier doc body should mention parallel execution"
        );
    }

    // === EXT_BUILTIN_FULL_DOCS table integrity ===

    #[test]
    fn ext_builtin_full_docs_nonempty() {
        assert!(!EXT_BUILTIN_FULL_DOCS.is_empty());
        // The generator scrapes /// blocks from daemon + ext_builtins —
        // ≥30 is a sane floor given the existing surface.
        assert!(
            EXT_BUILTIN_FULL_DOCS.len() >= 30,
            "expected ≥30 entries, got {}",
            EXT_BUILTIN_FULL_DOCS.len()
        );
    }

    #[test]
    fn ext_builtin_full_docs_no_empty_bodies() {
        for (name, body) in EXT_BUILTIN_FULL_DOCS {
            assert!(!body.is_empty(), "empty doc body for {name:?}");
        }
    }

    #[test]
    fn ext_builtin_full_docs_no_empty_names() {
        for (name, _) in EXT_BUILTIN_FULL_DOCS {
            assert!(
                !name.is_empty(),
                "empty builtin name in EXT_BUILTIN_FULL_DOCS"
            );
        }
    }

    #[test]
    fn ext_builtin_full_docs_no_duplicate_names() {
        // The lookup returns the FIRST hit. If two entries have the
        // same key, one is unreachable. Catch the generator emitting
        // duplicates before they silently hide entries.
        let mut seen = std::collections::HashSet::new();
        for (name, _) in EXT_BUILTIN_FULL_DOCS {
            assert!(
                seen.insert(*name),
                "duplicate entry for builtin {name:?} (second occurrence shadowed by first)"
            );
        }
    }

    #[test]
    fn ext_builtin_full_docs_names_have_reasonable_shape() {
        // Names should look like identifiers (or known-special tokens
        // like `add_zsh_hook` / `compgen`). Catch the generator
        // accidentally pulling in stray whitespace or punctuation.
        for (name, _) in EXT_BUILTIN_FULL_DOCS {
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "unexpected character in builtin name {name:?}"
            );
        }
    }

    #[test]
    fn lookup_round_trips_for_every_entry() {
        // For every entry in the table, lookup_full must find the same
        // body string. Guards against accidental double-lookup or
        // table-mismatch bugs in the helper.
        for (name, body) in EXT_BUILTIN_FULL_DOCS {
            let got = lookup_full(name).expect("table entry must round-trip");
            assert_eq!(
                got, *body,
                "lookup_full returned a different body than the table entry for {name:?}"
            );
        }
    }
}
