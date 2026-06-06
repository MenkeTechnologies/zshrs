package com.menketechnologies.zshrs

import com.intellij.lexer.LexerBase
import com.intellij.psi.TokenType
import com.intellij.psi.tree.IElementType

/**
 * Hand-rolled zsh lexer. Recognizes:
 *
 *   * `#` line comments, `#!` shebang
 *   * single/double/backtick strings, `$'...'` (ANSI-C quoting), heredocs
 *   * integers and reserved-word keywords (if/then/else/elif/fi/for/while/do/done/case/esac/select/repeat/foreach/until)
 *   * sigil variables `$name`, `${name}`, special `$0`/`$?`/`$!`/`$$`/`$#`/etc
 *   * pipes `|`, `|&`; redirects `>`, `>>`, `<`, `<<`, `<<<`, `2>&1`, `&>`
 *   * logical operators `&&`, `||`; backgrounding `&`
 *   * glob chars `*`, `?`, `[...]`, `(#qX)`-style qualifiers (lightly)
 *   * brace expansion `{a,b,c}` / `{1..10}` (brace token, content not parsed)
 *   * assignment `=`, `+=`, `?=`, `:=`
 *   * paren / brace / bracket / comma / semicolon / `;;` (case) as separate
 *     categories so they can be colored independently
 *
 * The LSP server (zshrs --lsp) overlays semantic tokens; this lexer provides
 * instant feedback before the LSP turn-around.
 */
class ZshrsLexer : LexerBase() {
    private var buf: CharSequence = ""
    private var endOffset = 0
    private var pos = 0
    private var tokenStart = 0
    private var tokenEnd = 0
    private var tokenType: IElementType? = null
    private var state = 0

    // Depth of unclosed `${…}` openers. Incremented when we emit a `${`
    // opener (PARAM_EXPANSION), decremented when we hit the matching
    // `}`. Used to recognize the closing `}` as part of the param
    // expansion (PARAM_EXPANSION token, same color) rather than a bare
    // RBRACE / command-group close. Nested `${a${b}}` increments twice.
    private var paramBraceDepth = 0

    // When `${…}` was opened inside an interpolating string, save the
    // resume-state here so the matching `}` can pop us back into
    // string-content mode. Required because while inside `${…}` we
    // want the NORMAL lexer to run (the body is param-expansion
    // grammar, not string content); resuming string-content mode too
    // early would treat `HOME:-/tmp}y"` as a single string segment.
    private var pendingStringResumeState = STATE_NORMAL
    /// Paren-depth tracker used by [lexInsideDqArith] and
    /// [lexInsideDqCmdsub] to know when the matching `))` / `)` closes
    /// the interpolation block and we should flip back to the
    /// resume state. Reset on every `$((` / `$(` opener.
    private var dqInterpParenDepth = 0
    /// Where to flip back to after the matching `))` / `)` closes a
    /// `$((arith))` / `$(cmd)` interpolation. Set when the opener is
    /// emitted in [consumeDollar]:
    ///   * `STATE_NORMAL` when the opener was at top level (NOT inside
    ///     a `"..."` string) — the close should return to the outer
    ///     normal-state dispatcher.
    ///   * `STATE_DQ_RESUME` when the opener was inside a `"..."`
    ///     string — the close should return to string-content scanning.
    /// Without this field the close path unconditionally flipped to
    /// STATE_DQ_RESUME, which made every line after the first top-level
    /// `j=$(( … ))` / `x=$(cmd)` render as a single DQ string segment
    /// (the rest of the file dimmed-out as STRING_DQ color).
    private var dqInterpResumeState = STATE_NORMAL
    /// When STATE_DQ_FORMAT / STATE_BT_FORMAT is set, this is the
    /// pre-computed byte length of the printf format spec starting
    /// at the current pos. The next advance() emits exactly that
    /// many chars as STRING_FORMAT, then flips back to the
    /// matching string-resume state.
    private var pendingFormatLen = 0

    override fun start(buffer: CharSequence, startOffset: Int, endOffset: Int, initialState: Int) {
        buf = buffer
        this.endOffset = endOffset
        pos = startOffset
        state = initialState
        // Reset transient counters that DON'T survive lexer restarts.
        // `paramBraceDepth` tracks within-document `${…}` nesting and
        // must start at 0 — non-zero from a prior buffer would mis-
        // classify the first `}` we see as a param-close.
        paramBraceDepth = 0
        pendingStringResumeState = STATE_NORMAL
        dqInterpParenDepth = 0
        dqInterpResumeState = STATE_NORMAL
        advance()
    }

    override fun getState(): Int = state
    override fun getTokenType(): IElementType? = tokenType
    override fun getTokenStart(): Int = tokenStart
    override fun getTokenEnd(): Int = tokenEnd
    override fun getBufferSequence(): CharSequence = buf
    override fun getBufferEnd(): Int = endOffset

