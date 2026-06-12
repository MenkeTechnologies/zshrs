//! zsh source formatter — syntax-aware reindenter.
//!
//! zshrs extension (no C counterpart: zsh ships no formatter, and
//! shfmt explicitly does not support the zsh dialect). Exposed as:
//!   * `zshrs --fmt [-w] [-i N] [-t] [FILE…]` (bins/zshrs.rs)
//!   * LSP `textDocument/formatting` (src/extensions/lsp.rs), which
//!     the IntelliJ plugin's `LspFormattingSupport` drives via
//!     Reformat Code.
//!
//! Design contract:
//!   * Indentation and trailing whitespace are rewritten; INNER
//!     spacing is normalized to idiomatic zsh — whitespace runs
//!     between tokens collapse to one space (never joining tokens,
//!     never deleting a lone separator), and the command operators
//!     get canonical spacing: `a;b` → `a; b`, `a&&b` → `a && b`,
//!     `a||b` → `a || b`, `a|b` → `a | b`, `cmd&` → `cmd &`,
//!     case terminators `x;;` → `x ;;`, funcdef `name ()` →
//!     `name()`.
//!   * Token TEXT is never rewritten, and spacing inside every
//!     semantic-bearing region is preserved byte-for-byte: quoted
//!     strings (`'…'`, `"…"`, `$'…'`, backticks), `${…}` bodies,
//!     comments' text, heredoc operators' tags, and redirection fd
//!     forms (`2>&1`, `&>`, `<&-`). Pattern contexts keep their `|`
//!     untouched: inside any `( … )` (glob alternation is always
//!     parenthesized) and in case arm-pattern position.
//!   * Heredoc bodies (`<<TAG` … `TAG`) pass through verbatim,
//!     including their terminators.
//!   * Idempotent: `format(format(x)) == format(x)`.
//!
//! The line scanner tracks just enough lexical state to find the
//! STRUCTURAL tokens that drive indentation: quotes (`'…'`, `"…"`,
//! `$'…'`), backslash escapes, comments, `${…}` parameter braces
//! (which must NOT count as block braces), command/subshell parens,
//! `[[ … ]]`, and the reserved-word pairs if/fi, do/done (for, while,
//! until, select, repeat), case/esac with pattern-arm handling, and
//! `{ … }` grouping.

/// Formatting options, mirroring the LSP `FormattingOptions` shape.
#[derive(Debug, Clone, Copy)]
pub struct FmtOptions {
    /// Spaces per indent level (ignored for emission when `use_tabs`).
    pub indent_width: usize,
    /// Emit one tab per level instead of spaces.
    pub use_tabs: bool,
}

impl Default for FmtOptions {
    fn default() -> Self {
        FmtOptions {
            indent_width: 4,
            use_tabs: false,
        }
    }
}

/// One open block on the indent stack.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Block {
    /// `if` … `fi`. `open` flips true once `then` is seen.
    If { open: bool },
    /// for/while/until/select/repeat … `done`. `open` on `do`.
    Loop { open: bool },
    /// `{` … `}` command grouping / function body.
    Brace,
    /// `(` … `)` subshell / array literal / `$(…)` / `((…))` halves.
    Paren,
    /// `[[ … ]]` conditional (multi-line conditions).
    DCond,
    /// `case` … `esac`. `sub` tracks the arm cycle.
    Case { sub: CaseSub },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CaseSub {
    /// Between `in` / `;;` and the next `pattern)` — expecting an arm.
    Pattern,
    /// Inside an arm body (after `pattern)`), until `;;` / `;&` / `;|`.
    Body,
}

impl Block {
    /// Whether this block currently indents the lines inside it.
    fn contributes(&self) -> usize {
        match self {
            Block::If { open } | Block::Loop { open } => usize::from(*open),
            Block::Brace | Block::Paren | Block::DCond => 1,
            // case body lines sit two deeper than `case` itself
            // (one for the arm label, one for the body); arm labels
            // sit one deeper. The Pattern state contributes the arm
            // level; Body adds the second level.
            Block::Case { sub } => match sub {
                CaseSub::Pattern => 1,
                CaseSub::Body => 2,
            },
        }
    }
}

/// A heredoc awaiting its terminator.
#[derive(Debug, Clone)]
struct Heredoc {
    tag: String,
    /// `<<-` — terminator may be preceded by tabs.
    strip_tabs: bool,
}

