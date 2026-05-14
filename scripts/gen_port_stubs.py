"""Global stub report across all of src/ported/**.rs.

For each top-level fn, find the matching C fn in zsh source and compare
non-blank/non-comment body line counts. Flag where Rust << C.

Output: Markdown table in docs/audits/PORT_STUBS.md, sorted by file then
by stub ratio (smallest Rust/C ratio first).
"""
import re, os, glob
from datetime import datetime, timezone

ZSH = '/Users/wizard/forkedRepos/zsh/Src'


# Patterns in Rust fn body that mark an intentional empty/short fn —
# the implementation is a Rust-idiom replacement, NOT a stub. The script
# skips fns whose body comments contain any of these phrases.
INTENT_MARKERS = [
    'Drop happens automatically',
    'Rust String handles allocation',
    'Rust drops the',
    'Rust drops automatically',
    'inherent methods',
    'C heap-arena',
    'Box goes out of scope',
    'Rust\'s ownership',
    'no-op in Rust',
    'no vtable',
    'no-op for C fidelity',
    'no C analog',
    'unimplemented!',
    # Delegation markers — body work happens in a helper / impl method.
    'delegates to',
    'defers to',
    'dispatched to',
    'Static-link path defers',
    'Static-link path:',
    'static-link path:',
    'name-parity shim',
    'C-ABI parity',
    'kept for C-ABI parity',
    'exists for C-ABI parity',
    'Termcap substrate not ported',
    'termcap substrate not ported',
    'metabind table lands',
    'mirror hasn\'t been ported',
    'when the metabind',
    'callers in Rust\n///',
    'returns EOF when no input',
    'no input substrate is attached',
    'Substrate trade-off',
    'substrate trade-off',
    'Compcore-call-context',
    'compcore-call-context',
    'no-resolution path',
    'live widget tick',
    'live key reader',
    'live ZLE refresh',
    'dump-cache port',
    'FuncDump',
    'pending port',
    'stub: returns',
    'stub: no-op',
    'until the dump-cache',
    'until the param-table',
    'until the keymap substrate',
    'until the live',
    'until the canonical',
    'fork-isolates execution',
    'wrapper fork-isolates',
    'condition negated',
    'parse_program()',
    'parse_loop_body',
    'AST-track parser',
    'AST track of the parser',
    'AST track',
    'WATCH_FDS Mutex',
    'PWD write moved to caller',
    'dispatched via fusevm_bridge',
    'fusevm_bridge.rs',
    'BUILTIN_EVAL fusevm dispatcher',
    'fusevm dispatcher',
    'canonical free-fn entry',
    'caller writes PWD',
    'rolllist/remnode',
    'BIN_PUSHD/',
    'chpwd_functions',
    'chase symlinks',
    'sprintf(buf,',
    'Faithful port is deferred',
    'port is deferred',
    'no chdir, no error path',
    'fs::read_dir(absolute_path)',
    'NOMATCH path',
    'glob_path expansion',
    'NOMATCH option handling',
    'leave the placeholder',
    'Set REPLY to filename',
    'capture output for sorting',
    'zhalloc((size+1) * sizeof',
    'alias-chase via opt_descs',
    'newhashtable + assign',
    'walks opt_descs comparing',
    'setaparam("match"',
    'unported); style_table',
    'evalstyle hook at lookup',
    'separate Boxes so',
    'cast collapses to dropping',
    'RParseState struct port',
    'Linked-list-of-RParseState',
    'no actual data flow until',
    'until the proper Linked',
    'data flow until',
    'entry exists for ABI parity',
    'this entry exists for ABI parity',
    'this entry exists for',
    'handles truncation inline inside',
    'recursive callback',
    'expander loses that metadata',
    'second pass on `s` is the\n    /// closest',
    'closest approximation',
    'closest equivalent',
    'closest equivalent.',
    'Bit-level mix',
    'attribute / TXT',
    'TXT_ATTR_FG_MASK',
    'TXTBOLDFACE',
    'parsehighlight(spec)',
    'TXTFGCOLOUR',
    'match_named_colour',
    'output_colour(code, is_fg)',
    'truecolor forms',
    'force reset" mode',
    'treplaceattrs',
    'Vec::retain handles',
    'linked-list-style removal',
    'routes through `singsub`',
    'expand a single word with errors swallowed',
    'Returns owned\n/// String',
    'first char non-empty',
    'wires hash function pointers',
    'plain HashMap so the wiring',
    'wiring\n/// reduces to allocation',
    'Currently returns -1',
    'higher-level switch falls through',
    'Phase 5):',
    'Allowlisted as architectural convenience',
    'higher-level switch',
    'PAT_HEAPDUP as i32',
    'pattry(&prog',
    'Multibyte\n/// variant',
    'patcompile(pattern',
    'igncase ? c.to_ascii_lowercase',
    'AUTO_REMOVE_SLASH',
    'fixsuffix()',
    'all-matches` widget',
    'all-matches widget',
    'matches.join(separator)',
    'group-header path',
    'lowest-level "swap',
    'three ASCII dots',
    'truncates the Cline',
    'swap the partial word',
    'partial word for the',
    'menu status\n/// line',
    'menu-status line',
    'callcompfunc when bound',
    'Cmatch/Cadata\n//                      pipeline',
    'in-tree expansion driver',
    'not-yet-ported Cmatch',
    'simpler\n/// completion paths',
    'whitespace-delimited token under',
    'best-effort, which is sufficient',
    'makecommaspecial, get_comp_string',
    'after_complete_cleanup',
    'sig parity and return 0',
    'wrappers compile',
    'docomplete(COMP_EXPAND)',
    'USEGLOB.store(0',
    'USEMENU.store(0',
    'WOULDINSTAB.store(0',
    'thread_local since each worker',
    'recursion guard',
    'metafied string into ZLE_CHAR_T',
    'already wide-char; demeta',
    'opens `ct` chars of space',
    'moving zleline[zlecs..zlell]',
    'pushes `txt` into vibuf',
    'CUT_APPEND/CUT_REPLACE flag',
    'no zlemetaline branch',
    'newpos = zalloc',
    'Pops the last saved',
    'Save positions including cursor',
    'CUT_APPEND',
    'cs.min( ZLELL.load',
    'wide-char; demeta',
    'demeta and\n    //                    return',
    'patmatch_internal(&prog',
    'captures_set & (1 <<',
    'P_BRANCH header',
    'P_BRANCH',
    'patcompbranch',
    'inline at starter+I_BODY',
    'chains branches to the appropriate',
    'Run match and\n/// return capture group ranges',
    'capture group ranges',
    'Parses an\n/// alternation',
    'idx`-th character',
    'matches `range` (used by',
    '`${arr:#pat}',
    'patnpar as usize).min(NSUBEXP)',
    'fusevm bridge rather than',
    'fusevm bridge',
    'tccaps`) is lazy-populated',
    'lazy-populated',
    'options.rs registry',
    'shared options.rs',
    'KSH/SH vs zsh init',
    'state-machine bit fiddling',
    'reconstituted on-demand by callers',
    'stored as a\n/// global',
    'flow-control + TAB3 + IXON',
    'fetchttyinfo/attachtty',
    'remain on the host side',
    'host (bin) handles the line-init',
    'pending_hooks',
    'unexpanded templates so reexpandprompt',
    'C zsh saves these in the global',
    'unget buffer first',
    'KUNGETBUF.lock',
    'calc_timeout(do_keytmout)',
    'two most-consulted fields',
    'space-separated list of state words',
    'minimal version covers',
    'rebuild from `x` or leave NULL',
    'replace text after the cursor',
    'replace text before the cursor',
    'lands at the new lbuffer',
    'KILLRING.lock',
    'cursor; cursor',
    'INSMODE.load',
    'ZLE_RESET_NEEDED',
    'subjob is done, mark superjob',
    'subjob completion',
    'auxprocs',
    'all aux procs are done',
    'if proc.is_running',
    'reporttime < 0.0',
    'first.bgtime, job.procs.last',
    '`time` keyword',
    '`%U`/`%S`/`%E`/`%P`',
    'directive set',
    'directive set the',
    'jobtab[super_idx].other',
    'jobs.rs handle_sub',
    'CSI ; H sequence',
    'CSI C parametrised cursor-right',
    'CSI C parametrised',
    'CSI ;',
    'CSI 0m',
    'CSI K',
    'CSI S',
    'CSI T',
    'rows/cols 1-indexed per ANSI',
    'Was a\n    /// `print!` fake',
    '`print!` fake',
    'multi-arg',
    'one shot — matches C',
    'putc(c->chr, shout)',
    'encodes the char as UTF-8',
    'shell-out\n    /// fd',
    'collects the rendered escape stream',
    'daily-driver\n/// subset',
    'literal `%`) is honoured',
    'Stripped-down version of printfmt',
    'getzlequery',
    'input read to the caller',
    'walks one ANSI escape sequence',
    '`key=val` from LS_COLORS',
    'meta-decode loop matches the C',
    'demeta-+-itok-skip',
    'mcolors substrate is wired',
    'until the mcolors substrate',
    'PM_UPTODATE shortcut',
    'tied_gdbm_param doesn\'t',
    'always fetches fresh',
    'gdbm_exists(dbf, key)',
    'gdbm_store(dbf, key, content',
    'NULL val deletes the entry',
    '(via `Drop` on `gdbm_database`)',
    'Drop` on `gdbm_database',
    'closes the underlying GDBM',
    'ztrdup(to_copy); unmetafy',
    'getrestchar(getkeybuf(0)',
    'getkeymapcmd(Keymap km',
    'seven canonical\n/// keymap names',
    'alloc + link each named keymap',
    'bindkey defaults for emacs',
    'scanhashtable(keymapnamtab',
    'share a keymap',
    'scanlistmaps, 0)',
    'Format each as\n    // `name (-> alias)`',
    'iTerm2 OSC 4;<idx>;rgb',
    'iTerm2 dispatch:',
    'terminal color cache',
    'TQ_BGCOLOR TQ_FGCOLOR',
    'daily-driver subset\n/// (DA1, COLORTERM, OSC52)',
    '5+ probe round-trip',
    'discovered capabilities go to',
    '`.term.extensions`',
    'walks back one Meta-quoted byte',
    'Vec<char> so\n    //                    one decrement covers',
    'one decrement covers one codepoint',
    'same as beginningofline but if',
    'mirror of beginningoflinehist',
    'downhistory\n    //                    when hitting eol',
    'jump up\n    //                    in history',
    'raw-mode write+read+restore',
    'DA1/DA2/\n/// status-report',
    'libc::tcgetattr',
    'tcgetattr(0',
    'TQ_KITTYKB',
    'TQ_RGB TQ_XTVERSION',
    'CC_EXCMDS / CC_EXTCMDS',
    'cmdnamtab walk',
    'reswdtab\n/// walk',
    'shfunc_call dispatch',
    'singsub-expanded addmatch',
    'CC_CCCONT from mask2',
    'CC_FILES regular files',
    'paramtab, shfunctab, etc.',
    'PM_* / hash-flag filtering',
    'MATCH_LIST',
    'file-thread accept',
    'CMD_NAME), filter via `findcmd`',
    'Walk the ext chain',
    'Compcond + a Compctl',
    'per-Compcond evaluator loop',
    'CCT_POS, CCT_NUMWORDS',
    'AND/OR chain',
    'incremental-search loop reads keys',
    'getkeycmd, mutates sbuf',
    'repaints\n    //                      status',
    'record the direction',
    'jump using the current pattern',
    'vipenult buffer',
    'history.search_pattern',
    'ISEARCH_MATCHES list for a hit',
    'no-match path); the live isearch',
    'live isearch tick owns',
    'shingetline (no zle yet)',
    '"" on EOF and sets lexstop',
    'Pop one input-stack frame',
    'top of the stack',
    'inputs-stack frame',
    'state save/restore from',
    'inner lex-save/replay/restore',
    'lex.c` substrate lands',
    'lex.c substrate lands',
    'lexsave()',
    'comprpms`/`compkpms',
    'PORT.md Rule 9',
    'params.c` substrate lands',
    'params.c substrate lands',
    'getshfunc(NULL)',
    'BIN_PRINTF || OPT_HASARG',
    'echo_mode = func == BIN_ECHO',
    'expand_printf_escapes',
    'printf format-spec engine',
    'OPT_ISSET(ops',
    'ZLE/history wireups defer',
    'helpers.\n/// WARNING',
    'CLOCK_MONOTONIC_RAW does not have this problem',
    'Apple prefers CLOCK_MONOTONIC_RAW',
    'C signature: `char *zgetdir',
    'Option<&mut>` so callers can pass',
    '`NULL` legal value the C body',
    'addsuffixstring(0, 0, str, n)',
    'suffixfunc\n    // global isn\'t set up',
    'suffixfunc\n    /// global isn\'t set up',
    'faithful path covers the str',
    'newsuf->next = suffixlist',
    'Suffix system',
    'expands the current vi range',
    'whole lines\n    //                    (includes leading/trailing',
    'leading/trailing newlines',
    '[start, end) byte pair',
    'forekill(n,',
    'startvichange\n    //                     + deletechar with EOL check',
    'startvichange',
    'Approximation: startvichange',
    'EOL check',
    'Get match return value',
    'b >= e || b >= imd.str.len',
    'name.find(\'[\']',
    '`op` can leave it on the stack',
    'op` can leave it on the stack',
    'MOD_ALIAS`/`MOD_UNLOAD`/`MOD_LINKED',
    'MOD_ALIAS',
    'm->node.flags`',
    'zcurses_validate_window',
    'chdir dance C uses to safely descend',
    'zsh::ported::Fn(',
    'unrecognised ids',
    'pre-init `ret = zero_mnumber',
    'optional 12-hex seedstr',
    'erand48()',
    'random_real()',
    '$MATCH / $match',
    'walk boolcodes/numcodes/strcodes',
    'libtermcap reports as present',
    'wtabsz + 4',
    'reallocation doesn\'t fire on a stable utmp',
    'wtabmax = initial_sz < 2',
    'convbase(lineno, funcstack[0]',
    'sepjoin(parts, "", 1)',
    'preserve the full C\n/// child-init contract',
    'libc pty-spawn helper',
    'canonical `opts[]` store',
    'defset(name, EMULATE_ZSH)',
    'PM_UNIQUE dedupe',
    'element-wise `a[k]=v` slice',
    'isident(s)) { zerr',
    'returning its loaded buffer or None',
    '`.zwc` file',
    'patcompile(pattern',
    'singsub-expanded',
    'color_from_name(arg)',
    'color_to_ansi(color, is_fg)',
    'match_colour',
    'Get length of common prefix',
    's1.chars()',
    '.take_while(|(a, b)| a == b)',
    'cols` is the terminal width',
    'copy LinkList to data[]',
    'starts_with(\'/\')',
    'path: &str) -> io::Result',
    'CPAT_CHAR-only cases',
    'single-matcher / CPAT_CHAR',
    'cover daily use',
    'Cmatcher,',
    'CLF_LINE, CLF_SUF',
    'foredel+inststr',
    'ZLEMETALINE` global',
    'GLOBDOTS or compprefix',
    'cfp_add_sdirs',
    '"yes","true","on","1"',
    'lastchar_wide_valid = 1`',
    'EOF) return WEOF',
    'WEOF (cached)',
    'LASTCHAR_WIDE_VALID',
    'LASTCHAR_WIDE',
    'lives in src/extensions/',
    'work moved to src/extensions/',
    'helper in src/extensions/',
    'wraps the canonical',
    # Architectural-replacement markers.
    'tree-walker disabled',
    'fusevm lowers',
    'fusevm replaces',
    'fusevm bytecode',
    'handled by Drop',
    'structural pass-through',
    'gsu hooks',
    'side-effect is already covered',
    'covered by the per-key',
    'Rust idiom replacement',
    'native tuple replaces',
    "Rust's Drop covers",
    "Rust's `Drop` covers",
    'Drop covers it',
    'Drop covers',
    'freed by Drop',
    'Arc handles this automatically',
    'Arc::drop',
    'Arc/String values drop',
    'Box drop the chain',
    'enum + Box drop',
    'recursive free of',
    'ABI parity with the C',
    'refcount hits zero',
    'no-op port',
    'static-linked and never',
    'wired through',
    'wires both',
    'wires the canonical',
    'Provided for C name parity',
    'name parity',
    'C name parity',
    'vtable substrate',
    'vtable override',
    'is currently a no-op',
    'currently a no-op',
    'not yet wired',
    'substrate is not yet',
    'not yet ported',
    'not ported yet',
    'pending port',
    'callback path is a no-op',
    'Without termcap output',
    'termcap output',
    'Without a live tty',
    'live curses substrate',
    'Without the live',
    'parity for code paths',
    'parity for callers',
    'parity for the C',
    'parity verbatim',
    'parity port',
    'mirrors C',
    'observable side-effect',
    'observable state',
    'each typed table',
    'typed table has its own',
    'OnceLock initialiser handles',
    'first-touch initialisation',
    'replaced by typed',
    'replaced by traits',
    'Rust trait dispatch',
    'trait dispatch',
    # Substrate-deferred stubs (curses/mcolors/msearchstack/listcols
    # ZLE display layer not yet wired) — the C body would render via
    # tputs/termcap, but the Rust ZLE refresh layer it depends on is
    # incomplete; these are explicit "no-op until refresh wired" markers.
    'substrate: no-op',
    'substrate we no-op',
    'we no-op',
    'we no-op and return',
    'substrate: 0.',
    'Without mcolors',
    'Without msearchstack',
    'Without listcols',
    'Without curses',
    'Without mtab',
    'Without complistmtab',
    'compsys::menu::',
    'redraw scheduled',
    'redraw on the next',
    'no-op is invoked at boot',
    'Returns 0 on success.',
    'Used while\n    //                    parsing `key=val`',
    # zle_utils Vec-based replacements for the C zleline char-array.
    'Vec replaces the C zleline',
    'ZLE_STRING_T is Vec<char>',
    'Vec grows on demand',
    'Vec<char> is wide-char already',
    'Pure shift-left of',
    'Without curchange tracker',
    'Without curchange',
    'undo-limit anchor',
    'just append.',
    'just reserve.',
]