    override fun advance() {
        tokenStart = pos
        if (pos >= endOffset) {
            tokenType = null
            tokenEnd = pos
            return
        }
        // State-driven re-entry for interpolating strings. Set by
        // `consumeInterpolatingString` when it hits a `$var` / `${...}`
        // / `$(...)` boundary inside `"..."` or `` `...` ``: emit the
        // literal-prefix segment, return, then handle the interpolation
        // on the NEXT advance(). After the interpolation, swap to a
        // RESUME state so the following advance() picks up scanning the
        // tail of the string (without re-consuming an opening quote).
        when (state) {
            STATE_DQ_INTERP -> { consumeStringInterpolation('"'); return }
            STATE_DQ_RESUME -> { state = STATE_NORMAL; consumeStringContent('"', ZshrsTokenTypes.STRING_DQ); return }
            STATE_BT_INTERP -> { consumeStringInterpolation('`'); return }
            STATE_BT_RESUME -> { state = STATE_NORMAL; consumeStringContent('`', ZshrsTokenTypes.BACKTICK); return }
            STATE_DQ_ARITH -> { lexInsideDqArith(); return }
            STATE_DQ_CMDSUB -> { lexInsideDqCmdsub(); return }
            // Pre-computed printf format spec — emit the cached span
            // as ONE STRING_FORMAT token, then flip back to the
            // corresponding string-resume state so the next advance()
            // picks up scanning the rest of the string body.
            STATE_DQ_FORMAT -> {
                tokenStart = pos
                tokenEnd = pos + pendingFormatLen
                pos = tokenEnd
                tokenType = ZshrsTokenTypes.STRING_FORMAT
                pendingFormatLen = 0
                state = STATE_DQ_RESUME
                return
            }
            STATE_BT_FORMAT -> {
                tokenStart = pos
                tokenEnd = pos + pendingFormatLen
                pos = tokenEnd
                tokenType = ZshrsTokenTypes.STRING_FORMAT
                pendingFormatLen = 0
                state = STATE_BT_RESUME
                return
            }
        }
        val c = buf[pos]
        when {
            // Shebang first line takes priority over plain comment
            c == '#' && pos == 0 && peek(1) == '!' -> consumeShebang()
            // `#` inside an open `${…}` is the parameter-length
            // operator (`${#var}`) or a pattern-anchor (`${var/#pat/repl}`),
            // NOT a line comment. Only treat `#` as a comment-start
            // when we're at the OUTER lexer level (paramBraceDepth==0).
            c == '#' && paramBraceDepth == 0 -> consumeLineComment()
            // Inside `${…}`, emit `#` as part of the param-expansion
            // segment so it gets the param color (not BAD_CHARACTER
            // from the catch-all below). Covers `${#var}` length-op,
            // `${var/#pat/repl}` pattern-anchor, etc.
            c == '#' -> emit(1, ZshrsTokenTypes.PARAM_EXPANSION)
            c == '\n' || c == '\r' || c == ' ' || c == '\t' -> consumeWhitespace()
            c == '"' -> consumeInterpolatingString('"', ZshrsTokenTypes.STRING_DQ)
            c == '\'' -> consumeString('\'', ZshrsTokenTypes.STRING_SQ)
            c == '`' -> consumeInterpolatingString('`', ZshrsTokenTypes.BACKTICK)
            c == '$' && peek(1) == '\'' -> { pos++; consumeString('\'', ZshrsTokenTypes.STRING_DOLLAR) }
            c == '<' && peek(1) == '<' && peek(2) != '<' && isHeredocStart(2) -> consumeHeredoc()
            c == '<' && peek(1) == '<' && peek(2) == '<' -> emit(3, ZshrsTokenTypes.REDIRECT) // here-string
            c == '<' && peek(1) == '<' -> emit(2, ZshrsTokenTypes.REDIRECT)
            c == '>' && peek(1) == '>' -> emit(2, ZshrsTokenTypes.REDIRECT)
            c == '<' || c == '>' -> consumeRedirect(c)
            c == '&' && peek(1) == '&' -> emit(2, ZshrsTokenTypes.LOGICAL_OP)
            c == '|' && peek(1) == '|' -> emit(2, ZshrsTokenTypes.LOGICAL_OP)
            c == '|' && peek(1) == '&' -> emit(2, ZshrsTokenTypes.PIPE)
            c == '|' -> emit(1, ZshrsTokenTypes.PIPE)
            c == '&' && peek(1) == '>' -> emit(2, ZshrsTokenTypes.REDIRECT)
            c == '&' -> emit(1, ZshrsTokenTypes.BACKGROUND)
            c == '$' -> consumeDollar()
            c == '\\' -> consumeEscape()
            c == ';' && peek(1) == ';' -> {
                // `;;`, `;;&`, `;|` — all case-branch terminators
                var p = pos + 2
                if (p < endOffset && (buf[p] == '&' || buf[p] == '|')) p++
                tokenEnd = p; pos = p
                tokenType = ZshrsTokenTypes.DOUBLE_SEMI
            }
            c == ';' && peek(1) == '|' -> emit(2, ZshrsTokenTypes.DOUBLE_SEMI)
            c == '(' -> emit(1, ZshrsTokenTypes.LPAREN)
            c == ')' -> emit(1, ZshrsTokenTypes.RPAREN)
            c == '{' && paramBraceDepth > 0 -> {
                // Literal `{` inside a `${…}` default-value (e.g.
                // `${1:-{\}}` whose default is the literal `{}`).
                // Emit as PARAM_EXPANSION so the brace matcher
                // doesn't pair it with the surrounding function's
                // closing `}` — the regression was: function-close
                // `}` highlighted as the partner of this literal `{`,
                // and the visible `}` on the next line looked
                // unmatched / wrong-colored.
                emit(1, ZshrsTokenTypes.PARAM_EXPANSION)
            }
            c == '{' -> emit(1, ZshrsTokenTypes.LBRACE)
            c == '}' && paramBraceDepth > 0 -> {
                // Close marker for a `${…}` we previously opened —
                // emit as PARAM_EXPANSION so it pairs visually with
                // its `${` opener. Decrement the depth tracker so a
                // subsequent literal `}` (command-group close)
                // routes back through the bare-RBRACE branch.
                paramBraceDepth--
                emit(1, ZshrsTokenTypes.PARAM_EXPANSION)
                // If this close ends the OUTERMOST `${…}` AND we
                // entered it from a string-interpolation context,
                // restore the string-resume state so the next
                // advance() picks up the rest of the surrounding
                // `"…"` / `` `…` ``.
                if (paramBraceDepth == 0 && pendingStringResumeState != STATE_NORMAL) {
                    state = pendingStringResumeState
                    pendingStringResumeState = STATE_NORMAL
                }
            }
            c == '}' -> emit(1, ZshrsTokenTypes.RBRACE)
            c == '[' -> emit(1, ZshrsTokenTypes.LBRACKET)
            c == ']' -> emit(1, ZshrsTokenTypes.RBRACKET)
            c == ',' -> emit(1, ZshrsTokenTypes.COMMA)
            c == ';' -> emit(1, ZshrsTokenTypes.SEMICOLON)
            c == '*' || c == '?' -> emit(1, ZshrsTokenTypes.GLOB)
            c == '=' && (peek(1) == '~' || peek(1) == '=') -> emit(2, ZshrsTokenTypes.OPERATOR)
            c == '=' -> emit(1, ZshrsTokenTypes.ASSIGN_OP)
            c == '+' && peek(1) == '=' -> emit(2, ZshrsTokenTypes.ASSIGN_OP)
            c == '-' && peek(1) == '=' -> emit(2, ZshrsTokenTypes.ASSIGN_OP)
            c == ':' && peek(1) == '=' -> emit(2, ZshrsTokenTypes.ASSIGN_OP)
            c == '?' && peek(1) == '=' -> emit(2, ZshrsTokenTypes.ASSIGN_OP)
            c.isDigit() -> consumeNumber()
            c == '_' || c.isLetter() -> consumeWord()
            // zsh permits leading `.` / `+` / `@` in function / command
            // names — zinit liberally uses `.zinit-foo` / `+vi-…` /
            // `@hook-fn` for its private functions. Treat them as
            // identifier-start when the next char is a word char so
            // `.zinit-load` lexes as one IDENTIFIER instead of
            // OPERATOR + IDENTIFIER + … (which made `do`/`load`/etc.
            // mid-name get mis-highlighted as keywords).
            // zsh permits `.zinit-foo` / `+vi-mode` / `@hook-fn` / `:foo`
            // / `^foo` / `-hsmw-…` as identifier-start. Per
            // `Src/utils.c::inittyptab`, `.` and `-` are explicitly in
            // IUSER; function names accept anything that isn't a
            // metachar. Audited against ~/.zinit/plugins/** — the set
            // here covers zsh-syntax-highlighting (`-hsmw-…`), zui
            // (`-zui_std_…`), zinit (`.zinit-…`), p10k (`+vi-mode-…`),
            // async hooks (`@hook-fn`), and OO-style (`:hist:…`).
            //
            // Trade-off for leading `-`: `cmd -opt arg` now lexes as
            // IDENTIFIER+IDENTIFIER instead of IDENTIFIER+OPERATOR+IDENT.
            // That's actually closer to how shell treats the option —
            // it IS a single word at the parser level.
            (c == '.' || c == '+' || c == '@' || c == ':' || c == '^' || c == '-') &&
                peek(1).let { it == '_' || it.isLetter() } ->
                consumeWord()
            // Long flag (`--word`) — lex as a single IDENTIFIER
            // token so it doesn't render as two consecutive OPERATOR
            // dashes (visually broken / often colored as BAD_CHARACTER).
            // The third char must be a word char so we don't mis-eat
            // `--` separators that introduce a positional-args block.
            c == '-' && peek(1) == '-' && peek(2).let { it == '_' || it.isLetter() } ->
                consumeLongFlag()
            isOperatorChar(c) -> emit(1, ZshrsTokenTypes.OPERATOR)
            else -> emit(1, TokenType.BAD_CHARACTER)
        }
    }

