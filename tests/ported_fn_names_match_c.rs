//! Enforces PORT.md's "no functions whose names aren't in zsh C source"
//! rule. Walks every free `fn` in `src/ported/**.rs` and verifies the
//! same name appears as a function definition in the upstream zsh C
//! source under `~/forkedRepos/zsh/Src/`.
//!
//! Methods inside `impl` / `trait` blocks are skipped — those map onto
//! C's struct-of-fn-pointers indirection which doesn't preserve the
//! name. Only top-level free functions count.
//!
//! Why this test exists: the substitution-bug audit on 2026-05-07
//! found two helper ported I added (`paramsubst_bridge`, `store_assign`)
//! plus seven pre-existing helpers that drifted from the freeze. The
//! port was claiming "100% port" while running 11 helpers with no C
//! counterpart. This test fails CI on any future drift so the next
//! contributor can't quietly add `helper_to_make_it_work` again.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

fn collect_rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Extract free-fn names from a Rust source file. Skips methods
/// (lines indented inside `impl` / `trait` blocks) by tracking brace
/// depth: depth 0 = module level. A `fn` at depth > 0 is a method.
/// Also skips test-only ported (#[test], #[cfg(test)] modules).
fn collect_free_fns(src: &str) -> Vec<(String, usize)> {
    let mut fns: Vec<(String, usize)> = Vec::new();
    let mut depth: i32 = 0;
    let mut in_test_mod = false;
    let mut test_mod_depth: i32 = 0;
    let mut in_block_comment: i32 = 0;

    for (lineno, line) in src.lines().enumerate() {
        let lineno = lineno + 1;
        let trimmed = line.trim_start();

        if depth == 0 && (trimmed.starts_with("mod tests {") || trimmed.starts_with("mod test {")) {
            in_test_mod = true;
            test_mod_depth = depth + 1;
        }

        // Mirror of build.rs::collect_free_fns — keep in sync.
        // Walks the line char-by-char tracking comment/string state
        // so `{`/`}` inside strings/chars/comments don't perturb depth.
        let bytes = line.as_bytes();
        let mut i = 0;
        let mut delta: i32 = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if in_block_comment > 0 {
                if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    in_block_comment -= 1;
                    i += 2;
                    continue;
                }
                if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    in_block_comment += 1;
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }
            match b {
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => break,
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                    in_block_comment += 1;
                    i += 2;
                }
                b'"' => {
                    i += 1;
                    while i < bytes.len() {
                        let c = bytes[i];
                        if c == b'\\' {
                            i += 2;
                            continue;
                        }
                        if c == b'"' {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                }
                b'r' if i + 1 < bytes.len() && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') => {
                    let mut hashes = 0;
                    let mut j = i + 1;
                    while j < bytes.len() && bytes[j] == b'#' {
                        hashes += 1;
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b'"' {
                        i = j + 1;
                        loop {
                            if i >= bytes.len() {
                                break;
                            }
                            if bytes[i] == b'"' {
                                let mut closed = 0;
                                let mut k = i + 1;
                                while k < bytes.len() && bytes[k] == b'#' && closed < hashes {
                                    closed += 1;
                                    k += 1;
                                }
                                if closed >= hashes {
                                    i = k;
                                    break;
                                }
                            }
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                }
                b'\'' => {
                    let mut j = i + 1;
                    let mut found_close = false;
                    let mut escape = false;
                    while j < bytes.len() && j - i < 12 {
                        if !escape && bytes[j] == b'\'' {
                            found_close = true;
                            break;
                        }
                        escape = bytes[j] == b'\\' && !escape;
                        j += 1;
                    }
                    if found_close {
                        i = j + 1;
                    } else {
                        i += 1;
                        while i < bytes.len()
                            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                        {
                            i += 1;
                        }
                    }
                }
                b'{' => {
                    delta += 1;
                    i += 1;
                }
                b'}' => {
                    delta -= 1;
                    i += 1;
                }
                _ => i += 1,
            }
        }
        let pre_depth = depth;
        depth += delta;
        if in_test_mod && depth < test_mod_depth {
            in_test_mod = false;
        }

        if in_test_mod {
            continue;
        }
        if pre_depth != 0 {
            continue;
        }

        // Look for `fn NAME(` patterns. Allow visibility modifiers and
        // optional `unsafe` / `async` / `extern`.
        let stripped = trimmed
            .strip_prefix("pub(crate) ")
            .or_else(|| trimmed.strip_prefix("pub(super) "))
            .unwrap_or_else(|| trimmed.strip_prefix("pub ").unwrap_or(trimmed));
        let stripped = stripped.strip_prefix("unsafe ").unwrap_or(stripped);
        let stripped = stripped.strip_prefix("async ").unwrap_or(stripped);
        let stripped = stripped.strip_prefix(r#"extern "C" "#).unwrap_or(stripped);

        if let Some(rest) = stripped.strip_prefix("fn ") {
            // Extract NAME up to `(` or `<` (generics).
            let name_end = rest
                .find(|c: char| c == '(' || c == '<' || c.is_whitespace())
                .unwrap_or(0);
            if name_end > 0 {
                let name = rest[..name_end].to_string();
                if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    fns.push((name, lineno));
                }
            }
        }
    }
    fns
}

/// Load the C function-name index from
/// `tests/data/zsh_c_fn_names.txt`. The file is checked into git so
/// the test runs in any environment without depending on a local
/// checkout of zsh's C source.
///
/// Returns a map from function name to the set of C basenames
/// (e.g. "subst.c") that contain a definition for that name.
/// Regenerate after pulling new upstream commits via:
///   `tests/data/extract_c_fn_names.sh`
fn load_c_fn_index() -> HashMap<String, HashSet<String>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/zsh_c_fn_names.txt");
    let src = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing C-function index at {} ({}). \
             Regenerate via tests/data/extract_c_fn_names.sh.",
            path.display(),
            e
        )
    });
    let mut index: HashMap<String, HashSet<String>> = HashMap::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((file, name)) = line.split_once(':') {
            index
                .entry(name.to_string())
                .or_default()
                .insert(file.to_string());
        }
    }
    index
}