/// Format zsh source: reindent by block structure, strip trailing
/// whitespace, ensure exactly one trailing newline. See module doc
/// for the conservation guarantees.
pub fn format_source(src: &str, opts: &FmtOptions) -> String {
    let mut out = String::with_capacity(src.len() + 64);
    let mut stack: Vec<Block> = Vec::new();
    let mut pending_heredocs: Vec<Heredoc> = Vec::new();
    let mut active_heredoc: Option<Heredoc> = None;
    // Previous line ended with an unquoted `\` — indent one extra.
    let mut continuation = false;

    for line in src.split('\n') {
        // ── Heredoc body passthrough ────────────────────────────────
        if let Some(h) = &active_heredoc {
            let term_candidate = if h.strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            out.push_str(line);
            out.push('\n');
            if term_candidate == h.tag {
                active_heredoc = pending_heredocs.pop_front_or_none();
            }
            continue;
        }

        let body = line.trim_start();
        if body.is_empty() {
            // Blank line: no indent whitespace emitted.
            out.push('\n');
            continue;
        }

        // ── Normalize inner spacing to idiomatic zsh ────────────────
        // Runs BEFORE the structural scan so heredoc-tag parsing and
        // indent both see the canonical text. Seed the normalizer
        // with the line-start context the stack knows about (pattern
        // `|` and subshell-vs-glob disambiguation).
        let norm_ctx = NormCtx {
            paren_depth: stack
                .iter()
                .filter(|b| matches!(b, Block::Paren | Block::DCond))
                .count(),
            in_case_pattern: matches!(
                stack.last(),
                Some(Block::Case { sub: CaseSub::Pattern })
            ),
        };
        let body = normalize_spacing(body, norm_ctx);
        let body = body.as_str();

        // ── Idiomatic `; then` / `; do` joining ─────────────────────
        // All three style authorities agree (zsh's own
        // Etc/completion-style-guide: "the `then' or `do' should be
        // on the end of the line after a semi-colon"; OMZ wiki: "Put
        // `; do` and `; then` on the same line"; corpus counts:
        // zsh Functions/Completion 4556 `; then` vs 164 bare, zpwr
        // 636 vs 0). When the line is EXACTLY the keyword and the
        // immediately preceding emitted line is the still-pending
        // opener (top of stack If/Loop with open=false, previous
        // line non-blank, no continuation), splice `; then` / `; do`
        // onto it instead of emitting a bare-keyword line.
        if !continuation {
            let join_kw = match (body, stack.last()) {
                ("then", Some(Block::If { open: false })) => Some("; then"),
                ("do", Some(Block::Loop { open: false })) => Some("; do"),
                _ => None,
            };
            if let Some(kw) = join_kw {
                let prev_nonblank = out
                    .rsplit_once('\n')
                    .map(|(_, _last)| ())
                    .is_some()
                    && !out.ends_with("\n\n")
                    && out.len() >= 2;
                if prev_nonblank {
                    // Mark the block open (the scanner would have).
                    if let Some(b) = stack.last_mut() {
                        match b {
                            Block::If { open } | Block::Loop { open } => *open = true,
                            _ => {}
                        }
                    }
                    out.pop(); // trailing \n of the opener line
                    out.push_str(kw);
                    out.push('\n');
                    continue;
                }
            }
        }

        // ── Scan the line for structural tokens ─────────────────────
        let scan = scan_line(body, &mut stack, &mut pending_heredocs);

        // ── Compute this line's indent ──────────────────────────────
        // `leading_closers` blocks were popped by tokens at the very
        // start of the line (fi/done/esac/}/)/else/…) — the line
        // itself prints at the dedented level, which is exactly the
        // post-pop stack depth plus any re-opened mid-line blocks
        // counted in `depth_before_line`.
        let mut depth: usize = scan.indent_basis;
        if continuation {
            depth += 1;
        }
        let indent = if opts.use_tabs {
            "\t".repeat(depth)
        } else {
            " ".repeat(depth * opts.indent_width)
        };
        out.push_str(&indent);
        out.push_str(body.trim_end());
        out.push('\n');

        continuation = scan.ends_with_continuation;
        if !pending_heredocs.is_empty() && active_heredoc.is_none() {
            active_heredoc = pending_heredocs.pop_front_or_none();
        }
    }

    // Exactly one trailing newline.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    // src.split('\n') yields a trailing "" for newline-terminated
    // input which the blank-line arm turned into one extra \n — the
    // dedup loop above already collapsed it.
    out
}

/// Small helper: Vec-as-FIFO for the heredoc queue.
trait PopFront<T> {
    fn pop_front_or_none(&mut self) -> Option<T>;
}
impl<T> PopFront<T> for Vec<T> {
    fn pop_front_or_none(&mut self) -> Option<T> {
        if self.is_empty() {
            None
        } else {
            Some(self.remove(0))
        }
    }
}

struct LineScan {
    /// Indent level (in block levels) this line should print at.
    indent_basis: usize,
    /// Line ends with an unquoted backslash.
    ends_with_continuation: bool,
}

/// Current stack indent = sum of every block's contribution.
fn stack_indent(stack: &[Block]) -> usize {
    stack.iter().map(|b| b.contributes()).sum()
}