    private fun peek(off: Int): Char = if (pos + off < endOffset) buf[pos + off] else ' '

    private fun emit(len: Int, tt: IElementType) {
        tokenEnd = (pos + len).coerceAtMost(endOffset)
        pos = tokenEnd
        tokenType = tt
    }

    private fun consumeShebang() {
        var p = pos
        while (p < endOffset && buf[p] != '\n') p++
        tokenEnd = p; pos = p
        tokenType = ZshrsTokenTypes.SHEBANG
    }

    private fun consumeLineComment() {
        var p = pos
        while (p < endOffset && buf[p] != '\n') p++
        tokenEnd = p; pos = p
        // `##` doubled-hash → DOC_COMMENT (separate color slot,
        // matches the LSP's docstring convention for module /
        // function / variable docs). `#` alone → regular COMMENT.
        // Shebang `#!` has its own consumer earlier so it doesn't
        // reach here.
        val len = p - tokenStart
        tokenType = if (len >= 2 && buf[tokenStart + 1] == '#') {
            ZshrsTokenTypes.DOC_COMMENT
        } else {
            ZshrsTokenTypes.COMMENT
        }
    }

    private fun consumeWhitespace() {
        var p = pos
        while (p < endOffset && (buf[p] == ' ' || buf[p] == '\t' || buf[p] == '\n' || buf[p] == '\r')) p++
        tokenEnd = p; pos = p
        tokenType = TokenType.WHITE_SPACE
    }

    /// Open + consume a `"..."` or `` `...` `` string with `$var` /
    /// `${...}` / `$(...)` interpolation splitting. Always emits the
    /// opening quote as its OWN string token (a discontinuous token
    /// stream — gaps between tokens — is a contract violation that
    /// makes IntelliJ's searchable-options indexer crash); the body
    /// scan runs on the next advance() via the RESUME state.
    /// Splitting the string into [STRING] [VARIABLE] [STRING] is what
    /// lets the IDE offer completion on the var.
    private fun consumeInterpolatingString(quote: Char, tt: IElementType) {
        pos++  // past the opening quote
        tokenEnd = pos
        tokenType = tt
        state = if (quote == '"') STATE_DQ_RESUME else STATE_BT_RESUME
    }

    /// Scan the body of a `"..."` / `` `...` `` string starting at the
    /// CURRENT pos (no leading quote to skip). Emits one of:
    ///   * The entire run up to the closing quote, INCLUDING the close,
    ///     and clears state.
    ///   * The literal prefix up to (but not including) a `$`-interp
    ///     marker, sets `STATE_*_INTERP` so the next advance() handles
    ///     the interpolation.
    private fun consumeStringContent(quote: Char, tt: IElementType) {
        var p = pos
        while (p < endOffset) {
            val c = buf[p]
            if (c == '\\' && p + 1 < endOffset) { p += 2; continue }
            if (c == quote) {
                p++  // include the closing quote in the literal token
                tokenEnd = p; pos = p
                state = STATE_NORMAL
                tokenType = tt
                return
            }
            if (c == '$' && p + 1 < endOffset && isInterpStart(buf, p + 1)) {
                if (p > pos) {
                    tokenEnd = p; pos = p
                    state = if (quote == '"') STATE_DQ_INTERP else STATE_BT_INTERP
                    tokenType = tt
                    return
                }
                consumeStringInterpolation(quote)
                return
            }
            // printf format specifier `%d` / `%10.2f` / `%-15s` /
            // `%%` etc. Mirror of strykelang's STRING_FORMAT
            // handling: emit the literal prefix as STRING first, set
            // the FORMAT state so the next advance() emits the
            // format-spec span as STRING_FORMAT, then resumes
            // string-content scanning.
            if (c == '%') {
                val flen = printfFormatLen(p)
                if (flen > 0) {
                    if (p > pos) {
                        tokenEnd = p; pos = p
                        state = if (quote == '"') STATE_DQ_FORMAT else STATE_BT_FORMAT
                        pendingFormatLen = flen
                        tokenType = tt
                        return
                    }
                    // Empty literal prefix — emit the format spec
                    // directly and flip to resume state.
                    tokenStart = p
                    tokenEnd = p + flen
                    pos = tokenEnd
                    tokenType = ZshrsTokenTypes.STRING_FORMAT
                    state = if (quote == '"') STATE_DQ_RESUME else STATE_BT_RESUME
                    return
                }
            }
            p++
        }
        // Unterminated string — emit whatever's left as the literal,
        // clear state so the outer lexer recovers cleanly.
        tokenEnd = p; pos = p
        state = STATE_NORMAL
        tokenType = tt
    }