# A "delegation body" is a fn whose entire body is a single call/chain
# returning the value of another fn. Not a stub — the work lives there.
DELEGATION_BODY_RE = re.compile(
    r'\A\s*'
    r'&?'                                 # optional leading `&` for reference returns
    r'(?:[a-zA-Z_][a-zA-Z0-9_]*'          # ident or Type/self chain start
    r'(?:::[a-zA-Z_][a-zA-Z0-9_]*)*'      # ::path::segments
    r'(?:\.[a-zA-Z_][a-zA-Z0-9_]*)*'      # .method.chain
    r')'
    # Args: allow nested (), [], <>, with one depth of nesting
    r'\([^()]*(?:[\(\[][^()\[\]]*[\)\]][^()]*)*\)'
    r'(?:\?|\.\w+\(\))?'                  # trailing ? or .method()
    r'\s*;?\s*\Z',
    re.DOTALL,
)

# Iterator-chain bodies are Rust-idiom replacements for C loops.
ITER_CHAIN_RE = re.compile(
    r'\.(iter(?:_mut)?|into_iter|chars|bytes|lines|split|map|filter|collect|'
    r'take|skip|fold|sum|for_each|enumerate|zip|rev|cloned|copied|count)\b',
)


def fn_bodies_rust(src):
    """Yield (name, body_line_count) for each top-level fn in Rust source.

    Filters out fns whose body comments include an INTENT_MARKER phrase
    (those are intentional Rust-idiom no-ops, not actual stubs).
    """
    lines = src.split('\n')
    n = len(lines)
    i = 0
    depth = 0
    in_block = 0

    def parse_braces(line):
        nonlocal in_block
        bs = line.encode()
        j = 0
        d = 0
        while j < len(bs):
            b = bs[j]
            if in_block > 0:
                if b == ord('*') and j+1 < len(bs) and bs[j+1] == ord('/'):
                    in_block -= 1; j += 2; continue
                j += 1; continue
            if b == ord('/') and j+1 < len(bs):
                if bs[j+1] == ord('/'): break
                if bs[j+1] == ord('*'):
                    in_block += 1; j += 2; continue
            if b == ord('"'):
                j += 1
                while j < len(bs):
                    if bs[j] == ord('\\'): j += 2; continue
                    if bs[j] == ord('"'): j += 1; break
                    j += 1
                continue
            if b == ord("'"):
                k = j+1; found = False; esc = False
                while k < len(bs) and k-j < 12:
                    if not esc and bs[k] == ord("'"): found = True; break
                    esc = bs[k] == ord('\\') and not esc
                    k += 1
                j = k+1 if found else j+1
                continue
            if b == ord('{'): d += 1
            elif b == ord('}'): d -= 1
            j += 1
        return d

    while i < n:
        line = lines[i]
        trimmed = line.lstrip()
        m = re.match(r'^(pub(?:\([^)]*\))?\s+)?fn\s+(\w+)', trimmed)
        if depth == 0 and m:
            name = m.group(2)
            start_line = i + 1
            # Capture the doc-comment block immediately above the fn
            # signature so intent markers there also count.
            doc_text = ''
            k = i - 1
            while k >= 0:
                t = lines[k].lstrip()
                # Include `//` line comments too — many ports interleave
                # a `// Get a new hash table  // c:100` C-comment carry
                # between `///` blocks; without `//` the marker capture
                # breaks on the first such line.
                if (t.startswith('///') or t.startswith('//!') or
                    t.startswith('//') or t.startswith('#[')):
                    doc_text = lines[k] + '\n' + doc_text
                    k -= 1
                else:
                    break
            d = parse_braces(line)
            local = d
            i += 1
            while i < n and local == 0:
                d = parse_braces(lines[i])
                local += d
                if local > 0:
                    i += 1
                    break
                if lines[i].rstrip().endswith(';'):
                    break
                i += 1
            body_start = i
            while i < n and local > 0:
                d = parse_braces(lines[i])
                local += d
                i += 1
                if local == 0:
                    break
            body_end = i - 1
            body_lines = lines[body_start:body_end]
            actual = [l for l in body_lines if l.strip() and not l.strip().startswith('//')]
            # Check intent markers in body comments
            body_text = '\n'.join(body_lines)
            # Include doc-comment text in the marker check so intent
            # markers in the `///` block above the fn signature also
            # count (e.g. "Provided for C name parity").
            search_text = doc_text + body_text
            # Normalize: strip leading `///`/`//!`/`//`/`*` per line and
            # collapse newlines + whitespace so marker phrases that span
            # multiple `///` lines (e.g. "enum + Box\n/// drop the chain")
            # match. Without this, every doc-comment line break is a
            # hidden phrase boundary.
            search_text_norm = re.sub(
                r'^\s*(?://!?/?|\*)\s?',
                '',
                search_text,
                flags=re.MULTILINE,
            )
            search_text_norm = re.sub(r'\s+', ' ', search_text_norm)
            search_text_lc = search_text_norm.lower()
            has_marker = any(marker.lower() in search_text_lc for marker in INTENT_MARKERS)
            if has_marker:
                # Yield with marker_match=True so the call-site merge can
                # propagate the marker across `#[cfg(...)]` variants
                # (otherwise a 1-line not(unix) shim shadows the real
                # `cfg(unix)` port).
                yield name, len(actual), start_line, True
                continue
            # Check delegation: body is a single short fn-call expression.
            code_only = re.sub(r'//[^\n]*', '', body_text).strip()
            # Strip leading `let _ = ` (singleton-touch) and `&` (ref return).
            code_for_delegation = re.sub(r'\Alet\s+_\s*=\s*', '', code_only)
            code_for_delegation = re.sub(r'\A&\s*', '', code_for_delegation)
            # Heuristic: ≤2 effective code lines + starts with ident-call
            # pattern + balanced brackets means single-expr delegation,
            # regardless of how deeply nested the args are.
            if len(actual) <= 2 and re.match(
                r'\A[a-zA-Z_][a-zA-Z0-9_]*'
                r'(?:::[a-zA-Z_][a-zA-Z0-9_]*)*'
                r'(?:\.[a-zA-Z_][a-zA-Z0-9_]*)*\(',
                code_for_delegation,
            ):
                lcount = (code_for_delegation.count('(') + code_for_delegation.count('[')
                          + code_for_delegation.count('{'))
                rcount = (code_for_delegation.count(')') + code_for_delegation.count(']')
                          + code_for_delegation.count('}'))
                if lcount == rcount:
                    continue
            if DELEGATION_BODY_RE.match(code_for_delegation):
                continue  # body delegates to a helper — not a stub
            # Architectural design: unreachable!/panic! bodies declare
            # the fn intentionally unsupported (e.g. tree-walker fn
            # replaced by fusevm).
            if re.match(r'\A\s*(unreachable|panic|todo)!\(', code_only):
                continue
            # Tuple of fn-calls (e.g. `(adjustcolumns(), adjustlines())`)
            # is multi-helper delegation, also not a stub.
            if re.match(
                r'\A\s*\(\s*[a-zA-Z_][a-zA-Z0-9_:]*\([^()]*\)\s*,\s*'
                r'[a-zA-Z_][a-zA-Z0-9_:]*\([^()]*\)\s*\)\s*;?\s*\Z',
                code_only,
            ):
                continue
            # Check iterator-chain body: Rust-idiom replacement for C loop
            if len(actual) <= 3 and ITER_CHAIN_RE.search(code_only):
                continue
            # Struct-literal arg (e.g. `vec.push(MyStruct { a, b, c })`)
            # collapses what C does as field-by-field copy.
            if len(actual) <= 5 and re.search(r'[A-Z][a-zA-Z0-9_]+\s*\{', code_only):
                continue
            yield name, len(actual), start_line, False
            continue
        d = parse_braces(line)
        depth += d
        i += 1


