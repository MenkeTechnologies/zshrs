//! Zsh-compatible completion system (compsys)
//!
//! This module implements zsh's new completion system with full compatibility
//! for compadd, compset, zstyle, and all completion special parameters.
//!
//! # Architecture
//!
//! ## Default Mode (SQLite-backed)
//! - Cache: `~/.cache/zshrs/compsys.db` (55MB with 16,872 function bodies)
//! - `compinit`: Parallel fpath scan with rayon, stores bodies in SQLite
//! - `autoload -Xz`: Instant lookup from SQLite (~2.7µs)
//! - No .zcompdump file created
//!
//! ## --zsh-compat Mode (Traditional)
//! - Cache: `~/.zcompdump` (761KB)
//! - `compinit`: Sequential scan, creates .zcompdump
//! - `autoload -Xz`: Scans fpath/zwc files (~70µs)
//! - Full zsh behavior for debugging/compatibility
//!
//! # Usage
//! ```ignore
//! // Default mode (recommended)
//! zshrs -c 'compinit'
//!
//! // Compat mode
//! zshrs --zsh-compat -c 'compinit'
//! ```
//!
//! Architecture based on analysis of:
//! - zsh Src/Zle/compcore.c, complete.c, computil.c
//! - fish src/complete.rs (for Rust patterns)

#![allow(dead_code)]
#![allow(unused_variables)]

pub mod arguments;
pub mod base;
pub mod cache;
pub mod compadd;
pub mod compcore;
pub mod compdef;
pub mod compinit;
pub mod completion;
pub mod compset;
pub mod computil;
pub mod describe;
pub mod files;
pub mod functions;
pub mod generate;
pub mod library;
pub mod matching;
pub mod menu;
pub mod shell_runner;
pub mod state;
pub mod system;
pub mod zle;
pub mod zpwr_colors;
pub mod zstyle;

pub use arguments::{
    arguments_analyze, arguments_execute, parse_action, ActionType, ArgRequirement,
    ArgumentsAnalysis, ArgumentsSpec, ArgumentsState, OptSpec, OptType,
};
pub use base::{
    all_labels,
    alternative,
    completer_approximate,
    // Completers
    completer_complete,
    completer_correct,
    completer_expand,
    completer_history,
    completer_ignored,
    completer_match,
    completer_menu,
    completer_prefix,
    // Messages
    description as base_description,
    dispatch_complete,
    get_ignored_patterns,
    is_ignored,
    main_complete,
    message,
    multi_parts,
    next_label,
    normal_complete,
    // Tags
    requested,
    sep_parts,
    values_complete,
    wanted,
    // Utility
    Alternative,
    CompleterResult,
    CompletionContext as BaseCompletionContext,
    // Core
    MainCompleteState,
    TagManager,
    Value,
};
pub use compadd::{compadd_execute, CompadOpts};
pub use compcore::{
    do_completion, sort_and_prioritize, AmbiguousInfo, CompletionMode, CompletionRequestOptions,
    CompletionState, MenuInfo,
};
pub use compinit::{
    build_cache_from_fpath, cache_entry_count, cache_is_valid, check_dump, compdump, compinit,
    compinit_lazy, get_system_fpath, load_from_cache, CompDef, CompFile, CompFileDef, CompInitOpts,
    CompInitResult,
};
pub use completion::{
    Completion, CompletionFlags, CompletionGroup, CompletionReceiver, GroupFlags,
};
pub use compset::{
    compcall_execute, compquote_execute, compset_execute, CompcallOpts, CompquoteOpts, CompsetOp,
};
pub use computil::{
    describe_execute, ArgSpec as UtilArgSpec, CompArguments, CompDescribe, CompFiles,
    CompGroupConfig, CompGroups, CompTags, CompValues, ValueSpec,
};
pub use describe::{describe_execute as native_describe, parse_items, DescribeItem, DescribeOpts};
pub use files::{directories_execute, files_execute, FilesOpts};
pub use generate::{
    complete_builtins, complete_commands_from_cache, complete_files, complete_from_cache_function,
    complete_parameters, complete_shell_functions, detect_completion_context, generate_completions,
    CompContext,
};
pub use menu::{
    default_menuselect_bindings, parse_bindkey_output, GroupLayout, KeySequence, MenuAction,
    MenuColors, MenuItem, MenuKeymap, MenuLine, MenuMotion, MenuRendering, MenuResult, MenuState,
    SearchDirection, GROUP_COLORS,
};
pub use shell_runner::{
    call_program, BuiltinDispatcher, CompletionResult, CompletionRunner, ShellCompletionContext,
};
pub use state::{CompParams, CompState, CompletionContext};
pub use system::{groups, hosts, net_interfaces, pids, ports, signals, urls, users};
pub use zle::{ZleAction, ZleCompletionState, ZleWidgets};
pub use zpwr_colors::{
    load_zpwr_config, parse_zstyles_from_config, parse_zstyles_from_content, zpwr_list_colors,
    HeaderColors, ParsedZstyle, ZstyleColors, DEFAULT_PREFIX_COLOR, MENU_SELECTION_COLOR,
};
pub use zstyle::{
    ZStyle, ZStyleLookup, ZStyleStore, STANDARD_COMPLETERS, STANDARD_STYLES, STANDARD_TAGS,
};
