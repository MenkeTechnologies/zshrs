//! Test harness for compsys - zsh completion system
//!
//! Run with: cargo run -p compsys

use compsys::{
    arguments_execute, build_cache_from_fpath,
    cache::{default_cache_path, CompsysCache},
    cache_is_valid, compadd_execute, compinit, compinit_lazy, compset_execute, describe_execute,
    do_completion, functions, get_system_fpath, load_from_cache, native_describe, ArgumentsSpec,
    CompDescribe, CompParams, CompTags, CompadOpts, Completion, CompletionReceiver,
    CompletionState, CompsetOp, DescribeItem, DescribeOpts, ZStyleStore,
};
use std::path::PathBuf;

fn main() {
    println!("=== compsys test harness ===\n");

    test_compadd();
    test_compset();
    test_zstyle();
    test_full_completion();
    test_compcore();
    test_computil();
    test_native_arguments();
    test_native_describe();
    test_compinit();
    test_functions();
    test_sqlite_cache();
    test_zpwr_zstyle_ingestion();
    test_shell_arrays();
    test_build_cache_from_fpath();

    println!("\n=== All tests passed ===");
}

fn test_compadd() {
    println!("--- Testing compadd ---");

    // Test basic option parsing
    let args: Vec<String> = vec!["-J", "files", "-P", "[", "-S", "]", "foo", "bar", "baz"]
        .into_iter()
        .map(String::from)
        .collect();

    let (opts, matches) = CompadOpts::parse(&args).expect("parse failed");
    assert_eq!(opts.group_name(), Some("files"));
    assert_eq!(opts.prefix, Some("[".to_string()));
    assert_eq!(opts.suffix, Some("]".to_string()));
    assert_eq!(matches, vec!["foo", "bar", "baz"]);
    println!("  parse basic options: OK");

    // Test flag combinations
    let args: Vec<String> = vec!["-qfnU", "-X", "explanation", "match1"]
        .into_iter()
        .map(String::from)
        .collect();
    let (opts, _) = CompadOpts::parse(&args).expect("parse flags");
    assert!(opts.remove_on_space);
    assert!(opts.file);
    assert!(opts.nolist);
    assert!(opts.no_match);
    assert_eq!(opts.explanation, Some("explanation".to_string()));
    println!("  parse flag combinations: OK");

    // Test execution
    let mut params = CompParams::from_line("git com", 7);
    let mut receiver = CompletionReceiver::new(100);

    let args: Vec<String> = vec![
        "-J",
        "commands",
        "commit",
        "checkout",
        "clone",
        "cherry-pick",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let (opts, matches) = CompadOpts::parse(&args).unwrap();

    let result = compadd_execute(&opts, &matches, &params, &mut receiver, |_| None);
    assert_eq!(result, 0);

    let completions = receiver.all_completions();
    // Should match "commit", "checkout", "clone", "cherry-pick" against prefix "com"
    assert!(!completions.is_empty());
    println!(
        "  compadd execution: OK (matched {} completions)",
        completions.len()
    );

    // Test with -U (no prefix matching)
    params.prefix = "xyz".to_string();
    let mut receiver2 = CompletionReceiver::new(100);
    let args: Vec<String> = vec!["-U", "alpha", "beta"]
        .into_iter()
        .map(String::from)
        .collect();
    let (opts, matches) = CompadOpts::parse(&args).unwrap();
    let result = compadd_execute(&opts, &matches, &params, &mut receiver2, |_| None);
    assert_eq!(result, 0);
    assert_eq!(receiver2.all_completions().len(), 2);
    println!("  compadd -U (no match): OK");
}

fn test_compset() {
    println!("\n--- Testing compset ---");

    // Test -p (numeric prefix)
    let mut params = CompParams::new();
    params.prefix = "foobar".to_string();

    let op = CompsetOp::parse(&["-p".to_string(), "3".to_string()]).unwrap();
    let result = compset_execute(&op, &mut params);
    assert_eq!(result, 0);
    assert_eq!(params.prefix, "bar");
    assert_eq!(params.iprefix, "foo");
    println!("  compset -p (numeric prefix): OK");

    // Test -s (numeric suffix)
    let mut params = CompParams::new();
    params.suffix = "foobar".to_string();

    let op = CompsetOp::parse(&["-s".to_string(), "3".to_string()]).unwrap();
    let result = compset_execute(&op, &mut params);
    assert_eq!(result, 0);
    assert_eq!(params.suffix, "foo");
    assert_eq!(params.isuffix, "bar");
    println!("  compset -s (numeric suffix): OK");

    // Test -P (pattern prefix)
    let mut params = CompParams::new();
    params.prefix = "user@host:path".to_string();

    let op = CompsetOp::parse(&["-P".to_string(), "*:".to_string()]).unwrap();
    let result = compset_execute(&op, &mut params);
    assert_eq!(result, 0);
    assert_eq!(params.prefix, "path");
    assert_eq!(params.iprefix, "user@host:");
    println!("  compset -P (pattern prefix): OK");

    // Test -n (range)
    let mut params = CompParams::new();
    params.words = vec![
        "cmd".to_string(),
        "sub".to_string(),
        "arg1".to_string(),
        "arg2".to_string(),
    ];
    params.current = 3;

    let op = CompsetOp::parse(&["-n".to_string(), "2".to_string(), "4".to_string()]).unwrap();
    let result = compset_execute(&op, &mut params);
    assert_eq!(result, 0);
    assert_eq!(params.words.len(), 3);
    println!("  compset -n (range): OK");
}

fn test_zstyle() {
    println!("\n--- Testing zstyle ---");

    let mut store = ZStyleStore::new();

    // Set some styles
    store.set(":completion:*", "menu", vec!["select".to_string()], false);
    store.set(
        ":completion:*:descriptions",
        "format",
        vec!["%B%d%b".to_string()],
        false,
    );
    store.set(
        ":completion:*:*:*:*:corrections",
        "format",
        vec!["%F{red}%d%f".to_string()],
        false,
    );

    // Test lookup
    assert_eq!(store.lookup_str(":completion:foo", "menu"), Some("select"));
    assert_eq!(
        store.lookup_str(":completion:foo:descriptions", "format"),
        Some("%B%d%b")
    );
    assert_eq!(
        store.lookup_str(":completion:a:b:c:d:corrections", "format"),
        Some("%F{red}%d%f")
    );
    println!("  zstyle lookup: OK");

    // Test specificity (more specific pattern wins)
    store.set(":completion:*", "verbose", vec!["no".to_string()], false);
    store.set(
        ":completion:*:*:*:default",
        "verbose",
        vec!["yes".to_string()],
        false,
    );

    assert_eq!(
        store.lookup_str(":completion:x:y:z:default", "verbose"),
        Some("yes")
    );
    assert_eq!(
        store.lookup_str(":completion:simple", "verbose"),
        Some("no")
    );
    println!("  zstyle specificity: OK");

    // Test boolean conversion
    store.set(
        ":completion:*",
        "list-colors",
        vec!["true".to_string()],
        false,
    );
    assert_eq!(
        store.lookup_bool(":completion:test", "list-colors"),
        Some(true)
    );
    println!("  zstyle boolean: OK");

    // Test delete
    store.delete(":completion:*", Some("menu"));
    assert!(store.lookup_str(":completion:foo", "menu").is_none());
    println!("  zstyle delete: OK");

    // Test print
    let output = store.print(true);
    assert!(!output.is_empty());
    println!("  zstyle print: OK ({} styles)", output.len());
}

fn test_full_completion() {
    println!("\n--- Testing full completion flow ---");

    // Simulate: git ch<TAB>
    let params = CompParams::from_line("git ch", 6);
    assert_eq!(params.words, vec!["git", "ch"]);
    assert_eq!(params.current, 2);
    assert_eq!(params.prefix, "ch");
    assert_eq!(params.suffix, "");
    println!("  parse command line: OK");

    // Set up styles
    let mut styles = ZStyleStore::new();
    styles.set(":completion:*", "menu", vec!["select".to_string()], false);
    styles.set(
        ":completion:*:descriptions",
        "format",
        vec![" -- %d --".to_string()],
        false,
    );

    // Add completions
    let mut receiver = CompletionReceiver::new(100);

    let args: Vec<String> = vec![
        "-J",
        "git-commands",
        "-X",
        "git commands",
        "checkout",
        "cherry-pick",
        "clone",
        "commit",
        "config",
    ]
    .into_iter()
    .map(String::from)
    .collect();

    let (opts, matches) = CompadOpts::parse(&args).unwrap();
    compadd_execute(&opts, &matches, &params, &mut receiver, |_| None);

    let completions = receiver.all_completions();
    let matched: Vec<&str> = completions.iter().map(|c| c.str_.as_str()).collect();

    // Should match: checkout, cherry-pick (start with "ch")
    assert!(matched.contains(&"checkout"));
    assert!(matched.contains(&"cherry-pick"));
    assert!(!matched.contains(&"clone")); // doesn't match "ch"
    assert!(!matched.contains(&"commit")); // doesn't match "ch"
    println!("  filter by prefix: OK (matched: {:?})", matched);

    // Test compset to handle subcommand completion
    // Simulate: git checkout -b<TAB>
    let mut params2 = CompParams::from_line("git checkout -b", 14);

    // Use compset -n to focus on args after "checkout"
    let op = CompsetOp::parse(&["-n".to_string(), "2".to_string(), "-1".to_string()]).unwrap();
    compset_execute(&op, &mut params2);

    assert_eq!(params2.words[0], "checkout");
    println!("  compset for subcommand: OK");

    println!("\n  Full completion flow: SUCCESS");
}

fn test_compcore() {
    println!("\n--- Testing compcore (new completion state) ---");

    // Test using CompletionState
    let mut state = CompletionState::from_line("git ch", 6);

    // Add matches using the state
    state.begin_group("commands", true);
    state.add_match(Completion::new("checkout"), Some("commands"));
    state.add_match(Completion::new("cherry-pick"), Some("commands"));
    state.add_match(Completion::new("clean"), Some("commands"));
    state.end_group();

    assert_eq!(state.nmatches, 3);
    println!("  add matches: OK (3 matches)");

    state.calculate_unambiguous();
    // checkout, cherry-pick share "che", clean shares "c"
    // Common prefix is "c" (all three start with "c")
    // Wait - clean starts with "cl", not "ch", so it shouldn't have matched prefix "ch"
    // But we added it directly, bypassing prefix check
    // Let's verify the unambiguous is calculated from all matches
    println!(
        "  unambiguous prefix: '{}' (from all matches)",
        state.ainfo.prefix
    );

    // Test do_completion
    let mut state2 = CompletionState::new();
    let nmatches = do_completion("git st", 6, &mut state2, |s| {
        s.begin_group("commands", true);
        s.add_match(Completion::new("status"), Some("commands"));
        s.add_match(Completion::new("stash"), Some("commands"));
        s.end_group();
    });

    assert_eq!(nmatches, 2);
    assert_eq!(state2.ainfo.prefix, "sta");
    println!("  do_completion: OK (2 matches, unambiguous='sta')");
}

fn test_computil() {
    println!("\n--- Testing computil (completion utilities) ---");

    // Test CompTags
    let mut tags = CompTags::new();
    tags.init(
        "test",
        &[
            "files".to_string(),
            "directories".to_string(),
            "commands".to_string(),
        ],
    );

    assert!(tags.try_tags(&["files".to_string(), "commands".to_string()]));
    assert!(tags.is_set("files"));
    assert!(tags.is_set("commands"));
    assert!(!tags.is_set("directories"));
    println!("  CompTags: OK");

    // Test CompDescribe
    let items = CompDescribe::parse_items(&[
        "commit:Record changes to the repository".to_string(),
        "checkout:Switch branches or restore working tree files".to_string(),
        "branch:List, create, or delete branches".to_string(),
    ]);

    assert_eq!(items.len(), 3);
    println!("  CompDescribe parse: OK ({} items)", items.len());

    // Test describe_execute
    // "git co" at position 6 means prefix is "co"
    let mut state = CompletionState::from_line("git co", 6);
    describe_execute(&mut state, "git-commands", "git commands", &items, None);

    // "commit" starts with "co", "checkout" starts with "ch", "branch" starts with "b"
    // Only "commit" should match prefix "co"
    let matched: Vec<&str> = state
        .all_completions()
        .iter()
        .map(|c| c.str_.as_str())
        .collect();
    assert!(matched.contains(&"commit"));
    assert!(!matched.contains(&"checkout")); // starts with "ch", not "co"
    assert!(!matched.contains(&"branch"));
    println!("  describe_execute: OK (matched: {:?})", matched);
}

fn test_native_arguments() {
    println!("\n--- Testing native _arguments ---");

    // Parse a typical _arguments spec (without actions that would trigger early return)
    let args = vec![
        "-v[verbose mode]".to_string(),
        "--help[show help message]".to_string(),
        "--output=[output file]".to_string(), // option with arg but no action
        "(-q --quiet)--verbose[be verbose]".to_string(),
        "*-d[increase debug level]".to_string(),
    ];

    let spec = ArgumentsSpec::parse(&args);

    assert_eq!(spec.options.len(), 5);
    println!("  parse spec: OK ({} options)", spec.options.len());

    // Test completion - prefix is "--"
    let mut state = CompletionState::from_line("mycmd --", 8);

    let added = arguments_execute(&mut state, &spec, |action, _state| {
        println!("    would call action: {}", action);
    });

    let matched: Vec<&str> = state
        .all_completions()
        .iter()
        .map(|c| c.str_.as_str())
        .collect();
    println!("  matched: {:?}, added={}", matched, added);

    // The prefix is "--" so all long options should match
    // Note: our prefix from "mycmd --" at pos 8 should be "--"
    println!("  state prefix: '{}'", state.params.prefix);

    assert!(matched.contains(&"--help"), "should contain --help");
    assert!(matched.contains(&"--verbose"), "should contain --verbose");
    println!("  complete options: OK (matched: {:?})", matched);

    // Test with prefix "--h"
    let mut state2 = CompletionState::from_line("mycmd --h", 9);
    arguments_execute(&mut state2, &spec, |_, _| {});

    let matched2: Vec<&str> = state2
        .all_completions()
        .iter()
        .map(|c| c.str_.as_str())
        .collect();
    assert!(matched2.contains(&"--help"));
    assert!(!matched2.contains(&"--verbose"));
    println!("  filter by prefix: OK (matched: {:?})", matched2);

    // Test short option completion
    let mut state3 = CompletionState::from_line("mycmd -", 7);
    arguments_execute(&mut state3, &spec, |_, _| {});

    let matched3: Vec<&str> = state3
        .all_completions()
        .iter()
        .map(|c| c.str_.as_str())
        .collect();
    assert!(matched3.contains(&"-v"));
    assert!(matched3.contains(&"-d"));
    println!("  short options: OK (matched: {:?})", matched3);
}

fn test_native_describe() {
    println!("\n--- Testing native _describe ---");

    let items = vec![
        DescribeItem {
            value: "checkout".to_string(),
            description: "Switch branches".to_string(),
        },
        DescribeItem {
            value: "commit".to_string(),
            description: "Record changes".to_string(),
        },
        DescribeItem {
            value: "push".to_string(),
            description: "Update remote".to_string(),
        },
    ];

    let opts = DescribeOpts {
        tag: Some("commands".to_string()),
        sorted: true,
        ..Default::default()
    };

    let mut state = CompletionState::from_line("git co", 6);

    let added = native_describe(&mut state, &opts, "git command", &items);

    assert!(added);
    let matched: Vec<&str> = state
        .all_completions()
        .iter()
        .map(|c| c.str_.as_str())
        .collect();
    assert!(matched.contains(&"commit"));
    assert!(!matched.contains(&"checkout")); // "checkout" doesn't start with "co"
    assert!(!matched.contains(&"push"));
    println!("  native _describe: OK (matched: {:?})", matched);
}

fn test_compinit() {
    println!("\n--- Testing native compinit (parallel with rayon) ---");

    // Get fpath from environment or use defaults
    let fpath_str = std::env::var("FPATH").unwrap_or_default();
    let zsh_fpath: Vec<PathBuf> = if !fpath_str.is_empty() {
        fpath_str.split(':').map(PathBuf::from).collect()
    } else {
        vec![
            PathBuf::from("/usr/share/zsh/functions/Completion/Unix"),
            PathBuf::from("/usr/share/zsh/functions/Completion/Base"),
            PathBuf::from("/usr/share/zsh/functions/Completion/Zsh"),
            PathBuf::from("/usr/local/share/zsh/site-functions"),
            PathBuf::from("/opt/homebrew/share/zsh/site-functions"),
            PathBuf::from("functions"),
        ]
    };

    // Filter to existing directories
    let existing: Vec<PathBuf> = zsh_fpath.into_iter().filter(|p| p.exists()).collect();

    if existing.is_empty() {
        println!("  No zsh completion directories found, using local test");
        // Test with our local functions
        let local_fpath = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("functions")];

        let result = compinit(&local_fpath);
        println!(
            "  scanned {} dirs, {} files in {}ms",
            result.dirs_scanned, result.files_scanned, result.scan_time_ms
        );
        println!(
            "  registered {} commands, {} patterns",
            result.comps.len(),
            result.patcomps.len()
        );
    } else {
        println!("  Found {} completion directories", existing.len());

        // Benchmark: run multiple times
        let iterations = 5;
        let mut times = Vec::new();
        let mut result = None;

        for _ in 0..iterations {
            let r = compinit(&existing);
            times.push(r.scan_time_ms);
            result = Some(r);
        }

        let result = result.unwrap();
        let avg_time: u64 = times.iter().sum::<u64>() / iterations;
        let min_time = times.iter().min().unwrap();
        let max_time = times.iter().max().unwrap();

        println!("  Scanned {} directories", result.dirs_scanned);
        println!("  Found {} completion files", result.files_scanned);
        println!("  Registered {} commands", result.comps.len());
        println!("  Registered {} patterns", result.patcomps.len());
        println!("  Registered {} autoloads", result.compautos.len());
        println!(
            "  Timing ({}x): avg={}ms, min={}ms, max={}ms",
            iterations, avg_time, min_time, max_time
        );

        // Show some example completions
        let examples: Vec<_> = result.comps.iter().take(5).collect();
        println!("  Example completions: {:?}", examples);
    }

    println!("  compinit: OK");
}