// ── Header-mirror Rust files ─────────────────────────────────────
//
// `src/ported/X_h.rs` mirrors a zsh HEADER (`Src/X.h`), not a `.c`.
// The C-name index this test loads (`tests/data/zsh_c_fn_names.txt`)
// is produced by `tests/data/extract_c_fn_names.sh`, which greps
// `*.c` files ONLY:
//
//     find "$ZSH_SRC" -name "*.c" -type f | while read -r f; do
//
// A symbol `#define`d in a header is consequently NEVER recorded
// against that header — it is recorded against every `.c` file the
// preprocessor expands it into. So `WC_LIST_TYPE`, declared at
// `Src/zsh.h:919`
//
//     #define WC_LIST_TYPE(C)     wc_data(C)
//
// is indexed as `text.c` / `hist.c` / `exec.c` — its CALL SITES —
// and no `.c` basename exists that the file owning it could match.
// Same for `Src/ztype.h:48 #define idigit(X) zistype(X,IDIGIT)`,
// `Src/Zle/zle.h:316 #define Th(X) (&thingies[X])`, and every other
// header macro: the `X.rs → X.c` rule is unsatisfiable by
// construction, not because the port is in the wrong file.
//
// What is taught below is exactly that structural fact and nothing
// more: a free fn in `src/ported/X_h.rs` whose name is `#define`d in
// `Src/X.h` carries NO file constraint. Every OTHER fn in a header
// mirror still expects `X.h`, which no `.c` index entry can ever
// match — so a real `.c` function misfiled into a header mirror is
// still reported. Phase 1 (name presence in the C source) applies to
// header mirrors unchanged.
//
// The name sets below are the COMPLETE `#define` list of each header,
// not a curated subset. Regenerate any of them with:
//
//     perl -ne 'print "$1\n" if /^#\s*define\s+([A-Za-z_][A-Za-z_0-9]*)/' \
//         ~/forkedRepos/zsh/Src/<header>.h | sort -u