    /// Length of the printf format spec starting at `start` (which
    /// must point at `%`), or 0 if the chars after `%` don't form a
    /// valid spec. Grammar:
    ///   `%` [flags `-+0 #'`]* [width digits]?
    ///       (`.` [precision digits]+)?
    ///       [length `l`/`h`/`L`/`q`/`z`/`j`/`t`]?
    ///       conversion `diouxXeEfgGsScCpnb%`
    /// `%%` is a literal-percent — also returned as length 2 so it
    /// gets the format-spec color (visually distinct from prose `%`).
    private fun printfFormatLen(start: Int): Int {
        if (start >= endOffset || buf[start] != '%') return 0
        var p = start + 1
        if (p >= endOffset) return 0
        if (buf[p] == '%') return 2
        while (p < endOffset && buf[p] in FORMAT_FLAGS) p++
        while (p < endOffset && buf[p].isDigit()) p++
        if (p < endOffset && buf[p] == '.') {
            p++
            while (p < endOffset && buf[p].isDigit()) p++
        }
        while (p < endOffset && buf[p] in FORMAT_LENGTH) p++
        if (p < endOffset && buf[p] in FORMAT_CONV) {
            return (p - start) + 1
        }
        return 0
    }

    /// Handle one `$`-interpolation inside an open `"..."` / `` `...` ``
    /// string. Emits the var / param-expansion / cmd-subst as its own
    /// token (VARIABLE, PARAM_EXPANSION, etc. — same token types the
    /// top-level lexer uses), then sets the RESUME state so the next
    /// advance() picks up scanning the rest of the string.
    private fun consumeStringInterpolation(quote: Char) {
        val resume = if (quote == '"') STATE_DQ_RESUME else STATE_BT_RESUME
        // Reset `tokenStart` — see consumeStringInterpolation docs above.
        tokenStart = pos
        val depthBefore = paramBraceDepth
        consumeDollar()
        // If consumeDollar opened a `${…}` (depth went up), DON'T
        // resume string mode yet — the body needs normal-lexer
        // tokenization. Save the resume target so the matching `}`
        // pops us back into string-content mode.
        if (paramBraceDepth > depthBefore) {
            pendingStringResumeState = resume
            state = STATE_NORMAL
        } else if (state == STATE_DQ_ARITH || state == STATE_DQ_CMDSUB) {
            // consumeDollar entered a `$((arith))` / `$(cmd)` sub-lex
            // block — keep its state (the sub-lex handlers know to
            // flip back via dqInterpResumeState on the matching `))` /
            // `)`). consumeDollar defaulted dqInterpResumeState to
            // STATE_NORMAL (top-level case); override to the in-DQ
            // resume state so the close returns into string-content
            // mode instead of falling out of the surrounding string.
            // Without this skip + override, the `state = resume` line
            // below would overwrite the sub-lex state and the body
            // would be re-absorbed into STRING_DQ on the next advance().
            dqInterpResumeState = resume
        } else {
            state = resume
        }
    }

    /// One `advance()` while we're inside the body of a `$((arith))`
    /// interpolation that opened in a `"..."` string. Closes on the
    /// matching `))` at paren depth 0 and flips back to STATE_DQ_RESUME
    /// so the rest of the string lexes normally. Otherwise dispatches
    /// one normal-state lex (numbers / operators / identifiers each
    /// emitted as their own token, with their own color in the IDE)
    /// while tracking paren depth via `dqInterpParenDepth` so a
    /// nested `(…)` inside the body doesn't close the outer arith
    /// prematurely.
    ///
    /// Modeled on strykelang's `lexInsideInterpolation`
    /// (editors/intellij/.../stryke/StrykeLexer.kt:432-477).
    private fun lexInsideDqArith() {
        // Closing `))` at depth 0 — emit as one OPERATOR and return
        // to string mode.
        if (pos + 1 < endOffset
            && buf[pos] == ')'
            && buf[pos + 1] == ')'
            && dqInterpParenDepth == 0
        ) {
            tokenStart = pos
            tokenEnd = pos + 2
            pos = tokenEnd
            tokenType = ZshrsTokenTypes.OPERATOR
            // Resume in-DQ string content if the opener was inside a
            // `"..."`, otherwise drop back to the top-level dispatcher.
            // Without picking the right state here, top-level `$((expr))`
            // (e.g. `j=$(( RANDOM % i + 1 ))`) left the lexer in DQ_RESUME
            // and the entire rest of the file rendered as STRING_DQ.
            state = dqInterpResumeState
            return
        }
        // Otherwise run the normal dispatcher once; track depth so
        // nested `(` / `)` inside the body don't end the arith block
        // early. Saved/restored entry state so we can return to it.
        val savedState = state
        state = STATE_NORMAL
        // Run one normal advance — re-enters this fn via the dispatch
        // at the top of advance() but state is now STATE_NORMAL so the
        // normal lex path executes. `advance()` sets tokenStart/
        // tokenEnd to the new token's actual span — DO NOT restore an
        // older tokenStart after this point or IntelliJ will see the
        // inner token as starting where the `$((` opener started and
        // re-color the whole range as STRING. Previous revision saved
        // and restored tokenStart which broke `$((expr))` syntax
        // highlighting inside `"..."`.
        advance()
        // Track paren depth on the emitted token (might span multiple
        // chars but only `(` / `)` single-char tokens affect depth).
        if (tokenEnd == tokenStart + 1) {
            when (buf[tokenStart]) {
                '(' -> dqInterpParenDepth++
                ')' -> dqInterpParenDepth--
            }
        }
        // Stay in arith mode for the next advance().
        state = savedState  // STATE_DQ_ARITH
    }