/// Scan one (whitespace-trimmed) line: update `stack` with every
/// structural token, queue heredocs, and report the indent level the
/// line itself should print at.
fn scan_line(
    body: &str,
    stack: &mut Vec<Block>,
    heredocs: &mut Vec<Heredoc>,
) -> LineScan {
    let b = body.as_bytes();
    let n = b.len();
    let mut i = 0usize;
    // Tokens seen so far on this line (false = still at line start —
    // leading closers dedent the line itself).
    let mut seen_token = false;
    // The indent the line prints at. Starts at the current stack
    // level and is lowered by leading closers as they pop.
    let mut indent_basis = stack_indent(stack);
    let mut ends_with_continuation = false;

    // Word accumulator for reserved-word recognition.
    macro_rules! at_line_start {
        () => {
            !seen_token
        };
    }

    while i < n {
        let c = b[i];
        match c {
            b' ' | b'\t' => {
                i += 1;
            }
            b'\\' => {
                // Escape: skip the next char. A `\` as the LAST char
                // is a line continuation.
                if i + 1 >= n {
                    ends_with_continuation = true;
                }
                i += 2;
                seen_token = true;
            }
            b'\'' => {
                // Single-quoted: skip to closing '.
                i += 1;
                while i < n && b[i] != b'\'' {
                    i += 1;
                }
                i += 1;
                seen_token = true;
            }
            b'"' => {
                // Double-quoted: skip to closing unescaped ". `$(…)`
                // inside DQ can span structure, but multi-line DQ
                // strings pass through verbatim anyway (the quote
                // state is line-local by design: a string that spans
                // lines leaves the remainder lines untouched at their
                // original indent only if they parse as body text —
                // acceptable for a conservative formatter).
                i += 1;
                while i < n {
                    if b[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if b[i] == b'"' {
                        break;
                    }
                    i += 1;
                }
                i += 1;
                seen_token = true;
            }
            b'`' => {
                i += 1;
                while i < n && b[i] != b'`' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                seen_token = true;
            }
            b'#' => {
                // Comment for the rest of the line (we are always at
                // a word boundary here: the word arm below consumes
                // `#` inside words like `${#var}` via the `$` arm or
                // the word scanner).
                break;
            }
            b'$' => {
                seen_token = true;
                if i + 1 < n && b[i + 1] == b'\'' {
                    // $'…' ANSI-C string.
                    i += 2;
                    while i < n && b[i] != b'\'' {
                        if b[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                    i += 1;
                } else if i + 1 < n && b[i + 1] == b'{' {
                    // ${…} parameter brace — depth-track to its `}`
                    // so it never counts as a block brace. Nested
                    // quotes inside are skipped coarsely.
                    i += 2;
                    let mut depth = 1usize;
                    while i < n && depth > 0 {
                        match b[i] {
                            b'{' => depth += 1,
                            b'}' => depth -= 1,
                            b'\\' => i += 1,
                            _ => {}
                        }
                        i += 1;
                    }
                } else {
                    // `$(`/`$((` fall through to the paren arms below
                    // on the next iteration; bare `$var` is a word.
                    i += 1;
                }
            }
            b'{' => {
                // Block-open brace. zsh brace expansion `{a,b}` is
                // glued to a word (no preceding whitespace); the
                // word scanner consumes those inline, so a bare `{`
                // here is command grouping.
                stack.push(Block::Brace);
                seen_token = true;
                i += 1;
            }
            b'}' => {
                let popped = matches!(stack.last(), Some(Block::Brace));
                if popped {
                    stack.pop();
                    if at_line_start!() {
                        indent_basis = indent_basis.saturating_sub(1);
                    }
                }
                seen_token = true;
                i += 1;
            }
            b'(' => {
                // In a case arm-pattern position, `(pat)` parens are
                // part of the pattern: push nothing, the matching `)`
                // closes the pattern.
                if let Some(Block::Case { sub: CaseSub::Pattern }) = stack.last() {
                    // Optional leading `(` of an arm pattern: skip.
                    i += 1;
                    seen_token = true;
                    continue;
                }
                stack.push(Block::Paren);
                seen_token = true;
                i += 1;
            }
            b')' => {
                match stack.last_mut() {
                    Some(Block::Case { sub }) if *sub == CaseSub::Pattern => {
                        // `pattern)` — the arm opens its body.
                        *sub = CaseSub::Body;
                        // The arm-label line prints at the Pattern
                        // contribution level; recompute nothing —
                        // label was already being printed at the
                        // pattern level via indent_basis (computed
                        // before this token executed only if the
                        // label began the line; for mid-line `)` the
                        // next lines pick up Body level naturally).
                    }
                    Some(Block::Paren) => {
                        stack.pop();
                        if at_line_start!() {
                            indent_basis = indent_basis.saturating_sub(1);
                        }
                    }
                    _ => {}
                }
                seen_token = true;
                i += 1;
            }
            b';' => {
                // `;;` / `;&` / `;|` terminate a case arm body.
                let two = b.get(i + 1).copied();
                if matches!(two, Some(b';') | Some(b'&') | Some(b'|')) {
                    if let Some(Block::Case { sub }) = stack.last_mut() {
                        if *sub == CaseSub::Body {
                            *sub = CaseSub::Pattern;
                            if at_line_start!() {
                                // `;;` on its own line prints at the
                                // BODY level (one deeper than the arm
                                // label) — matches the dominant zsh
                                // style. indent_basis already holds
                                // the pre-pop (body) level; keep it.
                            }
                        }
                    }
                    i += 2;
                } else {
                    i += 1;
                }
                seen_token = true;
            }
            b'<' => {
                // Heredoc `<<TAG` / `<<-TAG` (but not herestring `<<<`).
                if i + 1 < n && b[i + 1] == b'<' {
                    if i + 2 < n && b[i + 2] == b'<' {
                        i += 3; // <<< herestring
                    } else {
                        i += 2;
                        let strip_tabs = i < n && b[i] == b'-';
                        if strip_tabs {
                            i += 1;
                        }
                        while i < n && (b[i] == b' ' || b[i] == b'\t') {
                            i += 1;
                        }
                        // Delimiter word, possibly quoted.
                        let mut tag = String::new();
                        let mut quote: Option<u8> = None;
                        while i < n {
                            let ch = b[i];
                            match quote {
                                Some(q) if ch == q => {
                                    quote = None;
                                    i += 1;
                                }
                                Some(_) => {
                                    tag.push(ch as char);
                                    i += 1;
                                }
                                None => match ch {
                                    b'\'' | b'"' => {
                                        quote = Some(ch);
                                        i += 1;
                                    }
                                    b'\\' => {
                                        i += 1;
                                        if i < n {
                                            tag.push(b[i] as char);
                                            i += 1;
                                        }
                                    }
                                    b' ' | b'\t' | b';' | b'&' | b'|' | b'<' | b'>'
                                    | b'(' | b')' => break,
                                    _ => {
                                        tag.push(ch as char);
                                        i += 1;
                                    }
                                },
                            }
                        }
                        if !tag.is_empty() {
                            heredocs.push(Heredoc { tag, strip_tabs });
                        }
                    }
                } else {
                    i += 1;
                }
                seen_token = true;
            }
            b'>' => {
                // Redirection operator — structurally inert here, but
                // it MUST have its own arm: `>` is in the word
                // scanner's break set, so falling into the word arm
                // produced a zero-length word and the scan loop never
                // advanced (infinite loop on `(( x > 1 ))`).
                i += 1;
                seen_token = true;
            }
            b'[' => {
                // `[[ … ]]` — push only the DOUBLE form; single `[`
                // is the test command (word).
                if i + 1 < n && b[i + 1] == b'[' && is_word_boundary(b, i, 2) {
                    stack.push(Block::DCond);
                    i += 2;
                } else {
                    i += 1;
                }
                seen_token = true;
            }
            b']' => {
                if i + 1 < n
                    && b[i + 1] == b']'
                    && matches!(stack.last(), Some(Block::DCond))
                {
                    stack.pop();
                    if at_line_start!() {
                        indent_basis = indent_basis.saturating_sub(1);
                    }
                    i += 2;
                } else {
                    i += 1;
                }
                seen_token = true;
            }
            _ => {
                // Word: consume [^ \t;()<>{}'"`\\#]+ and check the
                // reserved words that drive indentation. `#` inside a
                // word (e.g. `a#b`, extendedglob) must not start a
                // comment — include it in the word.
                let start = i;
                while i < n {
                    match b[i] {
                        b' ' | b'\t' | b';' | b'(' | b')' | b'<' | b'>' | b'\''
                        | b'"' | b'`' | b'\\' => break,
                        b'{' | b'}' => {
                            // Brace glued inside a word = brace
                            // expansion / ${…} tail — consume it as
                            // word text (depth-track pairs).
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
                // Defensive forward-progress guarantee: if a byte is
                // in the word break-set but lacks its own outer arm,
                // a zero-length word would loop forever. Consume one
                // byte and move on instead.
                if i == start {
                    i += 1;
                    seen_token = true;
                    continue;
                }
                let word = &body[start..i];
                let leading = at_line_start!();
                seen_token = true;
                match word {
                    "if" => stack.push(Block::If { open: false }),
                    "then" => {
                        if let Some(Block::If { open }) = stack.last_mut() {
                            if leading && *open {
                                // bare `then` line after a multi-line
                                // condition prints at the `if` level.
                                indent_basis = indent_basis.saturating_sub(1);
                            }
                            *open = true;
                        }
                    }
                    "elif" | "else" => {
                        if leading {
                            if let Some(Block::If { open: true }) = stack.last() {
                                indent_basis = indent_basis.saturating_sub(1);
                            }
                        }
                        // elif re-arms the then-cycle; body depth is
                        // unchanged (If stays open).
                    }
                    "fi" => {
                        if matches!(stack.last(), Some(Block::If { .. })) {
                            let was_open =
                                matches!(stack.last(), Some(Block::If { open: true }));
                            stack.pop();
                            if leading && was_open {
                                indent_basis = indent_basis.saturating_sub(1);
                            }
                        }
                    }
                    "for" | "while" | "until" | "select" | "repeat" => {
                        // `while` also appears as `do … done` driver in
                        // `repeat N; do`; all push a Loop.
                        stack.push(Block::Loop { open: false });
                    }
                    "do" => {
                        if let Some(Block::Loop { open }) = stack.last_mut() {
                            if leading && *open {
                                indent_basis = indent_basis.saturating_sub(1);
                            }
                            *open = true;
                        }
                    }
                    "done" => {
                        if matches!(stack.last(), Some(Block::Loop { .. })) {
                            let was_open =
                                matches!(stack.last(), Some(Block::Loop { open: true }));
                            stack.pop();
                            if leading && was_open {
                                indent_basis = indent_basis.saturating_sub(1);
                            }
                        }
                    }
                    "case" => stack.push(Block::Case {
                        sub: CaseSub::Pattern,
                    }),
                    "esac" => {
                        if let Some(Block::Case { sub }) = stack.last() {
                            let contrib = Block::Case { sub: *sub }.contributes();
                            stack.pop();
                            if leading {
                                indent_basis =
                                    indent_basis.saturating_sub(contrib);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    LineScan {
        indent_basis,
        ends_with_continuation,
    }
}

/// Line-start context for [`normalize_spacing`], seeded from the
/// indent stack so multi-line constructs keep their guards.
#[derive(Debug, Clone, Copy)]
struct NormCtx {
    /// Open `( … )` / `[[ … ]]` levels at line start — inside any
    /// paren level `|` may be glob alternation, `;` may be a
    /// `(( ; ; ))` separator: operator spacing is left alone.
    paren_depth: usize,
    /// Stack top is a case arm-pattern position — `|` is pattern
    /// alternation, never a pipe.
    in_case_pattern: bool,
}

/// Normalize inner spacing on one (indent-trimmed) line to idiomatic
/// zsh. Token text is never rewritten; protected regions (quotes,
/// `${…}`, comments, fd-redirections) pass through verbatim. The
/// transform set:
///   * whitespace runs between tokens → one space (never joins
///     tokens, never deletes a lone separator),
///   * `a ; b` / `a ;b` / `a;b` → `a; b` (no space before, one
///     after) — skipped inside parens (`(( i=0; i<n; i++ ))`),
///   * `x;;` / `x ;; ` → `x ;;` (one space before, case style),
///     likewise `;&` / `;|`,
///   * `&&` / `||` → one space each side (also inside `[[ … ]]`;
///     skipped in paren/pattern contexts where `||` can be glob
///     alternation),
///   * `|` / `|&` at top level outside case patterns → one space
///     each side,
///   * background `&` (not `&&`, not fd forms `>&` `<&` `&>`, not
///     `&|`/`&!` which keep one space before as a unit) → one space
///     before,
///   * funcdef `name ()` → `name()`.
fn normalize_spacing(body: &str, ctx: NormCtx) -> String {
    let b = body.as_bytes();
    let n = b.len();
    let mut out = String::with_capacity(n + 8);
    let mut i = 0usize;
    let mut paren_depth = ctx.paren_depth;
    let mut case_pattern = ctx.in_case_pattern;
    // Set when we saw `case` on this line and expect `in` → patterns.
    let mut case_kw_seen = false;

    // Emit exactly one space, collapsing whatever `out` already ends
    // with; no-op at line start.
    macro_rules! one_space {
        () => {
            while out.ends_with(' ') || out.ends_with('\t') {
                out.pop();
            }
            if !out.is_empty() {
                out.push(' ');
            }
        };
    }
    // Strip any spacing `out` currently ends with (glue next token).
    macro_rules! no_space {
        () => {
            while out.ends_with(' ') || out.ends_with('\t') {
                out.pop();
            }
        };
    }

    while i < n {
        let c = b[i];
        match c {
            b' ' | b'\t' => {
                // Whitespace run → single space (the operator arms
                // below may re-trim it).
                while i < n && (b[i] == b' ' || b[i] == b'\t') {
                    i += 1;
                }
                if !out.is_empty() && i < n {
                    out.push(' ');
                }
            }
            b'\\' => {
                // Escape: verbatim with its target (or trailing `\`).
                out.push('\\');
                if i + 1 < n {
                    out.push(b[i + 1] as char);
                }
                i += 2;
            }
            b'\'' => {
                let start = i;
                i += 1;
                while i < n && b[i] != b'\'' {
                    i += 1;
                }
                i = (i + 1).min(n);
                out.push_str(&body[start..i]);
            }
            b'"' => {
                let start = i;
                i += 1;
                while i < n {
                    if b[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if b[i] == b'"' {
                        break;
                    }
                    i += 1;
                }
                i = (i + 1).min(n);
                out.push_str(&body[start..i]);
            }
            b'`' => {
                let start = i;
                i += 1;
                while i < n && b[i] != b'`' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 1).min(n);
                out.push_str(&body[start..i]);
            }
            b'#' => {
                // Comment: one space before (when not at line start),
                // text verbatim.
                one_space!();
                out.push_str(&body[i..]);
                i = n;
            }
            b'$' => {
                if i + 1 < n && b[i + 1] == b'\'' {
                    let start = i;
                    i += 2;
                    while i < n && b[i] != b'\'' {
                        if b[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                    i = (i + 1).min(n);
                    out.push_str(&body[start..i]);
                } else if i + 1 < n && b[i + 1] == b'{' {
                    let start = i;
                    i += 2;
                    let mut depth = 1usize;
                    while i < n && depth > 0 {
                        match b[i] {
                            b'{' => depth += 1,
                            b'}' => depth -= 1,
                            b'\\' => i += 1,
                            _ => {}
                        }
                        i += 1;
                    }
                    out.push_str(&body[start..i]);
                } else {
                    out.push('$');
                    i += 1;
                }
            }
            b'(' => {
                // Funcdef glue: `name ()` → `name()`. Only when the
                // pair is EMPTY and follows a word char.
                let mut j = i + 1;
                while j < n && (b[j] == b' ' || b[j] == b'\t') {
                    j += 1;
                }
                let empty_pair = j < n && b[j] == b')';
                let after_word = out
                    .trim_end()
                    .chars()
                    .last()
                    .map(|ch| ch.is_alphanumeric() || ch == '_' || ch == '.' || ch == ':' || ch == '-')
                    .unwrap_or(false);
                if empty_pair && after_word {
                    no_space!();
                    out.push_str("()");
                    i = j + 1;
                    continue;
                }
                paren_depth += 1;
                out.push('(');
                i += 1;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                if case_pattern {
                    // `pat)` — pattern ends; body follows.
                    case_pattern = false;
                }
                out.push(')');
                i += 1;
            }
            b'&' => {
                let nx = b.get(i + 1).copied();
                match nx {
                    Some(b'&') => {
                        // `&&` — one space each side, except in
                        // paren/pattern contexts (glob `(a&&b)` is
                        // nonsense but cheap to guard anyway).
                        if paren_depth == 0 && !case_pattern {
                            one_space!();
                            out.push_str("&&");
                            i += 2;
                            while i < n && (b[i] == b' ' || b[i] == b'\t') {
                                i += 1;
                            }
                            if i < n {
                                out.push(' ');
                            }
                        } else {
                            out.push_str("&&");
                            i += 2;
                        }
                    }
                    Some(b'>') => {
                        // `&>` / `&>>` redirection unit: one space
                        // before, glued to what follows.
                        one_space!();
                        out.push_str("&>");
                        i += 2;
                    }
                    Some(b'|') | Some(b'!') => {
                        // `&|` / `&!` — background + disown.
                        if paren_depth == 0 && !case_pattern {
                            one_space!();
                        }
                        out.push('&');
                        out.push(nx.unwrap() as char);
                        i += 2;
                    }
                    _ => {
                        // Bare `&`: background — one space before,
                        // UNLESS it's an fd target glue (`>&`, `<&`
                        // already consumed by the `<`/`>` arms; a
                        // digit-fd form like `2>&1` arrives here only
                        // via the `>` arm, never as bare `&`).
                        if paren_depth == 0 && !case_pattern {
                            one_space!();
                        }
                        out.push('&');
                        i += 1;
                    }
                }
            }
            b'|' => {
                let nx = b.get(i + 1).copied();
                if paren_depth == 0 && !case_pattern {
                    if nx == Some(b'|') {
                        one_space!();
                        out.push_str("||");
                        i += 2;
                    } else if nx == Some(b'&') {
                        one_space!();
                        out.push_str("|&");
                        i += 2;
                    } else {
                        one_space!();
                        out.push('|');
                        i += 1;
                    }
                    while i < n && (b[i] == b' ' || b[i] == b'\t') {
                        i += 1;
                    }
                    if i < n {
                        out.push(' ');
                    }
                } else {
                    out.push('|');
                    i += 1;
                }
            }
            b';' => {
                let nx = b.get(i + 1).copied();
                if matches!(nx, Some(b';') | Some(b'&') | Some(b'|')) {
                    // Case terminator `;;` / `;&` / `;|` — one space
                    // before (idiomatic `cmd ;;`), one after if more
                    // text follows.
                    one_space!();
                    out.push(';');
                    out.push(nx.unwrap() as char);
                    i += 2;
                    while i < n && (b[i] == b' ' || b[i] == b'\t') {
                        i += 1;
                    }
                    if i < n {
                        out.push(' ');
                    }
                    case_pattern = true;
                } else if paren_depth == 0 {
                    // Command separator: glue left, one space right.
                    no_space!();
                    out.push(';');
                    i += 1;
                    while i < n && (b[i] == b' ' || b[i] == b'\t') {
                        i += 1;
                    }
                    if i < n {
                        out.push(' ');
                    }
                } else {
                    // Inside `( … )` / `(( … ))` — e.g. the C-style
                    // for header — leave untouched.
                    out.push(';');
                    i += 1;
                }
            }
            b'<' | b'>' => {
                // Redirection cluster: consume the full operator
                // (`<`, `<<`, `<<-`, `<<<`, `>`, `>>`, `>&`, `<&`,
                // `>>&`, `>|`) verbatim so its glued fd targets
                // (`2>&1`, `<&-`) are never spaced.
                let start = i;
                i += 1;
                while i < n && matches!(b[i], b'<' | b'>' | b'&' | b'-' | b'|') {
                    i += 1;
                }
                // Glue trailing fd digit of `>&1` / `<&0` forms.
                if i > start + 1 && b[i - 1] == b'&' {
                    while i < n && b[i].is_ascii_digit() {
                        i += 1;
                    }
                    if i < n && b[i] == b'-' {
                        i += 1;
                    }
                }
                out.push_str(&body[start..i]);
            }
            _ => {
                // Word chunk: copy verbatim up to the next byte any
                // arm above handles. `{`/`}`/`[`/`]` stay word-glued.
                let start = i;
                while i < n {
                    match b[i] {
                        b' ' | b'\t' | b'\\' | b'\'' | b'"' | b'`' | b'#'
                        | b'$' | b'(' | b')' | b'&' | b'|' | b';' | b'<'
                        | b'>' => break,
                        _ => i += 1,
                    }
                }
                if i == start {
                    out.push(b[i] as char);
                    i += 1;
                    continue;
                }
                let word = &body[start..i];
                out.push_str(word);
                if word == "case" {
                    case_kw_seen = true;
                } else if word == "in" && case_kw_seen {
                    case_kw_seen = false;
                    case_pattern = true;
                }
            }
        }
    }
    out
}

/// True when the token starting at `i` with byte length `len` is
/// delimited by whitespace / line boundaries on both sides.
fn is_word_boundary(b: &[u8], i: usize, len: usize) -> bool {
    let before_ok = i == 0 || matches!(b[i - 1], b' ' | b'\t' | b'(' | b'!' | b'{');
    let after = i + len;
    let after_ok = after >= b.len() || matches!(b[after], b' ' | b'\t');
    before_ok && after_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(s: &str) -> String {
        format_source(s, &FmtOptions::default())
    }

    /// Indentation by block structure: if/for/case nesting.
    #[test]
    fn nested_blocks_reindent() {
        let src = "if true; then\nfor x in a b; do\nprint $x\ndone\nfi\n";
        let want = "if true; then\n    for x in a b; do\n        print $x\n    done\nfi\n";
        assert_eq!(fmt(src), want);
    }

    /// case arms at +1, bodies at +2, `;;` at body level, `esac` at
    /// case level. Arm-pattern `)` must not pop a paren block.
    #[test]
    fn case_arms_and_bodies() {
        let src = "case $x in\na)\nprint a\n;;\n(b|c)\nprint bc\n;;\nesac\n";
        let want = "case $x in\n    a)\n        print a\n        ;;\n    (b|c)\n        print bc\n        ;;\nesac\n";
        assert_eq!(fmt(src), want);
    }

    /// `${…}` braces never open a block; `{a,b}` brace expansion glued
    /// to a word never opens a block.
    #[test]
    fn param_and_expansion_braces_ignored() {
        let src = "print ${name:-x} file{1,2}\nprint after\n";
        assert_eq!(fmt(src), src);
    }

    /// Function bodies via `{ … }` indent; closing `}` dedents.
    #[test]
    fn function_braces() {
        let src = "f() {\nprint hi\n}\n";
        let want = "f() {\n    print hi\n}\n";
        assert_eq!(fmt(src), want);
    }

    /// Heredoc bodies and terminators pass through verbatim — no
    /// reindent, no trailing-space strip inside.
    #[test]
    fn heredoc_verbatim() {
        let src = "if true; then\ncat <<EOF\n  raw  spaces  \n\tand tabs\nEOF\nfi\n";
        let want = "if true; then\n    cat <<EOF\n  raw  spaces  \n\tand tabs\nEOF\nfi\n";
        assert_eq!(fmt(src), want);
    }

    /// `<<-TAG` terminator with leading tabs is recognized; body kept.
    #[test]
    fn heredoc_dash_tab_terminator() {
        let src = "cat <<-EOF\n\tbody\n\tEOF\nprint after\n";
        let want = "cat <<-EOF\n\tbody\n\tEOF\nprint after\n";
        assert_eq!(fmt(src), want);
    }

    /// Keywords inside quotes/comments are inert.
    #[test]
    fn quoted_keywords_inert() {
        let src = "print 'if then fi'\nprint \"do done\" # case in\nprint x\n";
        assert_eq!(fmt(src), src);
    }

    /// Line continuations indent one extra level.
    #[test]
    fn continuation_indents() {
        let src = "print one \\\ntwo\nprint three\n";
        let want = "print one \\\n    two\nprint three\n";
        assert_eq!(fmt(src), want);
    }

    /// Multi-line `$( … )` and array literals indent inside parens.
    #[test]
    fn multiline_cmdsubst_and_array() {
        let src = "files=(\na\nb\n)\nprint $files\n";
        let want = "files=(\n    a\n    b\n)\nprint $files\n";
        assert_eq!(fmt(src), want);
    }

    /// else / elif dedent to the `if` level.
    #[test]
    fn else_elif_level() {
        let src = "if a; then\nb\nelif c; then\nd\nelse\ne\nfi\n";
        let want = "if a; then\n    b\nelif c; then\n    d\nelse\n    e\nfi\n";
        assert_eq!(fmt(src), want);
    }

    /// Trailing whitespace stripped; exactly one final newline.
    #[test]
    fn trailing_ws_and_final_newline() {
        assert_eq!(fmt("print x   \n\n\n"), "print x\n");
        assert_eq!(fmt("print x"), "print x\n");
    }

    /// Idempotence on a representative composite.
    #[test]
    fn idempotent() {
        let src = "f() {\nif x; then\ncase $1 in\na) y ;;\nesac\nfi\n}\ncat <<EOF\nkeep\nEOF\n";
        let once = fmt(src);
        assert_eq!(fmt(&once), once);
    }

    /// Tabs mode emits one tab per level.
    #[test]
    fn tabs_mode() {
        let opts = FmtOptions {
            indent_width: 4,
            use_tabs: true,
        };
        let got = format_source("if x; then\ny\nfi\n", &opts);
        assert_eq!(got, "if x; then\n\ty\nfi\n");
    }

    /// Bare `then` / `do` lines join onto their opener as `; then` /
    /// `; do` — zsh Etc/completion-style-guide + OMZ rule, 96-100%
    /// of both corpora (zsh Functions/Completion, zpwr).
    #[test]
    fn then_do_join_idiomatic() {
        let src = "if true\nthen\nprint a\nfi\nfor i in 1 2\ndo\nprint $i\ndone\n";
        let want = "if true; then\n    print a\nfi\nfor i in 1 2; do\n    print $i\ndone\n";
        assert_eq!(fmt(src), want);
    }

    /// The join lands on the LAST condition line of a multi-line
    /// condition, and a blank-line gap suppresses the join.
    #[test]
    fn then_join_multiline_condition_and_blank_gap() {
        let src = "if true &&\nfalse\nthen\nx\nfi\nif true\n\nthen\ny\nfi\n";
        let want = "if true &&\nfalse; then\n    x\nfi\nif true\n\nthen\n    y\nfi\n";
        assert_eq!(fmt(src), want);
    }

    /// Inner spacing: runs dedupe to one space; `;` glues left with
    /// one space right; `&&`/`||`/`|` get canonical spacing;
    /// background `&` gets a space; quotes and `${…}` keep their
    /// bytes.
    #[test]
    fn spacing_normalized_idiomatic() {
        let src = "print  a   b;print c\nfoo&&bar||baz\nls -l|wc -l\nsleep 1&\nprint \"two  sp\" 'kept  sp' ${a:-  x}\n";
        let want = "print a b; print c\nfoo && bar || baz\nls -l | wc -l\nsleep 1 &\nprint \"two  sp\" 'kept  sp' ${a:-  x}\n";
        assert_eq!(fmt(src), want);
    }

    /// Funcdef `name ()` glues to `name()`; an empty subshell-ish
    /// pair after a non-word char is left alone.
    #[test]
    fn funcdef_paren_glue() {
        assert_eq!(fmt("f ()  {\nx\n}\n"), "f() {\n    x\n}\n");
    }

    /// Redirection fd forms are never spaced: `2>&1`, `>&-`, `<&0`,
    /// `&>`; the C-style `for (( ; ; ))` semicolons and case-pattern
    /// `a|b` alternation stay untouched.
    #[test]
    fn operator_guards_hold() {
        let src = "print x >/tmp/o 2>&1\nexec 3>&-\ncmd &>/dev/null\nfor ((i=0;i<3;i++)); do\ny\ndone\ncase $x in\na|b) z ;;\nesac\n";
        let want = "print x >/tmp/o 2>&1\nexec 3>&-\ncmd &>/dev/null\nfor ((i=0;i<3;i++)); do\n    y\ndone\ncase $x in\n    a|b) z ;;\nesac\n";
        assert_eq!(fmt(src), want);
    }

    /// `;;` gets the idiomatic single space before it when trailing a
    /// command, and one space after when text follows.
    #[test]
    fn case_terminator_spacing() {
        let src = "case $x in\na) y;;\nb) z ;;\nesac\n";
        let want = "case $x in\n    a) y ;;\n    b) z ;;\nesac\n";
        assert_eq!(fmt(src), want);
    }

    /// extendedglob `#` inside a word doesn't start a comment and
    /// `(( … ))` arithmetic stays balanced.
    #[test]
    fn hash_in_word_and_arith() {
        let src = "print ${#arr}\nif (( x > 1 )); then\ny\nfi\n";
        let want = "print ${#arr}\nif (( x > 1 )); then\n    y\nfi\n";
        assert_eq!(fmt(src), want);
    }

    /// Multi-line [[ … ]] conditions indent their continuation.
    #[test]
    fn multiline_dcond() {
        let src = "if [[ -n $a &&\n-n $b ]]; then\nx\nfi\n";
        let want = "if [[ -n $a &&\n    -n $b ]]; then\n    x\nfi\n";
        assert_eq!(fmt(src), want);
    }
}