/// Every `#define` name in `Src/zsh.h` (726 directives).
const ZSH_H_MACROS: &[&str] = &[
    "addlinknode", "AFTERTRAPHOOK", "ALIAS_GLOBAL", "ALIAS_SUFFIX",
    "ALLOWHIST", "arena", "ARRPARAMDEF", "ASG_ARRAYP", "ASG_VALUEP", "Bang",
    "Bar", "BEFORETRAPHOOK", "BIN_PREFIX", "BINF_ADDED", "BINF_ASSIGN",
    "BINF_AUTOALL", "BINF_BUILTIN", "BINF_CLEARENV", "BINF_COMMAND",
    "BINF_DASH", "BINF_DASHDASHVALID", "BINF_EXEC", "BINF_HANDLES_OPTS",
    "BINF_KEEPNUM", "BINF_MAGICEQUALS", "BINF_NOGLOB", "BINF_PLUSOPTS",
    "BINF_PREFIX", "BINF_PRINTOPTS", "BINF_PSPECIAL", "BINF_SKIPDASH",
    "BINF_SKIPINVALID", "Bnull", "Bnullkeep", "BUILTIN", "CMDSTACKSZ",
    "COL_SEQ_BG", "COL_SEQ_FG", "Comma", "COND_AND", "COND_EF", "COND_EQ",
    "COND_GE", "COND_GT", "COND_LE", "COND_LT", "COND_MOD", "COND_MODI",
    "COND_NE", "COND_NOT", "COND_NT", "COND_OR", "COND_OT", "COND_REGEX",
    "COND_STRDEQ", "COND_STREQ", "COND_STRGTR", "COND_STRLT", "COND_STRNEQ",
    "CONDDEF", "CONDF_ADDED", "CONDF_AUTOALL", "CONDF_INFIX", "CS_ALWAYS",
    "CS_ARRAY", "CS_BQUOTE", "CS_BRACE", "CS_BRACEPAR", "CS_CASE",
    "CS_CMDAND", "CS_CMDOR", "CS_CMDSUBST", "CS_COND", "CS_COUNT", "CS_CURSH",
    "CS_DQUOTE", "CS_ELIF", "CS_ELIFTHEN", "CS_ELSE", "CS_ERRPIPE", "CS_FOR",
    "CS_FOREACH", "CS_FUNCDEF", "CS_HEREDOC", "CS_HEREDOCD", "CS_IF",
    "CS_IFTHEN", "CS_MATH", "CS_MATHSUBST", "CS_PIPE", "CS_QUOTE",
    "CS_REPEAT", "CS_SELECT", "CS_SUBSH", "CS_UNTIL", "CS_WHILE", "Dash",
    "decnode", "DEFAULT_IFS", "DEFAULT_IFS_SH", "DISABLED", "Dnull", "DPUTS",
    "DPUTS1", "DPUTS2", "DPUTS3", "dummy_patprog1", "dummy_patprog2",
    "EF_HEAP", "EF_MAP", "EF_REAL", "EF_RUN", "empty", "EMULATE_CSH",
    "EMULATE_FULLY", "EMULATE_KSH", "EMULATE_SH", "EMULATE_UNUSED",
    "EMULATE_ZSH", "EMULATION", "Equals", "ERRMSG", "EXITHOOK",
    "FDT_EXTERNAL", "FDT_FLOCK", "FDT_FLOCK_EXEC", "FDT_INTERNAL",
    "FDT_MODULE", "FDT_PROC_SUBST", "FDT_SAVED_MASK", "FDT_TYPE_MASK",
    "FDT_UNUSED", "FDT_XTRACE", "firsthist", "firstnode", "getaddrdata",
    "GETCOLORATTR", "getdata", "GETHIST_DOWNWARD", "GETHIST_EXACT",
    "GETHIST_UPWARD", "GETKEYS_BINDKEY", "GETKEYS_DOLLARS_QUOTE",
    "GETKEYS_ECHO", "GETKEYS_MATH", "GETKEYS_PRINT", "GETKEYS_PRINTF_ARG",
    "GETKEYS_PRINTF_FMT", "GETKEYS_SEP", "GETKEYS_SUFFIX", "GF_BACKREF",
    "GF_IGNCASE", "GF_LCMATCHUC", "GF_MATCHREF", "GF_MULTIBYTE", "HASHED",
    "Hat", "HEAP_ERROR", "HEAPID_FMT", "HEAPID_PERMANENT", "HFILE_APPEND",
    "HFILE_FAST", "HFILE_NO_REWRITE", "HFILE_SKIPDUPS", "HFILE_SKIPFOREIGN",
    "HFILE_SKIPOLD", "HFILE_USE_OPTIONS", "HIST_DUP", "HIST_FOREIGN",
    "HIST_MAKEUNIQUE", "HIST_NOWRITE", "HIST_OLD", "HIST_READ",
    "HIST_TMPSTORE", "HISTFLAG_DONE", "HISTFLAG_NOEXEC", "HISTFLAG_RECALL",
    "HISTFLAG_SETTY", "HOOK_SUFFIX", "HOOK_SUFFIX_LEN", "HOOKDEF",
    "HOOKF_ALL", "IN_CMD", "IN_COND", "IN_ENV", "IN_EVAL_TRAP", "IN_MATH",
    "IN_NOTHING", "IN_PAR", "Inang", "Inbrace", "Inbrack", "incnode",
    "init_list0", "init_list1", "INP_ALCONT", "INP_ALIAS", "INP_APPEND",
    "INP_CONT", "INP_FREE", "INP_HIST", "INP_HISTCONT", "INP_LINENO",
    "INP_RAW_KEEP", "Inpar", "Inparmath", "interact", "INTPARAMDEF",
    "IS_APPEND_REDIR", "IS_BASECHAR", "IS_CLOBBER_REDIR", "IS_COMBINING",
    "IS_DASH", "IS_ERROR_REDIR", "IS_READFD", "IS_REDIROP", "IS_WRITE_FILE",
    "islogin", "isset", "jobbing", "JOBTEXTSIZE", "LAST_NORMAL_TOK",
    "lastnode", "LEXFLAGS_ACTIVE", "LEXFLAGS_COMMENTS",
    "LEXFLAGS_COMMENTS_KEEP", "LEXFLAGS_COMMENTS_STRIP", "LEXFLAGS_NEWLINE",
    "LEXFLAGS_ZLE", "local_list0", "local_list1", "Marker", "MAX_ARRLEN",
    "MAX_OPS", "MAX_PIPESTATS", "MAXJOBS_ALLOC", "MB_CHARINIT", "MB_CHARLEN",
    "MB_CHARLENCONV", "MB_CUR_MAX", "MB_INCOMPLETE", "MB_INVALID",
    "MB_METACHARINIT", "MB_METACHARLEN", "MB_METACHARLENCONV",
    "MB_METASTRLEN", "MB_METASTRLEN2", "MB_METASTRLEN2END", "MB_METASTRWIDTH",
    "MB_NICECHAR", "Meta", "META_ALLOC", "META_DUP", "META_HEAPDUP",
    "META_HREALLOC", "META_NOALLOC", "META_REALLOC", "META_STATIC",
    "META_USEHEAP", "MFF_ADDED", "MFF_AUTOALL", "MFF_STR", "MFF_USERFUNC",
    "minimum", "MN_FLOAT", "MN_INTEGER", "MN_UNSET", "MOD_ALIAS", "MOD_BUSY",
    "MOD_INIT_B", "MOD_INIT_S", "MOD_LINKED", "MOD_SETUP", "MOD_UNLOAD",
    "MULTIOUNIT", "ND_NOABBREV", "ND_USERNAME", "NEWHEAPS", "nextnode",
    "nicezputs", "nonempty", "Nularg", "NULLBINCMD", "NUMMATHFUNC",
    "OLDHEAPS", "OPT_ARG", "OPT_ARG_SAFE", "OPT_HASARG", "OPT_ISSET",
    "OPT_MINUS", "OPT_PLUS", "Outang", "OutangProc", "Outbrace", "Outbrack",
    "Outpar", "Outparmath", "PAD_64_BIT", "PARAMDEF", "PAT_ANY", "PAT_FILE",
    "PAT_FILET", "PAT_HAS_EXCLUDP", "PAT_HEAPDUP", "PAT_LCMATCHUC",
    "PAT_NOANCH", "PAT_NOGLD", "PAT_NOTEND", "PAT_NOTSTART", "PAT_PURES",
    "PAT_SCAN", "PAT_STATIC", "PAT_ZDUP", "PATCHARS", "peekfirst", "peeklast",
    "PM_ABSPATH_USED", "PM_ANONYMOUS", "PM_ARRAY", "PM_AUTOALL",
    "PM_AUTOLOAD", "PM_CUR_FPATH", "PM_DECLARED", "PM_DEFAULTED",
    "PM_DONTIMPORT", "PM_DONTIMPORT_SUID", "PM_EFLOAT", "PM_EXPORTED",
    "PM_FFLOAT", "PM_HASHED", "PM_HASHELEM", "PM_HIDE", "PM_HIDEVAL",
    "PM_INTEGER", "PM_KSHSTORED", "PM_LEFT", "PM_LOADDIR", "PM_LOCAL",
    "PM_LOWER", "PM_NAMEDDIR", "PM_NAMEREF", "PM_NORESTORE", "PM_READONLY",
    "PM_READONLY_SPECIAL", "PM_REMOVABLE", "PM_RESTRICTED", "PM_RIGHT_B",
    "PM_RIGHT_Z", "PM_RO_BY_DESIGN", "PM_SCALAR", "PM_SINGLE", "PM_SPECIAL",
    "PM_TAGGED", "PM_TAGGED_LOCAL", "PM_TIED", "PM_TYPE", "PM_UNALIASED",
    "PM_UNDEFINED", "PM_UNIQUE", "PM_UNSET", "PM_UPPER", "PM_WARNNESTED",
    "PM_ZSHSTORED", "Pound", "PP_ALNUM", "PP_ALPHA", "PP_ASCII", "PP_BLANK",
    "PP_CNTRL", "PP_DIGIT", "PP_FIRST", "PP_GRAPH", "PP_IDENT", "PP_IFS",
    "PP_IFSSPACE", "PP_INCOMPLETE", "PP_INVALID", "PP_LAST", "PP_LOWER",
    "PP_PRINT", "PP_PUNCT", "PP_RANGE", "PP_SPACE", "PP_UNKWN", "PP_UPPER",
    "PP_WORD", "PP_XDIGIT", "prevnode", "PRINT_INCLUDEVALUE", "PRINT_KV_PAIR",
    "PRINT_LINE", "PRINT_LIST", "PRINT_NAMEONLY", "PRINT_POSIX_EXPORT",
    "PRINT_POSIX_READONLY", "PRINT_TYPE", "PRINT_TYPESET", "PRINT_WHENCE_CSH",
    "PRINT_WHENCE_FUNCDEF", "PRINT_WHENCE_SIMPLE", "PRINT_WHENCE_VERBOSE",
    "PRINT_WHENCE_WORD", "PRINT_WITH_NAMESPACE", "pushnode", "Qstring",
    "QT_IS_SINGLE", "Qtick", "Quest", "REDIR_FROM_HEREDOC_MASK",
    "REDIR_TYPE_MASK", "REDIR_VARID_MASK", "SCANPM_ARRONLY",
    "SCANPM_ASSIGNING", "SCANPM_CHECKING", "SCANPM_DQUOTED",
    "SCANPM_ISVAR_AT", "SCANPM_KEYMATCH", "SCANPM_MATCHKEY",
    "SCANPM_MATCHMANY", "SCANPM_MATCHVAL", "SCANPM_NOEXEC",
    "SCANPM_NONAMEREF", "SCANPM_NONAMESPC", "SCANPM_WANTINDEX",
    "SCANPM_WANTKEYS", "SCANPM_WANTVALS", "setdata", "setsizednode",
    "SFC_COMPLETE", "SFC_CWIDGET", "SFC_DIRECT", "SFC_HOOK", "SFC_NONE",
    "SFC_SIGNAL", "SFC_SUBST", "SFC_WIDGET", "SGTABTYPE", "SGTTYFLAG",
    "SHELL_EMULATION", "Snull", "SP_RUNNING", "SPECCHARS", "SPECIALPMDEF",
    "Star", "STAT_ATTACH", "STAT_BUILTIN", "STAT_CHANGED", "STAT_CURSH",
    "STAT_DISOWN", "STAT_DONE", "STAT_INUSE", "STAT_LOCKED", "STAT_NOPRINT",
    "STAT_NOSTTY", "STAT_STOPPED", "STAT_SUBJOB", "STAT_SUBJOB_ORPHANED",
    "STAT_SUBLEADER", "STAT_SUPERJOB", "STAT_TIMED", "STAT_WASSUPER",
    "STOPHIST", "String", "STRINGIFY", "STRINGIFY_LITERAL", "STRMATHFUNC",
    "STRPARAMDEF", "SUB_ALL", "SUB_BIND", "SUB_DOSUBST", "SUB_EGLOB",
    "SUB_EIND", "SUB_END", "SUB_GLOBAL", "SUB_LEN", "SUB_LIST", "SUB_LONG",
    "SUB_MATCH", "SUB_REST", "SUB_RETFAIL", "SUB_START", "SUB_SUBSTR",
    "SWITCHBACKHEAPS", "SWITCHHEAPS", "TC_COUNT", "TCALLATTRSOFF",
    "TCBACKSPACE", "TCBGCOLOUR", "TCBOLDFACEBEG", "tccan", "TCCLEAREOD",
    "TCCLEAREOL", "TCCLEARSCREEN", "TCCURINV", "TCCURVIS", "TCDEL",
    "TCDELLINE", "TCDOWN", "TCDOWNCURSOR", "TCFAINTBEG", "TCFGCOLOUR",
    "TCHORIZPOS", "TCINS", "TCINSLINE", "TCITALICSBEG", "TCITALICSEND",
    "TCLEFT", "TCLEFTCURSOR", "TCMULTDEL", "TCMULTDOWN", "TCMULTINS",
    "TCMULTLEFT", "TCMULTRIGHT", "TCMULTUP", "TCNEXTTAB", "TCRESTRCURSOR",
    "TCRIGHT", "TCRIGHTCURSOR", "TCSAVECURSOR", "TCSTANDOUTBEG",
    "TCSTANDOUTEND", "TCUNDERLINEBEG", "TCUNDERLINEEND", "TCUP", "TCUPCURSOR",
    "TERM_BAD", "TERM_NARROW", "TERM_NOUP", "TERM_SHORT", "TERM_UNKNOWN",
    "Tick", "Tilde", "TXT_ATTR_ALL", "TXT_ATTR_BG_24BIT",
    "TXT_ATTR_BG_COL_MASK", "TXT_ATTR_BG_COL_SHIFT", "TXT_ATTR_BG_MASK",
    "TXT_ATTR_COLOUR_MASK", "TXT_ATTR_FG_24BIT", "TXT_ATTR_FG_COL_MASK",
    "TXT_ATTR_FG_COL_SHIFT", "TXT_ATTR_FG_MASK", "TXT_ATTR_FONT_WEIGHT",
    "TXT_ERROR", "TXT_MULTIWORD_MASK", "TXTBGCOLOUR", "TXTBOLDFACE",
    "txtchangeget", "TXTFAINT", "TXTFGCOLOUR", "TXTITALIC", "TXTSTANDOUT",
    "TXTUNDERLINE", "TYPESET_OPTNUM", "TYPESET_OPTSTR", "uaddlinknode",
    "unset", "WC_ARITH", "WC_ASSIGN", "WC_ASSIGN_ARRAY", "WC_ASSIGN_INC",
    "WC_ASSIGN_NEW", "WC_ASSIGN_NUM", "WC_ASSIGN_SCALAR", "WC_ASSIGN_TYPE",
    "WC_ASSIGN_TYPE2", "WC_AUTOFN", "wc_bdata", "wc_bld", "WC_CASE",
    "WC_CASE_AND", "WC_CASE_FREE", "WC_CASE_HEAD", "WC_CASE_OR",
    "WC_CASE_SKIP", "WC_CASE_TESTAND", "WC_CASE_TYPE", "wc_code",
    "WC_CODEBITS", "WC_COND", "WC_COND_SKIP", "WC_COND_TYPE", "WC_COUNT",
    "WC_CURSH", "WC_CURSH_SKIP", "wc_data", "WC_END", "WC_FOR", "WC_FOR_COND",
    "WC_FOR_LIST", "WC_FOR_PPARAM", "WC_FOR_SKIP", "WC_FOR_TYPE",
    "WC_FUNCDEF", "WC_FUNCDEF_SKIP", "WC_IF", "WC_IF_ELIF", "WC_IF_ELSE",
    "WC_IF_HEAD", "WC_IF_IF", "WC_IF_SKIP", "WC_IF_TYPE", "WC_LIST",
    "WC_LIST_FREE", "WC_LIST_SKIP", "WC_LIST_TYPE", "WC_PIPE", "WC_PIPE_END",
    "WC_PIPE_LINENO", "WC_PIPE_MID", "WC_PIPE_TYPE", "WC_REDIR",
    "WC_REDIR_FROM_HEREDOC", "WC_REDIR_TYPE", "WC_REDIR_VARID",
    "WC_REDIR_WORDS", "WC_REPEAT", "WC_REPEAT_SKIP", "WC_SELECT",
    "WC_SELECT_LIST", "WC_SELECT_PPARAM", "WC_SELECT_SKIP", "WC_SELECT_TYPE",
    "WC_SIMPLE", "WC_SIMPLE_ARGC", "WC_SUBLIST", "WC_SUBLIST_AND",
    "WC_SUBLIST_COPROC", "WC_SUBLIST_END", "WC_SUBLIST_FLAGS",
    "WC_SUBLIST_FREE", "WC_SUBLIST_NOT", "WC_SUBLIST_OR", "WC_SUBLIST_SIMPLE",
    "WC_SUBLIST_SKIP", "WC_SUBLIST_TYPE", "WC_SUBSH", "WC_SUBSH_SKIP",
    "WC_TIMED", "WC_TIMED_EMPTY", "WC_TIMED_PIPE", "WC_TIMED_TYPE", "WC_TRY",
    "WC_TRY_SKIP", "WC_TYPESET", "WC_TYPESET_ARGC", "WC_WHILE",
    "WC_WHILE_SKIP", "WC_WHILE_TYPE", "WC_WHILE_UNTIL", "WC_WHILE_WHILE",
    "WCB_ARITH", "WCB_ASSIGN", "WCB_AUTOFN", "WCB_CASE", "WCB_COND",
    "WCB_CURSH", "WCB_END", "WCB_FOR", "WCB_FUNCDEF", "WCB_IF", "WCB_LIST",
    "WCB_PIPE", "WCB_REDIR", "WCB_REPEAT", "WCB_SELECT", "WCB_SIMPLE",
    "WCB_SUBLIST", "WCB_SUBSH", "WCB_TIMED", "WCB_TRY", "WCB_TYPESET",
    "WCB_WHILE", "WCWIDTH", "WCWIDTH_WINT", "WRAPDEF", "WRAPF_ADDED",
    "Z_ASYNC", "Z_DISOWN", "Z_END", "Z_SIMPLE", "Z_SYNC", "Z_TIMED",
    "zaddlinknode", "ZLONG_CONST", "ZLONG_MAX", "ZLRF_HISTORY",
    "ZLRF_IGNOREEOF", "ZLRF_NOSETTY", "zpushnode", "ZSIG_ALIAS", "ZSIG_FUNC",
    "ZSIG_IGNORED", "ZSIG_MASK", "ZSIG_SHIFT", "ZSIG_TRAPPED", "ZWC", "ZWS",
];