    /// One `advance()` while inside the body of a `$(cmd)` command
    /// substitution that opened in a `"..."` string. Closes on the
    /// matching `)` at paren depth 0. Mirrors [lexInsideDqArith] but
    /// the closer is a single `)` instead of `))`.
    private fun lexInsideDqCmdsub() {
        // Closing `)` at depth 0 — emit as one OPERATOR and return.
        if (pos < endOffset && buf[pos] == ')' && dqInterpParenDepth == 1) {
            tokenStart = pos
            tokenEnd = pos + 1
            pos = tokenEnd
            tokenType = ZshrsTokenTypes.OPERATOR
            // Resume in-DQ string content if the opener was inside a
            // `"..."`, otherwise drop back to the top-level dispatcher.
            // See dqInterpResumeState docs at field declaration —
            // top-level `x=$(cmd)` was leaving lexer in DQ_RESUME and
            // the entire rest of the file rendered as STRING_DQ.
            state = dqInterpResumeState
            dqInterpParenDepth = 0
            return
        }
        val savedState = state
        state = STATE_NORMAL
        // Same caveat as lexInsideDqArith above — do NOT restore
        // tokenStart after `advance()`; the dispatcher set it to the
        // new token's actual start. Restoring an older value makes
        // IntelliJ see the inner identifier/number/operator span as
        // starting at the `$(` opener and the IDE re-colors the whole
        // range as STRING, so `$(classify 200)` inside `"..."` had
        // `classify 200` rendered with string color instead of
        // command/number colors.
        advance()
        if (tokenEnd == tokenStart + 1) {
            when (buf[tokenStart]) {
                '(' -> dqInterpParenDepth++
                ')' -> dqInterpParenDepth--
            }
        }
        state = savedState  // STATE_DQ_CMDSUB
    }

    private fun consumeString(quote: Char, tt: IElementType) {
        var p = pos + 1
        while (p < endOffset) {
            val c = buf[p]
            if (quote == '\'') {
                // Single-quoted strings in zsh have no escapes (`$'...'` we treat the same here)
                if (c == quote) { p++; break }
            } else {
                if (c == '\\' && p + 1 < endOffset) { p += 2; continue }
                if (c == quote) { p++; break }
            }
            p++
        }
        tokenEnd = p; pos = p
        tokenType = tt
    }

    private fun isHeredocStart(off: Int): Boolean {
        var p = pos + off
        if (p < endOffset && buf[p] == '-') p++
        if (p < endOffset && (buf[p] == '\'' || buf[p] == '"')) p++
        return p < endOffset && (buf[p] == '_' || buf[p].isLetter())
    }

    private fun consumeHeredoc() {
        // `<<EOT`, `<<-EOT`, `<<'EOT'`, `<<"EOT"`
        var p = pos + 2
        if (p < endOffset && buf[p] == '-') p++
        val quote = if (p < endOffset && (buf[p] == '\'' || buf[p] == '"')) {
            val q = buf[p]; p++; q
        } else 0.toChar()
        val nameStart = p
        while (p < endOffset && (buf[p] == '_' || buf[p].isLetterOrDigit())) p++
        val name = buf.subSequence(nameStart, p).toString()
        if (quote != 0.toChar() && p < endOffset && buf[p] == quote) p++
        while (p < endOffset) {
            while (p < endOffset && buf[p] != '\n') p++
            if (p >= endOffset) break
            p++ // consume newline
            val lineStart = p
            var q = lineStart
            while (q < endOffset && (buf[q] == ' ' || buf[q] == '\t')) q++
            val termEnd = q + name.length
            if (termEnd <= endOffset &&
                buf.subSequence(q, termEnd).toString() == name &&
                (termEnd == endOffset || buf[termEnd] == '\n' || buf[termEnd] == '\r')
            ) {
                p = termEnd
                break
            }
        }
        tokenEnd = p; pos = p
        tokenType = ZshrsTokenTypes.HEREDOC
    }

    private fun consumeRedirect(start: Char) {
        // Already at `<` or `>`. Forms: `>`, `>>`, `<`, `<<` (handled), `<<<` (handled),
        // `&>`, `>&`, `>|`, `2>&1`, `<>`. We try to be greedy.
        var p = pos + 1
        if (p < endOffset && (buf[p] == '&' || buf[p] == '|')) {
            p++
            // optional fd after `>&`
            while (p < endOffset && buf[p].isDigit()) p++
        }
        tokenEnd = p; pos = p
        tokenType = ZshrsTokenTypes.REDIRECT
    }

    private fun consumeEscape() {
        // `\<newline>` is a shell line continuation — the two-char sequence
        // is logically whitespace (the parser joins the lines). Optional
        // `\r` before `\n` covers CRLF files. `\X` for any other char is a
        // literal escape (e.g. `\$`, `\"`, `\\`); emit as STRING_ESCAPE so
        // it isn't flagged BAD_CHARACTER. Bare trailing `\` at EOF is also
        // accepted (one-char escape) to avoid red squiggles on partial input.
        val nxt = peek(1)
        if (nxt == '\n' || nxt == '\r') {
            var p = pos + 1
            if (p < endOffset && buf[p] == '\r') p++
            if (p < endOffset && buf[p] == '\n') p++
            tokenEnd = p; pos = p
            tokenType = TokenType.WHITE_SPACE
            return
        }
        val len = if (pos + 1 < endOffset) 2 else 1
        emit(len, ZshrsTokenTypes.STRING_ESCAPE)
    }