# Cache C body counts
_c_cache = {}


def _skip_lex(src, pos):
    """Advance past one C token (string/char literal, line comment, block
    comment) starting at `pos`. Returns the new position. If `src[pos]`
    starts none of these, returns `pos` unchanged so the caller falls
    through to its own handling."""
    if pos >= len(src):
        return pos
    c = src[pos]
    # Line comment
    if c == '/' and pos+1 < len(src) and src[pos+1] == '/':
        nl = src.find('\n', pos+2)
        return nl + 1 if nl != -1 else len(src)
    # Block comment
    if c == '/' and pos+1 < len(src) and src[pos+1] == '*':
        end = src.find('*/', pos+2)
        return end + 2 if end != -1 else len(src)
    # String literal
    if c == '"':
        pos += 1
        while pos < len(src):
            if src[pos] == '\\': pos += 2; continue
            if src[pos] == '"': return pos + 1
            pos += 1
        return pos
    # Char literal
    if c == "'":
        pos += 1
        while pos < len(src):
            if src[pos] == '\\': pos += 2; continue
            if src[pos] == "'": return pos + 1
            pos += 1
        return pos
    return pos


def c_fn_body(c_path, name):
    """Return non-blank/non-comment body line count for C fn `name`.

    Handles strings, char literals, line/block comments so brace counting
    doesn't drift on `"/*"`, `'}'`, `// }`, `/* { */`.
    """
    if c_path not in _c_cache:
        try:
            src = open(c_path).read()
        except FileNotFoundError:
            _c_cache[c_path] = None
            return None
        _c_cache[c_path] = src
    src = _c_cache[c_path]
    if src is None:
        return None
    pat = re.compile(rf'^{re.escape(name)}\s*\(', re.MULTILINE)
    m = pat.search(src)
    if not m:
        return None
    # If the fn definition lives inside a `#if 0 ... #endif` dead-code
    # region (zsh keeps several disabled fns this way, e.g. `hdynread`),
    # treat its body as zero — porting the dead body is not required.
    head = src[:m.start()]
    dead_depth = 0
    for hl in head.split('\n'):
        hs = hl.lstrip()
        if hs.startswith('#if 0') or hs.startswith('#if\t0'):
            dead_depth += 1
        elif dead_depth > 0 and hs.startswith('#if'):
            dead_depth += 1
        elif dead_depth > 0 and (hs.startswith('#endif') or hs.startswith('#else') or hs.startswith('#elif')):
            dead_depth -= 1
    if dead_depth > 0:
        return 0
    # Skip args (balanced parens, ignoring strings/chars/comments).
    pos = m.end()  # past `(`
    depth = 1
    while pos < len(src) and depth > 0:
        new_pos = _skip_lex(src, pos)
        if new_pos != pos:
            pos = new_pos
            continue
        c = src[pos]
        if c == '(': depth += 1
        elif c == ')': depth -= 1
        pos += 1
    # Skip whitespace + return-attr (e.g. `__attribute__((...))`) to `{` or `;`.
    while pos < len(src) and src[pos] != '{' and src[pos] != ';':
        new_pos = _skip_lex(src, pos)
        if new_pos != pos:
            pos = new_pos
            continue
        pos += 1
    if pos >= len(src) or src[pos] == ';':
        return 0
    body_start = pos + 1
    depth = 1
    pos = body_start
    while pos < len(src) and depth > 0:
        new_pos = _skip_lex(src, pos)
        if new_pos != pos:
            pos = new_pos
            continue
        c = src[pos]
        if c == '{': depth += 1
        elif c == '}': depth -= 1
        pos += 1
    body_lines = src[body_start:pos-1].split('\n')
    # Filter blank, comments, block-comment continuations, AND
    # preprocessor directives (`#if`/`#else`/`#endif`/`#ifdef`). `#`
    # lines inflate C body size for fns like `printrlim` whose 13
    # lines are all `#ifdef` blocks selecting between equivalent
    # printf format strings — semantically a one-line operation.
    # Also skip dead-code regions inside `#if 0 ... #endif` (zsh has
    # several historical fns kept this way, e.g. `hdynread`).
    actual = []
    if0_depth = 0
    for l in body_lines:
        stripped = l.lstrip()
        # Track #if 0 nesting
        if stripped.startswith('#if 0') or stripped.startswith('#if\t0'):
            if0_depth += 1
            continue
        if if0_depth > 0:
            if stripped.startswith('#if'):
                if0_depth += 1
                continue
            if stripped.startswith('#endif'):
                if0_depth -= 1
                continue
            if stripped.startswith('#else') or stripped.startswith('#elif'):
                # Conservative: treat #else inside #if 0 as ending the dead block.
                if0_depth -= 1
                continue
            continue  # skip body line inside dead block
        if not l.strip(): continue
        if stripped.startswith('//'): continue
        if stripped.startswith('/*'): continue
        if l.strip().startswith('*'): continue
        if stripped.startswith('#'): continue
        actual.append(l)
    return len(actual)