/// Every `#define` name in `Src/ztype.h` (45 directives).
const ZTYPE_H_MACROS: &[&str] = &[
    "ialnum", "IALNUM", "ialpha", "IALPHA", "iblank", "IBLANK", "icntrl",
    "ICNTRL", "idigit", "IDIGIT", "iident", "IIDENT", "imeta", "IMETA",
    "INAMESPC", "inblank", "INBLANK", "inull", "INULL", "ipattern",
    "IPATTERN", "isep", "ISEP", "ispecial", "ISPECIAL", "itok", "ITOK",
    "iuser", "IUSER", "iword", "IWORD", "iwsep", "IWSEP", "WC_ISPRINT",
    "WC_ZISTYPE", "ZISPRINT", "zistype", "ZTF_BANGCHAR", "ZTF_INIT",
    "ZTF_INTERACT", "ZTF_SP_COMMA",
];

/// Every `#define` name in `Src/signals.h` (31 directives).
const SIGNALS_H_MACROS: &[&str] = &[
    "child_block", "child_unblock", "dont_queue_signals", "killpg",
    "MAX_QUEUE_SIZE", "queue_signal_level", "queue_signals",
    "restore_queue_signals", "run_queued_signals", "SIGDEBUG", "SIGEXIT",
    "SIGIDX", "signal_default", "signal_ignore", "SIGNUM", "SIGZERR",
    "SV_INTERRUPT", "TRAPCOUNT", "unqueue_signals", "VSIGCOUNT",
    "winch_block", "winch_unblock",
];