    private fun consumeDollar() {
        val p0 = pos
        var p = pos + 1
        if (p >= endOffset) {
            tokenEnd = p; pos = p
            tokenType = ZshrsTokenTypes.SIGIL
            return
        }
        val nxt = buf[p]
        // ${...} parameter expansion — emit the leading `${` as
        // PARAM_EXPANSION (covers `${` opener), then re-enter the
        // outer lexer so the variable name inside lexes as its own
        // VARIABLE / IDENTIFIER token. The IDE can offer completion
        // on that token (Cmd-Space inside `${name|}` replaces the
        // `name` segment cleanly instead of overwriting the entire
        // expansion). The closing `}` and any modifiers (`:-default`,
        // `:offset:length`, `/pat/replace`, `(flags)`, etc.) flow
        // through the normal token stream until the matching close.
        //
        // The full multi-token rendering is enabled by setting a
        // STATE_IN_PARAM_BODY marker so the close `}` knows to emit
        // PARAM_EXPANSION_CLOSE rather than the bare LBRACE/RBRACE
        // it would otherwise. Brace depth tracked at the outer level
        // so the `}` that closes us doesn't get confused with a
        // command-group `}`.
        //
        // Pre-flight: verify the param expansion actually closes
        // before we commit to segment mode. Without this, an unclosed
        // `${` would leave the lexer in body mode forever (file-level
        // OOM cascade). The pre-scan uses the same brace-counting
        // rules as the old opaque path: `${` nests, bare `{` is
        // literal, `\}` is escaped, FIRST `}` at depth 0 closes.
        if (nxt == '{') {
            // pos at `$`. Pre-scan to find matching `}`.
            var pp = p + 1  // past `{`
            var depth = 1
            while (pp < endOffset && depth > 0) {
                val cc = buf[pp]
                if (cc == '\\' && pp + 1 < endOffset) { pp += 2; continue }
                if (cc == '$' && pp + 1 < endOffset && buf[pp + 1] == '{') {
                    depth++; pp += 2; continue
                }
                if (cc == '}') {
                    depth--; pp++
                    if (depth == 0) break
                    continue
                }
                pp++
            }
            if (depth != 0) {
                // Unclosed — fall back to opaque-token mode so the
                // rest of the file still lexes (rather than leaving
                // the lexer in body mode forever).
                tokenEnd = pp; pos = pp
                tokenType = ZshrsTokenTypes.PARAM_EXPANSION
                return
            }
            // Emit `${` as the opener, set state so the matching `}`
            // gets recognized as PARAM_EXPANSION_CLOSE, and let the
            // outer lexer take over for the body.
            paramBraceDepth++
            emit(2, ZshrsTokenTypes.PARAM_EXPANSION)
            return
        }
        // $((arith)) / $(cmd) — open a sub-lex block so the interior
        // re-tokenizes as full shell code (numbers / operators /
        // identifiers each with their own color). Mirrors strykelang's
        // `lexInsideInterpolation` pattern at
        // editors/intellij/.../stryke/StrykeLexer.kt:432-477 — flip to
        // a sub-state, emit the opener as OPERATOR, and on each
        // subsequent `advance()` recursively dispatch normal lex while
        // tracking paren depth so nested `(` / `)` don't close early.
        // The matching `))` / `)` flips us back to STATE_DQ_RESUME.
        //
        // Without this the user saw the entire `((arith))` body
        // re-absorbed into the next STRING-literal segment, so the
        // arithmetic interior inherited italic-green string color.
        if (nxt == '(') {
            // Decide where to flip back to after the matching `))` / `)`:
            // if we ARE already inside a DQ string (state==STATE_NORMAL
            // here while consumeStringInterpolation -> consumeDollar is in
            // progress is the case — strykelang/zsh use a sub-lex mode
            // for that), the wrapper at consumeStringInterpolation will
            // pin DQ resume regardless. The check below captures the
            // top-level case: when consumeDollar was called from the
            // OUTER advance dispatcher (line 155 `c == '$' ->
            // consumeDollar()`), nothing is wrapping us — the resume
            // state after close must be STATE_NORMAL, NOT STATE_DQ_RESUME.
            //
            // We default to STATE_DQ_RESUME and let consumeStringInterpolation
            // overwrite for in-DQ; here we just set the default based on
            // whether we entered from outside a string. The OUTER caller
            // (consumeStringInterpolation) sees the state we set and
            // doesn't touch dqInterpResumeState — it owns the in-DQ case
            // via its `else if (state == STATE_DQ_ARITH || ...)` arm.
            //
            // Heuristic: any call that arrived here via the outer
            // `c == '$' -> consumeDollar()` dispatch is at top level —
            // there is no surrounding interpolating-string scope so the
            // post-close resume must be STATE_NORMAL. The in-DQ path
            // overrides this by assigning STATE_DQ_RESUME below at
            // [consumeStringInterpolation]'s `else if (state == STATE_DQ_ARITH || …)`.
            dqInterpResumeState = STATE_NORMAL
            val isArith = p + 1 < endOffset && buf[p + 1] == '('
            if (isArith) {
                // Emit `$((` as OPERATOR (3 bytes). Next advance()
                // dispatches into lexInsideDqArith via STATE_DQ_ARITH.
                tokenEnd = p + 2  // p was at `(`, +2 covers `$((`
                pos = tokenEnd
                tokenType = ZshrsTokenTypes.OPERATOR
                state = STATE_DQ_ARITH
                dqInterpParenDepth = 0  // count `(` past the `$((` opener
                return
            }
            // Single-paren `$(cmd)` → command substitution.
            // Emit `$(` as OPERATOR (2 bytes) and flip to cmdsub mode.
            tokenEnd = p + 1  // covers `$(`
            pos = tokenEnd
            tokenType = ZshrsTokenTypes.OPERATOR
            state = STATE_DQ_CMDSUB
            dqInterpParenDepth = 1  // we're 1 level past the `$( opener
            return
        }
        // Special single-char variables: $0..$9, $?, $!, $$, $#, $*, $@, $-, $_
        if (nxt.isDigit() || nxt in "?!$#*@-_") {
            p++
            // numeric positional args can be multi-digit only inside ${...}; bare $10 == ${1}0 in POSIX
            tokenEnd = p; pos = p
            tokenType = ZshrsTokenTypes.SPECIAL_VAR
            return
        }
        // Regular identifier
        if (nxt == '_' || nxt.isLetter()) {
            while (p < endOffset && (buf[p] == '_' || buf[p].isLetterOrDigit())) p++
            tokenEnd = p; pos = p
            // Classify ALL_CAPS as env var
            val name = buf.subSequence(p0 + 1, p).toString()
            tokenType = if (looksLikeEnvVar(name)) ZshrsTokenTypes.ENV_VAR else ZshrsTokenTypes.VARIABLE
            return
        }
        tokenEnd = p; pos = p
        tokenType = ZshrsTokenTypes.SIGIL
    }