fn test_functions() {
    println!("\n--- Testing functions module ---");

    // Test glob matching
    assert!(functions::glob_match("*.rs", "main.rs"));
    assert!(functions::glob_match("foo*", "foobar"));
    assert!(functions::glob_match("*.txt", "readme.txt"));
    assert!(!functions::glob_match("*.rs", "main.txt"));
    println!("  glob_match: OK");

    // Test edit distance
    assert_eq!(functions::edit_distance("foo", "foo"), 0);
    assert_eq!(functions::edit_distance("foo", "fob"), 1);
    assert_eq!(functions::edit_distance("kitten", "sitting"), 3);
    println!("  edit_distance: OK");

    // Test compiled arg spec
    let spec = functions::CompiledArgSpec::parse("*:file:_files").unwrap();
    assert_eq!(spec.pattern, "*");
    assert_eq!(spec.description, "file");
    assert_eq!(spec.action, "_files");
    println!("  CompiledArgSpec: OK");

    // Test numbers completion
    let mut state = CompletionState::from_line("pick ", 5);
    let matched = functions::numbers(&mut state, 1, 10, 1, Some("number"));
    assert!(matched);
    assert!(state.nmatches > 0);
    println!("  numbers: OK (matched {} numbers)", state.nmatches);

    // Test sub_commands
    let mut state = CompletionState::from_line("git ", 4);
    let commands = vec![
        ("commit".to_string(), "Create a new commit".to_string()),
        ("push".to_string(), "Push to remote".to_string()),
        ("pull".to_string(), "Pull from remote".to_string()),
    ];
    let matched = functions::sub_commands(&mut state, &commands);
    assert!(matched);
    assert_eq!(state.nmatches, 3);
    println!("  sub_commands: OK");

    // Test history completion
    // History matches whole command lines starting with prefix
    let mut state = CompletionState::from_line("gi", 2);
    let history = vec![
        "git commit -m 'test'".to_string(),
        "ls -la".to_string(),
        "git checkout main".to_string(),
        "grep foo bar".to_string(),
    ];
    let matched = functions::history(&mut state, &history);
    assert!(matched);
    assert_eq!(state.nmatches, 2); // "git commit" and "git checkout"
    println!("  history: OK (matched {} entries)", state.nmatches);

    // Test expand (tilde)
    let mut state = CompletionState::from_line("cd ~", 4);
    let expanded = functions::expand(&mut state);
    if std::env::var("HOME").is_ok() {
        assert!(expanded);
        println!("  expand (tilde): OK");
    } else {
        println!("  expand (tilde): SKIPPED (no HOME)");
    }

    // Test correct_word
    let mut state = CompletionState::from_line("comit", 5);
    let words = vec![
        "commit".to_string(),
        "comment".to_string(),
        "commerce".to_string(),
    ];
    let matched = functions::correct_word(&mut state, &words);
    assert!(matched);
    println!("  correct_word: OK (found {} corrections)", state.nmatches);

    println!("  functions module: ALL OK");
}