/// Every `#define` name in `Src/zsh_system.h` (174 directives).
const ZSH_SYSTEM_H_MACROS: &[&str] = &[
    "__MALLOC_0_RETURNS_NULL", "_GNU_SOURCE", "_INCLUDE__STDC_A1_SOURCE",
    "_POSIX_C_SOURCE", "_STRPTIME_DONTZERO", "_XOPEN_SOURCE_EXTENDED",
    "_XPG_IV", "alloca", "BDIGBUFSIZE", "d_ino", "DEFAULT_TIMEFMT",
    "DEFAULT_WORDCHARS", "DIGBUFSIZE", "dirent", "F_OK", "fseek", "ftell",
    "GET_ST_ATIME_NSEC", "GET_ST_CTIME_NSEC", "GET_ST_MTIME_NSEC", "getlogin",
    "GETPGRP", "HAS_TIO", "HAVE_SETEGID", "HAVE_SETEUID", "HAVE_SETGID",
    "HAVE_SETREGID", "HAVE_SETRESGID", "HAVE_SETRESUID", "HAVE_SETREUID",
    "HAVE_SETUID", "HAVE_STRUCT_DIRENT_D_INO", "HAVE_STRUCT_DIRENT_D_STAT",
    "IS_DIRSEP", "lchown", "lstat", "mailstat", "memcpy", "memmove",
    "O_NOCTTY", "offsetof", "OPEN_MAX", "PATH_MAX", "R_OK", "readlink",
    "RLIM_INFINITY", "RLIM_NLIMITS", "RLIMIT_CORE", "RLIMIT_CPU",
    "RLIMIT_DATA", "RLIMIT_FSIZE", "RLIMIT_NOFILE", "RLIMIT_OPEN_MAX",
    "RLIMIT_RSS", "RLIMIT_STACK", "RLIMIT_VMEM", "S_IFMT", "S_IRGRP",
    "S_IROTH", "S_IRUGO", "S_IRUSR", "S_IRWXG", "S_IRWXO", "S_IRWXU",
    "S_ISBLK", "S_ISCHR", "S_ISDIR", "S_ISDOOR", "S_ISFIFO", "S_ISGID",
    "S_ISLNK", "S_ISMPB", "S_ISMPC", "S_ISNWK", "S_ISOFD", "S_ISOFL",
    "S_ISREG", "S_ISSOCK", "S_ISUID", "S_ISVTX", "S_IWGRP", "S_IWOTH",
    "S_IWUGO", "S_IWUSR", "S_IXGRP", "S_IXOTH", "S_IXUGO", "S_IXUSR",
    "setegid", "seteuid", "setgid", "setpgrp", "setregid", "setreuid",
    "setuid", "srand", "UNUSED", "USE_GETGRGID", "USE_GETGRNAM",
    "USE_GETPWENT", "USE_GETPWNAM", "USE_GETPWUID", "USE_INITGROUPS",
    "USE_LOCALE", "USE_SET_UNSET_ENV", "USES_TERM_H", "USES_TERMCAP_H",
    "VA_ALIST_PROTO1", "VA_ALIST_PROTO2", "VA_ALIST1", "VA_ALIST2", "VA_DCL",
    "VA_DEF_ARG", "VA_GET_ARG", "VA_START", "VARARR", "VDISABLEVAL", "W_OK",
    "WCOREDUMP", "WEXITSTATUS", "WIFEXITED", "WIFSIGNALED", "WIFSTOPPED",
    "WSTOPSIG", "WTERMSIG", "X_OK", "zopenmax", "ZSH_HAVE_NATIVE_SETREGID",
    "ZSH_HAVE_NATIVE_SETREUID", "ZSH_IMPLEMENT_SETRESGID",
    "ZSH_IMPLEMENT_SETRESUID", "ZSH_INITIAL_OPEN_MAX",
];

/// Every `#define` name in `Src/hashtable.h` (35 directives).
const HASHTABLE_H_MACROS: &[&str] = &[
    "BIN_BG", "BIN_BRACKET", "BIN_BREAK", "BIN_CD", "BIN_COMMAND",
    "BIN_CONTINUE", "BIN_DISABLE", "BIN_DISOWN", "BIN_ECHO", "BIN_ENABLE",
    "BIN_EVAL", "BIN_EXIT", "BIN_EXPORT", "BIN_FC", "BIN_FG", "BIN_JOBS",
    "BIN_LOGOUT", "BIN_POPD", "BIN_PRINT", "BIN_PRINTF", "BIN_PUSHD",
    "BIN_PUSHLINE", "BIN_R", "BIN_READONLY", "BIN_RETURN", "BIN_SCHED",
    "BIN_SETOPT", "BIN_TEST", "BIN_TYPESET", "BIN_UNALIAS", "BIN_UNFUNCTION",
    "BIN_UNHASH", "BIN_UNSET", "BIN_UNSETOPT", "BIN_WAIT",
];