    private fun looksLikeEnvVar(name: String): Boolean {
        if (name.isEmpty()) return false
        for (c in name) {
            if (!(c.isUpperCase() || c.isDigit() || c == '_')) return false
        }
        return name.any { it.isUpperCase() }
    }

    private fun consumeNumber() {
        var p = pos
        while (p < endOffset && (buf[p].isDigit() || buf[p] == '_')) p++
        if (p < endOffset && buf[p] == '.' && p + 1 < endOffset && buf[p + 1].isDigit()) {
            p++
            while (p < endOffset && buf[p].isDigit()) p++
        }
        if (p < endOffset && (buf[p] == 'e' || buf[p] == 'E')) {
            p++
            if (p < endOffset && (buf[p] == '+' || buf[p] == '-')) p++
            while (p < endOffset && buf[p].isDigit()) p++
        }
        tokenEnd = p; pos = p
        tokenType = ZshrsTokenTypes.NUMBER
    }

    /// Consume `--<word>` as a single IDENTIFIER token. Long-flag
    /// arguments to `zshrs` / external commands rendered as two
    /// individual `-` OPERATOR tokens before this branch existed,
    /// which often surfaced as red BAD_CHARACTER coloring.
    private fun consumeLongFlag() {
        var p = pos + 2 // skip `--`
        while (p < endOffset) {
            val c = buf[p]
            if (c.isLetterOrDigit() || c == '_' || c == '-' || c == '.') p++ else break
        }
        tokenEnd = p; pos = p
        tokenType = ZshrsTokenTypes.IDENTIFIER
    }

    private fun consumeWord() {
        var p = pos
        // Leading zsh-extended sigils: `.zinit-foo` / `+vi-mode` /
        // `@hook-fn`. The dispatcher in `advance()` only routes here
        // when the next char is a word char, so consuming the sigil
        // unconditionally here advances past it without re-checking.
        // (Skipping this step would leave `p == pos` and the outer
        // `advance()` loop would re-fire on the same byte → OOM.)
        if (p < endOffset) {
            val c0 = buf[p]
            if ((c0 == '.' || c0 == '+' || c0 == '@' || c0 == ':' || c0 == '^' || c0 == '-')
                && p + 1 < endOffset
                && (buf[p + 1] == '_' || buf[p + 1].isLetter())
            ) {
                p++
            }
        }
        while (p < endOffset) {
            val c = buf[p]
            if (c == '_' || c.isLetterOrDigit()) {
                p++
                continue
            }
            // Internal connector chars from `Src/utils.c::IUSER` — the
            // ones zsh allows mid-name. `-` is the common case
            // (`daemon-lock-do`, `daemon-export-pdf`); `.` covers
            // `_dotdrop.sh-compare` / `_polca.sh`; `:` covers
            // `:hist:precmd`. Only swallow when followed by another
            // word char so trailing `-` / `.` (option separator,
            // end-of-statement dot) stays a distinct token.
            if ((c == '-' || c == '.' || c == ':') && p + 1 < endOffset) {
                val nxt = buf[p + 1]
                if (nxt == '_' || nxt.isLetterOrDigit()) {
                    p++
                    continue
                }
            }
            break
        }
        val word = buf.subSequence(pos, p).toString()
        tokenEnd = p; pos = p
        tokenType = classifyWord(word)
    }

    private fun classifyWord(word: String): IElementType = when (word) {
        in CONTROL_KEYWORDS -> ZshrsTokenTypes.CONTROL_KEYWORD
        in DECL_KEYWORDS -> ZshrsTokenTypes.DECL_KEYWORD
        in FN_KEYWORDS -> ZshrsTokenTypes.FN_KEYWORD
        in LOOP_KEYWORDS -> ZshrsTokenTypes.LOOP_KEYWORD
        in MODIFIER_KEYWORDS -> ZshrsTokenTypes.MODIFIER_KEYWORD
        in IO_KEYWORDS -> ZshrsTokenTypes.IO_KEYWORD
        in BUILTINS -> ZshrsTokenTypes.BUILTIN
        else -> {
            if (word == word.uppercase() && word.length > 1 && word.any { it.isUpperCase() }) {
                ZshrsTokenTypes.OPTION_NAME
            } else {
                ZshrsTokenTypes.IDENTIFIER
            }
        }
    }

    private fun isOperatorChar(c: Char): Boolean = c in "+-*/%!~^:.@"