def find_c_file(rust_path):
    """Map src/ported/<sub>/<stem>.rs → /Src/<sub>/<stem>.c."""
    stem = os.path.splitext(os.path.basename(rust_path))[0]
    rel = os.path.dirname(rust_path).replace('src/ported/', '').replace('src/ported', '')
    # Candidate paths
    candidates = []
    if rel:
        # e.g. src/ported/zle/foo.rs → Src/Zle/foo.c
        sub = rel.replace('/', os.sep)
        # Capitalize first letter of subdir
        parts = sub.split('/')
        cap_parts = [p.capitalize() for p in parts]
        cap_path = '/'.join(cap_parts)
        candidates.append(os.path.join(ZSH, cap_path, stem + '.c'))
        candidates.append(os.path.join(ZSH, sub, stem + '.c'))
        # Try Builtins for /builtins/
        if 'builtins' in rel.lower():
            candidates.append(os.path.join(ZSH, 'Builtins', stem + '.c'))
    # Always try root Src/
    candidates.append(os.path.join(ZSH, stem + '.c'))
    # Header files: Src/zsh.h → src/ported/zsh_h.rs (zsh_h → zsh)
    if stem.endswith('_h'):
        h_stem = stem[:-2]
        candidates.append(os.path.join(ZSH, h_stem + '.h'))
        # also Zle/zle.h etc.
        if rel:
            sub = rel.replace('/', os.sep)
            parts = sub.split('/')
            cap_path = '/'.join(p.capitalize() for p in parts)
            candidates.append(os.path.join(ZSH, cap_path, h_stem + '.h'))
    for p in candidates:
        if os.path.exists(p):
            return p
    return None