/// Every `#define` name in `Src/prototypes.h` (4 directives).
const PROTOTYPES_H_MACROS: &[&str] = &[
    "SELECT_ARG_2_T", "TC_CONST",
];

/// Every `#define` name in `Src/Zle/zle.h` (157 directives).
const ZLE_H_MACROS: &[&str] = &[
    "ACCEPTCOMPHOOK", "AFTERCOMPLETEHOOK", "BEFORECOMPLETEHOOK", "CCLEFT",
    "CCLEFTPOS", "CCRIGHT", "CCRIGHTPOS", "CH_NEXT", "CH_PREV",
    "COMP_COMPLETE", "COMP_EXPAND", "COMP_EXPAND_COMPLETE", "COMP_ISEXPAND",
    "COMP_LIST_COMPLETE", "COMP_LIST_EXPAND", "COMP_SPELL", "COMPLETEHOOK",
    "CURF_BAR", "CURF_BLINK", "CURF_BLOCK", "CURF_BLUE_SHIFT", "CURF_COLOR",
    "CURF_COLOR_MASK", "CURF_DEFAULT", "CURF_GREEN_SHIFT", "CURF_HIDDEN",
    "CURF_RED_SHIFT", "CURF_SHAPE_MASK", "CURF_STEADY", "CURF_UNDERLINE",
    "CUT_FRONT", "CUT_RAW", "CUT_REPLACE", "CUT_YANK", "CUTBUFFER_LINE",
    "DECCS", "DECPOS", "INCCS", "INCPOS", "invalidatelist",
    "INVALIDATELISTHOOK", "invicmdmode", "IS_THINGY", "KRINGCTDEF",
    "LASTFULLCHAR", "LASTFULLCHAR_T", "listmatches", "LISTMATCHESHOOK",
    "METACHECK", "MOD_CHAR", "MOD_CLIP", "MOD_LINE", "MOD_MULT", "MOD_NEG",
    "MOD_NULL", "MOD_OSSEL", "MOD_PRI", "MOD_TMULT", "MOD_VIAPP", "MOD_VIBUF",
    "N_SPECIAL_HIGHLIGHTS", "NO_INSERT_CHAR", "removesuffix", "Th",
    "TH_IMMORTAL", "UNMETACHECK", "WIDGET_FREE", "WIDGET_INT", "WIDGET_INUSE",
    "WIDGET_NCOMP", "ZC_ialnum", "ZC_ialpha", "ZC_iblank", "ZC_icntrl",
    "ZC_idigit", "ZC_iident", "ZC_ilower", "ZC_inblank", "ZC_ipunct",
    "ZC_iupper", "ZC_iword", "ZC_tolower", "ZC_toupper", "ZLE_CHAR_SIZE",
    "ZLE_ISCOMP", "ZLE_KEEPSUFFIX", "ZLE_KILL", "ZLE_LASTCOL", "ZLE_LINEMOVE",
    "ZLE_MENUCMP", "ZLE_NOLAST", "ZLE_NOTCOMMAND", "ZLE_VIOPER", "ZLE_YANK",
    "ZLE_YANKAFTER", "ZLE_YANKBEFORE", "ZLEEOF", "ZMB_nicewidth", "zmult",
    "ZS_memchr", "ZS_memcmp", "ZS_memcpy", "ZS_memmove", "ZS_memset",
    "ZS_strchr", "ZS_strcpy", "ZS_strlen", "ZS_strncmp", "ZS_strncpy",
    "ZS_width", "ZS_zarrdup", "ZSH_CHAR_TO_INVALID_WCHAR",
    "ZSH_INVALID_WCHAR_BASE", "ZSH_INVALID_WCHAR_TEST",
    "ZSH_INVALID_WCHAR_TO_CHAR", "ZSH_INVALID_WCHAR_TO_INT",
];

/// Every `#define` name in `Src/Zle/comp.h` (145 directives).
const COMP_H_MACROS: &[&str] = &[
    "CAF_ALL", "CAF_ARRAYS", "CAF_KEYS", "CAF_MATCH", "CAF_MATSORT",
    "CAF_NOSORT", "CAF_NUMSORT", "CAF_QUOTE", "CAF_REVSORT", "CAF_UNIQALL",
    "CAF_UNIQCON", "CGF_FILES", "CGF_HASDL", "CGF_LINES", "CGF_MATSORT",
    "CGF_NOSORT", "CGF_NUMSORT", "CGF_PACKED", "CGF_REVSORT", "CGF_ROWS",
    "CGF_UNIQALL", "CGF_UNIQCON", "CHR_INVALID", "CLF_DIFF", "CLF_JOIN",
    "CLF_LINE", "CLF_MATCHED", "CLF_MID", "CLF_MISS", "CLF_NEW", "CLF_SKIP",
    "CLF_SUF", "CM_SPACE", "CMF_ALL", "CMF_DELETE", "CMF_DISPLINE",
    "CMF_DUMMY", "CMF_FILE", "CMF_FMULT", "CMF_HIDE", "CMF_INTER",
    "CMF_ISPAR", "CMF_LEFT", "CMF_LINE", "CMF_MORDER", "CMF_MULT",
    "CMF_NOLIST", "CMF_NOSPACE", "CMF_PACKED", "CMF_PARBR", "CMF_PARNEST",
    "CMF_REMOVE", "CMF_RIGHT", "CMF_ROWS", "COMPCTLCLEANUPHOOK",
    "COMPCTLMAKEHOOK", "COMPLISTMATCHESHOOK", "CONVCAST", "CP_ALLKEYS",
    "CP_ALLREALS", "CP_COMPSTATE", "CP_CONTEXT", "CP_CURRENT", "CP_EXACT",
    "CP_EXACTSTR", "CP_IGNORED", "CP_INSERT", "CP_INSERTP", "CP_IPREFIX",
    "CP_ISUFFIX", "CP_KEYPARAMS", "CP_LASTPROMPT", "CP_LIST", "CP_LISTLINES",
    "CP_LISTMAX", "CP_NMATCHES", "CP_OLDINS", "CP_OLDLIST", "CP_PARAMETER",
    "CP_PATINSERT", "CP_PATMATCH", "CP_PREFIX", "CP_QIPREFIX", "CP_QISUFFIX",
    "CP_QUOTE", "CP_QUOTES", "CP_QUOTING", "CP_REALPARAMS", "CP_REDIRECT",
    "CP_REDIRS", "CP_RESTORE", "CP_SUFFIX", "CP_TOEND", "CP_UNAMBIG",
    "CP_UNAMBIGC", "CP_UNAMBIGP", "CP_VARED", "CP_WORDS", "CPN_COMPSTATE",
    "CPN_CONTEXT", "CPN_CURRENT", "CPN_EXACT", "CPN_EXACTSTR", "CPN_IGNORED",
    "CPN_INSERT", "CPN_INSERTP", "CPN_IPREFIX", "CPN_ISUFFIX",
    "CPN_LASTPROMPT", "CPN_LIST", "CPN_LISTLINES", "CPN_LISTMAX",
    "CPN_NMATCHES", "CPN_OLDINS", "CPN_OLDLIST", "CPN_PARAMETER",
    "CPN_PATINSERT", "CPN_PATMATCH", "CPN_PREFIX", "CPN_QIPREFIX",
    "CPN_QISUFFIX", "CPN_QUOTE", "CPN_QUOTES", "CPN_QUOTING", "CPN_REDIRECT",
    "CPN_REDIRS", "CPN_RESTORE", "CPN_SUFFIX", "CPN_TOEND", "CPN_UNAMBIG",
    "CPN_UNAMBIGC", "CPN_UNAMBIGP", "CPN_VARED", "CPN_WORDS", "FC_INWORD",
    "FC_LINE", "INSERTMATCHHOOK", "MENUSTARTHOOK", "PATMATCHINDEX",
    "PATMATCHRANGE", "pcm_err",
];