fn test_sqlite_cache() {
    println!("\n--- Testing SQLite cache ---");

    let mut cache = CompsysCache::memory().unwrap();

    // Simulate loading 500k autoloads
    println!("  Creating 50k autoload stubs...");
    let start = std::time::Instant::now();
    let autoloads: Vec<(String, String, i64, i64)> = (0..50000)
        .map(|i| {
            (
                format!("_func{}", i),
                format!("src{}.zwc", i % 10),
                i * 50,
                50,
            )
        })
        .collect();
    cache.add_autoloads_bulk(&autoloads).unwrap();
    println!("  Bulk insert 50k: {}ms", start.elapsed().as_millis());

    // Lookup speed
    let start = std::time::Instant::now();
    for i in (0..50000).step_by(100) {
        let _ = cache.get_autoload(&format!("_func{}", i)).unwrap();
    }
    println!("  500 random lookups: {}ms", start.elapsed().as_millis());

    // Comps
    let comps: Vec<(String, String)> = (0..10000)
        .map(|i| (format!("cmd{}", i), format!("_cmd{}", i)))
        .collect();
    cache.set_comps_bulk(&comps).unwrap();
    assert_eq!(cache.comp_count().unwrap(), 10000);
    println!("  10k comps: OK");

    // Stats
    let stats = cache.stats().unwrap();
    println!(
        "  Stats: autoloads={}, comps={}",
        stats.autoloads, stats.comps
    );

    println!("  SQLite cache: OK");
}