    companion object {
        // Lexer states for the interpolating-string state machine.
        // STATE_NORMAL is the default; the *_INTERP / *_RESUME pairs
        // bracket each `$var` / `${...}` / `$(...)` interpolation inside
        // a `"..."` or `` `...` `` string, breaking the string into
        // [STRING] [VAR] [STRING] segments so the IDE can offer
        // completion + variable-coloring on the embedded `$var`.
        const val STATE_NORMAL = 0
        const val STATE_DQ_INTERP = 1     // about to lex `$…` inside "..."
        const val STATE_DQ_RESUME = 2     // just lexed `$…`; resume "..." content
        const val STATE_BT_INTERP = 3     // about to lex `$…` inside `...`
        const val STATE_BT_RESUME = 4     // just lexed `$…`; resume `...` content
        const val STATE_DQ_FORMAT = 5     // about to emit one `%d`/`%s`/etc format-spec token inside "..."
        const val STATE_BT_FORMAT = 6     // same inside `...` backtick
        /// Inside the body of a `$((arith))` interpolation that opened
        /// while we were lexing a `"..."` double-quoted string. Each
        /// `advance()` runs one normal-lex dispatch (numbers / operators
        /// / identifiers each as separate tokens with their own color)
        /// until the closing `))` flips back to STATE_DQ_RESUME so the
        /// rest of the string lexes normally. Modeled on strykelang's
        /// STATE_IN_DQ_INTERP. Paren depth tracked via
        /// `dqInterpParenDepth`.
        const val STATE_DQ_ARITH = 7
        /// Same as STATE_DQ_ARITH but for `$(cmd)` command-substitution
        /// (one level of parens instead of two). On the matching `)`
        /// at depth 0 we flip back to STATE_DQ_RESUME.
        const val STATE_DQ_CMDSUB = 8

        // printf format-spec character classes. Mirrors
        // strykelang's FORMAT_FLAGS / FORMAT_LENGTH / FORMAT_CONV.
        // Conversion chars cover the POSIX set plus zsh's
        // `printf %b` (backslash-escape interpretation).
        private val FORMAT_FLAGS = setOf('-', '+', '0', ' ', '#', '\'')
        private val FORMAT_LENGTH = setOf('l', 'h', 'L', 'q', 'z', 'j', 't')
        private val FORMAT_CONV = setOf(
            'd', 'i', 'o', 'u', 'x', 'X', 'e', 'E', 'f', 'g', 'G',
            's', 'c', 'b', 'p', 'n', '%',
        )

        /// True if the char after a `$` would START a real interpolation
        /// (rather than being a literal `$`). Letter / underscore for
        /// `$foo`, `{` for `${…}`, `(` for `$(…)`, digit / `?!$#*@-_`
        /// for the single-char special parameters.
        fun isInterpStart(buf: CharSequence, idx: Int): Boolean {
            if (idx >= buf.length) return false
            val c = buf[idx]
            return c == '_' || c.isLetter() || c == '{' || c == '(' ||
                c.isDigit() || c in "?!$#*@-_"
        }

        // ── Keyword & builtin sets (regenerated) ──────────────────────
        // Source: data/grammar/canonical.json
        // Regenerate: python3 scripts/gen_grammar_intellij.py
        // Do not hand-edit between BEGIN/END markers.

        // BEGIN-CANONICAL: control-keywords
        private val CONTROL_KEYWORDS = setOf(
            "!", "[[", "]]", "always", "case", "do", "done", "elif", "else", "end", "esac",
            "fi", "for", "foreach", "if", "in", "repeat", "select", "then", "until",
            "while", "{", "}",
        )
        // END-CANONICAL: control-keywords

        // BEGIN-CANONICAL: decl-keywords
        private val DECL_KEYWORDS = setOf(
            "declare", "export", "float", "integer", "let", "local", "readonly", "set",
            "shift", "typeset",
        )
        // END-CANONICAL: decl-keywords

        // BEGIN-CANONICAL: fn-keywords
        private val FN_KEYWORDS = setOf("function")
        // END-CANONICAL: fn-keywords

        // BEGIN-CANONICAL: loop-keywords
        private val LOOP_KEYWORDS = setOf(
            "break", "continue", "exit", "logout", "return",
        )
        // END-CANONICAL: loop-keywords

        // BEGIN-CANONICAL: modifier-keywords
        private val MODIFIER_KEYWORDS = setOf(
            "alias", "autoload", "bindkey", "builtin", "cdpath", "command", "compaudit",
            "compdef", "compinit", "compinstall", "coproc", "fignore", "fpath", "manpath",
            "nocorrect", "noglob", "path", "setopt", "time", "unalias", "unsetopt", "zcalc",
            "zcompile", "zcp", "zformat", "zftp", "zle", "zln", "zmodload", "zmv",
            "zparseopts", "zsh-newuser-install", "zstyle", "zsystem", "ztcp",
        )
        // END-CANONICAL: modifier-keywords

        // BEGIN-CANONICAL: io-keywords
        private val IO_KEYWORDS = setOf(
            ".", ":", "echo", "eval", "exec", "false", "mapfile", "print", "printf", "read",
            "readarray", "source", "trap", "true",
        )
        // END-CANONICAL: io-keywords

        // BEGIN-CANONICAL: builtins
        private val BUILTINS = setOf(
            "[", "bg", "bye", "cap", "cd", "chdir", "chgrp", "chmod", "chown", "clone",
            "dirs", "disable", "disown", "echotc", "echoti", "emulate", "enable", "example",
            "fc", "fg", "functions", "getcap", "getln", "getopts", "hash", "hashinfo",
            "history", "jobs", "kill", "ln", "log", "mem", "mkdir", "mv", "nameref",
            "patdebug", "pcre_compile", "pcre_match", "pcre_study", "popd", "private",
            "pushd", "pushln", "pwd", "r", "rehash", "rm", "rmdir", "setcap", "stat",
            "strftime", "suspend", "sync", "syserror", "sysopen", "sysread", "sysseek",
            "syswrite", "test", "times", "ttyctl", "type", "umask", "unfunction", "unhash",
            "unset", "wait", "whence", "where", "which", "zask", "zcache", "zcompdump",
            "zcomplete", "zcurses", "zd", "zdelattr", "zf_chgrp", "zf_chmod", "zf_chown",
            "zf_ln", "zf_mkdir", "zf_mv", "zf_rm", "zf_rmdir", "zf_sync", "zgdbmpath",
            "zgetattr", "zhistory", "zid", "zjob", "zlistattr", "zlock", "zlog", "zls",
            "znotify", "zping", "zprof", "zpty", "zpublish", "zregexparse", "zselect",
            "zsend", "zsetattr", "zsocket", "zsource", "zstat", "zsubscribe", "zsuggest",
            "zsync", "ztag", "ztie", "zunsubscribe", "zuntag", "zuntie", "zwc", "zwhere",
            // ztest framework (src/extensions/ztest.rs) — shell-level
            // unit-test builtins. Listed here so the plugin lexer
            // colors them as Builtin (not Identifier) and they survive
            // the "unknown command" inspection.
            "run_tests", "zassert_contains", "zassert_dies", "zassert_eq", "zassert_err",
            "zassert_false", "zassert_ge", "zassert_gt", "zassert_le", "zassert_lt",
            "zassert_match", "zassert_ne", "zassert_near", "zassert_ok", "zassert_true",
            "ztest_run", "ztest_skip",
        )
        // END-CANONICAL: builtins
    }
}