/// Every `#define` name in `Src/Zle/compctl.h` (52 directives).
const COMPCTL_H_MACROS: &[&str] = &[
    "CC_ALGLOB", "CC_ALREG", "CC_ARRAYS", "CC_BINDINGS", "CC_BUILTINS",
    "CC_CCCONT", "CC_COMMPATH", "CC_DEFCONT", "CC_DELETE", "CC_DIRS",
    "CC_DISCMDS", "CC_ENVVARS", "CC_EXCMDS", "CC_EXPANDEXPL", "CC_EXTCMDS",
    "CC_FILES", "CC_INTVARS", "CC_JOBS", "CC_NAMED", "CC_NOSORT",
    "CC_OPTIONS", "CC_PARAMS", "CC_PATCONT", "CC_QUOTEFLAG", "CC_READONLYS",
    "CC_REMOVE", "CC_RESERVED", "CC_RESWDS", "CC_RUNNING", "CC_SCALARS",
    "CC_SHFUNCS", "CC_SPECIALS", "CC_STOPPED", "CC_UNIQALL", "CC_UNIQCON",
    "CC_USERS", "CC_VARS", "CC_XORCONT", "CCT_CURPAT", "CCT_CURPRE",
    "CCT_CURSTR", "CCT_CURSUB", "CCT_CURSUBC", "CCT_CURSUF", "CCT_NUMWORDS",
    "CCT_POS", "CCT_QUOTE", "CCT_RANGEPAT", "CCT_RANGESTR", "CCT_UNUSED",
    "CCT_WORDPAT", "CCT_WORDSTR",
];

/// Every `#define` name in `Src/Modules/tcp.h` (7 directives).
const TCP_H_MACROS: &[&str] = &[
    "INET_ADDRSTRLEN", "INET6_ADDRSTRLEN", "SUPPORT_IPV6", "ZTCP_INBOUND",
    "ZTCP_LISTEN", "ZTCP_SHUTDOWN", "ZTCP_ZFTP",
];

/// `Src/config.h` is generated by `configure` and is not checked into
/// the upstream tree, so there is no macro set to enumerate. Any free
/// fn in `src/ported/config_h.rs` is therefore constrained to
/// `config.h` (unmatchable) and will be reported — correct, since the
/// file currently has none.
const CONFIG_H_MACROS: &[&str] = &[];