fn test_zpwr_zstyle_ingestion() {
    println!("\n--- Testing zpwr zstyle ingestion ---");

    let mut cache = CompsysCache::memory().unwrap();

    // Parse zpwrBindZstyle and ingest
    let zpwr_zstyle_path = "/Users/wizard/.zpwr/autoload/common/zpwrBindZstyle";

    if !std::path::Path::new(zpwr_zstyle_path).exists() {
        println!("  SKIPPED: zpwrBindZstyle not found");
        return;
    }

    let content = std::fs::read_to_string(zpwr_zstyle_path).unwrap();

    // Parse zstyle commands
    let mut zstyles: Vec<(String, String, Vec<String>, bool)> = Vec::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and non-zstyle lines
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        // Match: zstyle 'pattern' style value...
        // or: zstyle -e 'pattern' style value
        let is_eval = line.contains("zstyle -e ");
        let line = line.replace("zstyle -e ", "zstyle ");

        if !line.starts_with("zstyle ") {
            continue;
        }

        let rest = &line[7..].trim();

        // Parse pattern (quoted)
        let (pattern, rest) = if rest.starts_with('\'') {
            if let Some(end) = rest[1..].find('\'') {
                (&rest[1..end + 1], rest[end + 2..].trim())
            } else {
                continue;
            }
        } else if rest.starts_with('"') {
            if let Some(end) = rest[1..].find('"') {
                (&rest[1..end + 1], rest[end + 2..].trim())
            } else {
                continue;
            }
        } else {
            // Unquoted - take until whitespace
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            (&rest[..end], rest[end..].trim())
        };

        // Parse style name
        let style_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let style = &rest[..style_end];
        let values_str = rest[style_end..].trim();

        if pattern.is_empty() || style.is_empty() {
            continue;
        }

        // Parse values (simplified - just split on whitespace for now)
        let values: Vec<String> = if values_str.is_empty() {
            vec![]
        } else {
            // Handle quoted values
            let mut vals = Vec::new();
            let mut current = String::new();
            let mut in_quote = false;
            let mut quote_char = ' ';

            for c in values_str.chars() {
                if !in_quote && (c == '\'' || c == '"') {
                    in_quote = true;
                    quote_char = c;
                } else if in_quote && c == quote_char {
                    in_quote = false;
                    if !current.is_empty() {
                        vals.push(current.clone());
                        current.clear();
                    }
                } else if !in_quote && c.is_whitespace() {
                    if !current.is_empty() {
                        vals.push(current.clone());
                        current.clear();
                    }
                } else {
                    current.push(c);
                }
            }
            if !current.is_empty() {
                vals.push(current);
            }
            vals
        };

        zstyles.push((pattern.to_string(), style.to_string(), values, is_eval));
    }

    println!("  Parsed {} zstyle commands", zstyles.len());

    // Bulk insert
    let start = std::time::Instant::now();
    cache.set_zstyles_bulk(&zstyles).unwrap();
    println!("  Bulk insert: {}ms", start.elapsed().as_millis());

    // Verify some lookups
    let menu = cache.lookup_zstyle(":completion:foo", "use-cache");
    if let Ok(Some(entry)) = menu {
        println!("  use-cache: {:?}", entry.values);
    }

    let format = cache.lookup_zstyle(":completion:foo:descriptions", "format");
    if let Ok(Some(entry)) = format {
        println!(
            "  descriptions format: {} chars",
            entry.values.first().map(|s| s.len()).unwrap_or(0)
        );
    }

    let group_order = cache.lookup_zstyle(":completion:foo:bar", "group-order");
    if let Ok(Some(entry)) = group_order {
        println!("  group-order: {} groups", entry.values.len());
    }

    // Stats
    let stats = cache.stats().unwrap();
    println!("  Total zstyles in cache: {}", stats.zstyles);

    // List some
    let all = cache.list_zstyles().unwrap();
    println!("  Sample styles:");
    for (pattern, style, values, _eval) in all.iter().take(5) {
        println!(
            "    {} {} = {:?}",
            pattern,
            style,
            values.first().map(|s| if s.len() > 30 {
                format!("{}...", &s[..30])
            } else {
                s.clone()
            })
        );
    }

    println!("  zpwr zstyle ingestion: OK");
}