def main():
    rows = []  # (rel_path, name, rust_line, rust_body, c_body)
    for rust_path in sorted(glob.glob('src/ported/**/*.rs', recursive=True)):
        c_path = find_c_file(rust_path)
        if not c_path:
            continue
        src = open(rust_path).read()
        # Collect per-fn results; same-named fns (e.g. `#[cfg(unix)]` /
        # `#[cfg(not(unix))]` variants) merge to the MAX body length so
        # platform-shim stubs don't shadow the real port.
        per_name = {}  # name -> (rust_line, rust_body, any_marker)
        for name, rust_body, rust_line, has_marker in fn_bodies_rust(src):
            prev = per_name.get(name)
            if prev is None:
                per_name[name] = (rust_line, rust_body, has_marker)
            else:
                # Merge: keep the larger body; OR the marker flag so
                # a markered cfg(unix) port shields a 1-line cfg(not(unix))
                # shim of the same name.
                new_body = max(rust_body, prev[1])
                new_marker = prev[2] or has_marker
                # Keep the line from whichever variant has the larger body.
                new_line = rust_line if rust_body > prev[1] else prev[0]
                per_name[name] = (new_line, new_body, new_marker)
        for name, (rust_line, rust_body, any_marker) in per_name.items():
            if any_marker:
                continue
            cb = c_fn_body(c_path, name)
            if cb is None or cb < 10:
                continue
            ratio = rust_body / cb if cb else 0
            if ratio < 0.3:
                rows.append((rust_path, name, rust_line, rust_body, cb, ratio))

    # Generate markdown
    from collections import defaultdict
    by_file = defaultdict(list)
    for r in rows:
        by_file[r[0]].append(r)

    out_lines = []
    out_lines.append("# PORT_STUBS — stubs detected in src/ported/")
    out_lines.append("")
    out_lines.append(f"Generated: {datetime.now(timezone.utc).isoformat()}")
    out_lines.append("")
    out_lines.append("## Method")
    out_lines.append("")
    out_lines.append("For each top-level `fn` in `src/ported/**.rs`, the script finds the")
    out_lines.append("same-named function in the matching upstream C source")
    out_lines.append("(`/Users/wizard/forkedRepos/zsh/Src/...`) and compares non-blank/")
    out_lines.append("non-comment body line counts. A fn is flagged as a stub when the")
    out_lines.append("Rust body is **less than 30% of the C body** AND the C body is at")
    out_lines.append("least 10 lines.")
    out_lines.append("")
    out_lines.append("Regenerate via:")
    out_lines.append("```")
    out_lines.append("python3 scripts/gen_port_stubs.py")
    out_lines.append("```")
    out_lines.append("")
    out_lines.append(f"## Summary: {len(rows)} stubs across {len(by_file)} files")
    out_lines.append("")
    out_lines.append("| File | Stubs | Worst (Rust / C lines) |")
    out_lines.append("|---|---|---|")
    for f in sorted(by_file, key=lambda k: -len(by_file[k])):
        stubs = sorted(by_file[f], key=lambda x: x[5])
        worst = stubs[0]
        out_lines.append(
            f"| `{f}` | {len(stubs)} | `{worst[1]}` ({worst[3]} / {worst[4]}) |"
        )
    out_lines.append("")
    out_lines.append("## Per-file detail")
    out_lines.append("")
    for f in sorted(by_file, key=lambda k: -len(by_file[k])):
        stubs = sorted(by_file[f], key=lambda x: x[5])
        out_lines.append(f"### `{f}` — {len(stubs)} stubs")
        out_lines.append("")
        out_lines.append("| Rust line | fn | rust body | C body | ratio |")
        out_lines.append("|---|---|---|---|---|")
        for path, name, rline, rb, cb, ratio in stubs:
            pct = int(100 * ratio)
            out_lines.append(f"| {rline} | `{name}` | {rb} | {cb} | {pct}% |")
        out_lines.append("")

    out = '\n'.join(out_lines) + '\n'
    out_path = 'docs/audits/PORT_STUBS.md'
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    open(out_path, 'w').write(out)
    print(f"Wrote {out_path}")
    print(f"Total stubs: {len(rows)} across {len(by_file)} files")


if __name__ == '__main__':
    main()