/// Rust header-mirror file → (`Src/` header basename, its macro-name
/// set). Keyed on `(parent_dir, file_stem)` exactly as
/// `rust_path_to_c_files` computes them.
fn header_mirror(parent: &str, stem: &str) -> Option<(&'static str, &'static [&'static str])> {
    match (parent, stem) {
        ("ported", "zsh_h") => Some(("zsh.h", ZSH_H_MACROS)),
        ("ported", "ztype_h") => Some(("ztype.h", ZTYPE_H_MACROS)),
        ("ported", "signals_h") => Some(("signals.h", SIGNALS_H_MACROS)),
        ("ported", "zsh_system_h") => Some(("zsh_system.h", ZSH_SYSTEM_H_MACROS)),
        ("ported", "hashtable_h") => Some(("hashtable.h", HASHTABLE_H_MACROS)),
        ("ported", "prototypes_h") => Some(("prototypes.h", PROTOTYPES_H_MACROS)),
        ("ported", "config_h") => Some(("config.h", CONFIG_H_MACROS)),
        ("ported/zle", "zle_h") => Some(("zle.h", ZLE_H_MACROS)),
        ("ported/zle", "comp_h") => Some(("comp.h", COMP_H_MACROS)),
        ("ported/zle", "compctl_h") => Some(("compctl.h", COMPCTL_H_MACROS)),
        ("ported/modules", "tcp_h") => Some(("tcp.h", TCP_H_MACROS)),
        _ => None,
    }
}

/// Map a Rust ported file path to the C basename(s) it should
/// port from. Rule: `src/ported/X.rs` → `X.c`, with explicit
/// per-area overrides for files that span multiple C sources.
///
/// Returns the set of acceptable C-source basenames for this Rust
/// file. Empty set = "no constraint" (file is exempt from
/// file-mapping check, only name presence is enforced). This
/// fallback is for files we haven't categorized yet.
fn rust_path_to_c_files(rust_path: &Path, root: &Path, fn_name: &str) -> HashSet<String> {
    let rel = rust_path.strip_prefix(root).unwrap_or(rust_path);
    let stem = rel.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parent = rel.parent().and_then(|p| p.to_str()).unwrap_or("");

    // Header mirrors first — see the `header_mirror` block above for
    // why the `.c`-only index can never place a header macro.
    if let Some((header, macros)) = header_mirror(parent, stem) {
        if macros.contains(&fn_name) {
            // `#define`d in the header this file mirrors: the index
            // lists its expansion sites, so there is nothing to check.
            return HashSet::new();
        }
        // Not a macro of this header — hold it to `X.h`, which no
        // `.c` index entry matches, so a misfiled `.c` function is
        // still reported.
        let mut only_header = HashSet::new();
        only_header.insert(header.to_string());
        return only_header;
    }

    // Explicit per-file overrides — Rust files that legitimately
    // pull from more than one C source (or use a different
    // basename). Each entry MUST cite the C areas it covers.
    let mut acceptable: HashSet<String> = HashSet::new();
    match (parent, stem) {
        // Rust pattern.rs covers Src/pattern.c (same name).
        ("ported", "pattern") => {
            acceptable.insert("pattern.c".to_string());
        }
        // glob.rs covers Src/glob.c plus the glob-helper bits in
        // pattern.c (matchcat/getmatch/etc).
        ("ported", "glob") => {
            acceptable.insert("glob.c".to_string());
            acceptable.insert("pattern.c".to_string());
        }
        // utils.rs is the catch-all — same as Src/utils.c plus
        // smaller helpers from string.c, mem.c, openssh_bsd_setres_id.c.
        ("ported", "utils") => {
            acceptable.insert("utils.c".to_string());
            acceptable.insert("string.c".to_string());
            acceptable.insert("mem.c".to_string());
        }
        // params.rs covers Src/params.c plus the special-param
        // helpers in Modules/parameter.c.
        ("ported", "params") => {
            acceptable.insert("params.c".to_string());
            acceptable.insert("parameter.c".to_string());
        }
        // builtin.rs covers Src/builtin.c — many builtins live there.
        ("ported", "builtin") => {
            acceptable.insert("builtin.c".to_string());
        }
        // Modules/* maps to Src/Modules/* by name.
        (p, name) if p.starts_with("ported/modules") => {
            acceptable.insert(format!("{}.c", name));
        }
        // Zle subdir maps to Src/Zle/* by exact name.
        (p, name) if p.starts_with("ported/zle") => {
            acceptable.insert(format!("{}.c", name));
        }
        // Default: same basename + .c (e.g., subst.rs → subst.c,
        // hist.rs → hist.c, cond.rs → cond.c).
        ("ported", name) => {
            acceptable.insert(format!("{}.c", name));
        }
        _ => {}
    }
    acceptable
}

#[test]
fn ported_fns_match_c_source() {
    let mut ported_files: Vec<PathBuf> = Vec::new();
    let ported_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/ported");
    collect_rust_files(&ported_root, &mut ported_files);

    let c_index = load_c_fn_index();
    let c_names: HashSet<&String> = c_index.keys().collect();
    eprintln!("Loaded {} C function names from snapshot", c_names.len());

    // Allowlist loaded from `tests/data/fake_fn_allowlist.txt`.
    // Snapshot of pre-existing violations — anything in this file is
    // exempt-for-now. Anything NOT in this file but free-fn-without-
    // C-counterpart fails the test, blocking new drift.
    //
    // To shrink: inline the body at every call site (or rename to a
    // real C function), then remove the line from the snapshot file.
    let allowlist_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/fake_fn_allowlist.txt");
    let allowlist_src = fs::read_to_string(&allowlist_path).unwrap_or_else(|_| {
        panic!(
            "missing snapshot file {}. Generate via: \n  \
             cargo test --test ported_fn_names_match_c -- --nocapture 2>&1 | \
             grep 'no C counterpart' | sed -E 's/.*fn ([a-zA-Z_][a-zA-Z_0-9]*).*/\\1/' | sort -u",
            allowlist_path.display()
        )
    });
    let allowlist: HashSet<String> = allowlist_src
        .lines()
        .map(|l| {
            // Strip inline `# justification` so `name # comment`
            // parses to just `name`.
            let l = match l.find('#') {
                Some(i) => &l[..i],
                None => l,
            };
            l.trim()
        })
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();

    // File-mapping allowlist: pre-existing ported whose Rust file
    // doesn't match the expected C basename. Same shape as the
    // name allowlist — exempt-for-now snapshot. Anything new
    // landing in the wrong file fails immediately.
    let file_mapping_allowlist_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/ported_fn_file_mapping_allowlist.txt");
    let file_mapping_src = fs::read_to_string(&file_mapping_allowlist_path).unwrap_or_default();
    let file_mapping_allowlist: HashSet<String> = file_mapping_src
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(|l| {
            // Strip inline `# justification` so `key # comment`
            // parses to just `key`. Matches the name-allowlist
            // parser above.
            let l = match l.find('#') {
                Some(i) => &l[..i],
                None => l,
            };
            l.trim()
        })
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();

    let ported_root_canonical = ported_root
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));

    let mut name_violations: Vec<String> = Vec::new();
    let mut file_violations: Vec<String> = Vec::new();
    for path in &ported_files {
        let src = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let rel_display = path
            .strip_prefix(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
            .unwrap_or(path)
            .display()
            .to_string();
        for (name, lineno) in collect_free_fns(&src) {
            // Per-fn, because header mirrors resolve their expected
            // C area from the fn name (see `header_mirror`).
            let expected_c_files = rust_path_to_c_files(path, &ported_root_canonical, &name);
            // Phase 1: name presence check.
            if !allowlist.contains(&name) && !c_names.contains(&name) {
                name_violations.push(format!(
                    "  {}:{}  fn {} — no C counterpart in zsh source",
                    rel_display, lineno, name,
                ));
                continue;
            }
            // Phase 2: file-mapping check. Skip when the name is
            // in the name-allowlist (those are exempt globally) or
            // when the Rust file has no expected mapping.
            if allowlist.contains(&name) || expected_c_files.is_empty() {
                continue;
            }
            let mapping_key = format!("{}::{}", rel_display, name);
            if file_mapping_allowlist.contains(&mapping_key) {
                continue;
            }
            // Where does the C source actually define this name?
            if let Some(c_files) = c_index.get(&name) {
                if c_files.is_disjoint(&expected_c_files) {
                    let actual: Vec<&String> = c_files.iter().collect();
                    let mut acceptable: Vec<&String> = expected_c_files.iter().collect();
                    acceptable.sort();
                    file_violations.push(format!(
                        "  {}:{}  fn {} — defined in {:?}, but Rust file \
                         expects port from {:?}",
                        rel_display, lineno, name, actual, acceptable,
                    ));
                }
            }
        }
    }

    if !name_violations.is_empty() || !file_violations.is_empty() {
        name_violations.sort();
        file_violations.sort();
        let mut msg = String::new();
        if !name_violations.is_empty() {
            msg.push_str(&format!(
                "PORT.md freeze violation: {} NEW function(s) in src/ported/ \
                 have no matching definition in zsh's C source AND are not in \
                 the snapshot allowlist (tests/data/fake_fn_allowlist.txt). \
                 Either inline at call sites, rename to match a C function, \
                 or add to the snapshot with a justifying comment.\n\n{}\n",
                name_violations.len(),
                name_violations.join("\n")
            ));
        }
        if !file_violations.is_empty() {
            if !msg.is_empty() {
                msg.push_str("\n\n");
            }
            msg.push_str(&format!(
                "PORT.md file-mapping violation: {} function(s) in src/ported/ \
                 are defined in a Rust file whose C-counterpart basename \
                 doesn't match where the function lives in zsh's C source. \
                 Either move the fn to the right Rust file (e.g. paramsubst \
                 belongs in subst.rs because it's defined in subst.c), or \
                 add an explicit override in rust_path_to_c_files() in this \
                 test if the Rust file legitimately spans multiple C areas. \
                 Pre-existing mismatches are exempt via \
                 tests/data/ported_fn_file_mapping_allowlist.txt.\n\n{}\n",
                file_violations.len(),
                file_violations.join("\n")
            ));
        }
        panic!("{}", msg);
    }
}