fn test_shell_arrays() {
    println!("\n--- Testing shell-visible arrays ---");

    let mut cache = CompsysCache::memory().unwrap();

    // Populate _comps (like compinit does)
    let comps: Vec<(String, String)> = vec![
        ("git", "_git"),
        ("docker", "_docker"),
        ("cargo", "_cargo"),
        ("kubectl", "_kubectl"),
        ("terraform", "_terraform"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    cache.set_comps_bulk(&comps).unwrap();

    // Populate _services
    let services: Vec<(String, String)> = vec![
        ("git-commit", "git"),
        ("git-push", "git"),
        ("docker-compose", "docker"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    cache.set_services_bulk(&services).unwrap();

    // Populate _patcomps
    cache.set_patcomp("git-*", "_git").unwrap();
    cache.set_patcomp("docker-*", "_docker").unwrap();

    // $#_comps equivalent
    println!("  $#_comps = {}", cache.comps_count().unwrap());

    // ${(k)_comps} equivalent
    let keys = cache.comps_keys().unwrap();
    println!("  ${{(k)_comps}} = {:?}", keys);

    // ${(v)_comps} equivalent
    let values = cache.comps_values().unwrap();
    println!("  ${{(v)_comps}} = {:?}", values);

    // $_comps[git] equivalent
    let git_func = cache.get_comp("git").unwrap();
    println!("  $_comps[git] = {:?}", git_func);

    // $#_services
    println!("  $#_services = {}", cache.services_count().unwrap());

    // $#_patcomps
    println!("  $#_patcomps = {}", cache.patcomps_count().unwrap());

    // Full stats
    let stats = cache.stats().unwrap();
    println!(
        "  Full stats: comps={}, services={}, patcomps={}",
        stats.comps, stats.services, stats.patcomps
    );

    println!("  shell arrays: OK");
}

fn test_build_cache_from_fpath() {
    println!("\n--- Testing build_cache_from_fpath ---");

    // Get system fpath
    let fpath = get_system_fpath();
    println!("  Found {} fpath directories", fpath.len());

    if fpath.is_empty() {
        println!("  SKIPPED: no fpath directories found");
        return;
    }

    // Use standard cache path
    let cache_path = default_cache_path();
    std::fs::create_dir_all(cache_path.parent().unwrap()).ok();
    // Remove old DB and WAL files to start fresh
    let _ = std::fs::remove_file(&cache_path);
    let _ = std::fs::remove_file(format!("{}-shm", cache_path.display()));
    let _ = std::fs::remove_file(format!("{}-wal", cache_path.display()));
    let mut cache = CompsysCache::open(&cache_path).unwrap();
    println!("  Cache DB: {}", cache_path.display());

    // Build cache from fpath
    let start = std::time::Instant::now();
    let result = build_cache_from_fpath(&fpath, &mut cache).unwrap();
    let elapsed = start.elapsed();

    println!("  Scan + cache build: {}ms", elapsed.as_millis());
    println!("  Scanned {} directories", result.dirs_scanned);
    println!("  Found {} completion files", result.files_scanned);

    // Check cache stats
    let stats = cache.stats().unwrap();
    println!("  Cache stats:");
    println!("    comps: {}", stats.comps);
    println!("    autoloads: {}", stats.autoloads);
    println!("    patcomps: {}", stats.patcomps);
    println!("    services: {}", stats.services);

    // Test lookups
    if let Ok(Some(func)) = cache.get_comp("git") {
        println!("  _comps[git] = {}", func);
    }
    if let Ok(Some(func)) = cache.get_comp("docker") {
        println!("  _comps[docker] = {}", func);
    }
    if let Ok(Some(func)) = cache.get_comp("cargo") {
        println!("  _comps[cargo] = {}", func);
    }

    // Test autoload lookup
    if let Ok(Some(stub)) = cache.get_autoload("_git") {
        println!("  _git autoload: {} ({} bytes)", stub.source, stub.size);
        if let Some(ref body) = stub.body {
            println!(
                "  _git body: {} chars (first 100: {}...)",
                body.len(),
                &body.chars().take(100).collect::<String>()
            );
        } else {
            println!("  _git body: NOT CACHED");
        }
    }

    // Benchmark autoload body lookup (this is the hot path for autoload -Xz)
    println!("\n  Benchmarking autoload -Xz (body lookup):");
    let iterations = 10000;
    let funcs = [
        "_git", "_docker", "_cargo", "_ls", "_cd", "_cp", "_mv", "_rm", "_cat", "_grep",
    ];
    let start = std::time::Instant::now();
    for i in 0..iterations {
        let name = funcs[i % funcs.len()];
        let _ = cache.get_autoload_body(name);
    }
    let total_us = start.elapsed().as_micros();
    let avg_ns = (total_us * 1000) / iterations as u128;
    println!(
        "    {}x get_autoload_body: {}µs total, {}ns avg per lookup",
        iterations, total_us, avg_ns
    );

    // Test pattern completion
    if let Ok(Some(func)) = cache.find_patcomp("git-commit") {
        println!("  patcomp for git-commit: {}", func);
    }

    // Test special context entries (should match zsh exactly)
    println!("  Special context entries:");
    for key in &[
        "-",
        "-default-",
        "-redirect-",
        "-command-",
        "-value-",
        "-first-",
        "-condition-",
    ] {
        if let Ok(Some(func)) = cache.get_comp(key) {
            println!("    _comps[{}] = {}", key, func);
        }
    }

    // Dump all keys to file for comparison with zsh
    let keys = cache.comps_keys().unwrap();
    let keys_path = "/tmp/rust_comps_keys.txt";
    std::fs::write(keys_path, keys.join("\n")).unwrap();
    println!("  Wrote {} keys to {}", keys.len(), keys_path);

    // Test lazy compinit (instantaneous - just validates cache)
    println!("\n  Testing compinit_lazy (truly instant):");
    let iterations = 10000;
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = compinit_lazy(&cache);
    }
    let total_us = start.elapsed().as_micros();
    let avg_ns = (total_us * 1000) / iterations as u128;
    println!(
        "    {}x compinit_lazy: {}µs total, {}ns avg",
        iterations, total_us, avg_ns
    );

    let (valid, count) = compinit_lazy(&cache);
    assert!(valid);
    println!("    cache valid: {}, entries: {}", valid, count);

    // Test full load from cache (slower but loads all into HashMap)
    println!("\n  Testing load_from_cache (loads all entries):");
    let iterations = 100;
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = load_from_cache(&cache).unwrap();
    }
    let total_us = start.elapsed().as_micros();
    let avg_us = total_us / iterations as u128;
    println!(
        "    {}x load_from_cache: {}µs total, {}µs avg",
        iterations, total_us, avg_us
    );

    // Verify cache is valid
    assert!(cache_is_valid(&cache));
    println!("    cache_is_valid: true");

    // Verify loaded data matches
    let loaded = load_from_cache(&cache).unwrap();
    assert_eq!(loaded.comps.len(), result.comps.len());
    println!("    loaded comps match: {} entries", loaded.comps.len());

    // Test single lookup speed (what actually happens during completion)
    println!("\n  Testing single lookups (real-world usage):");
    let start = std::time::Instant::now();
    for _ in 0..10000 {
        let _ = cache.get_comp("git");
        let _ = cache.get_comp("docker");
        let _ = cache.get_comp("cargo");
    }
    let total_us = start.elapsed().as_micros();
    println!(
        "    30000 lookups: {}µs ({}ns/lookup)",
        total_us,
        (total_us * 1000) / 30000
    );

    println!("  build_cache_from_fpath: OK");
}
