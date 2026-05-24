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

    override fun start(buffer: CharSequence, startOffset: Int, endOffset: Int, initialState: Int) {
        buf = buffer
        this.endOffset = endOffset
        pos = startOffset
        state = initialState
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
        val c = buf[pos]
        when {
            // Shebang first line takes priority over plain comment
            c == '#' && pos == 0 && peek(1) == '!' -> consumeShebang()
            c == '#' -> consumeLineComment()
            c == '\n' || c == '\r' || c == ' ' || c == '\t' -> consumeWhitespace()
            c == '"' -> consumeString('"', ZshrsTokenTypes.STRING_DQ)
            c == '\'' -> consumeString('\'', ZshrsTokenTypes.STRING_SQ)
            c == '`' -> consumeString('`', ZshrsTokenTypes.BACKTICK)
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
            c == '{' -> emit(1, ZshrsTokenTypes.LBRACE)
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
        tokenType = ZshrsTokenTypes.COMMENT
    }

    private fun consumeWhitespace() {
        var p = pos
        while (p < endOffset && (buf[p] == ' ' || buf[p] == '\t' || buf[p] == '\n' || buf[p] == '\r')) p++
        tokenEnd = p; pos = p
        tokenType = TokenType.WHITE_SPACE
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
        // ${...} parameter expansion — consume the whole {...} block
        if (nxt == '{') {
            p++
            var depth = 1
            while (p < endOffset && depth > 0) {
                val c = buf[p]
                if (c == '{') depth++
                else if (c == '}') depth--
                if (depth == 0) { p++; break }
                if (c == '\\' && p + 1 < endOffset) { p += 2; continue }
                p++
            }
            tokenEnd = p; pos = p
            tokenType = ZshrsTokenTypes.PARAM_EXPANSION
            return
        }
        // $(...) command substitution — punctuation-only; treat as variable
        if (nxt == '(') {
            // Let outer flow handle ( separately; emit just `$`
            tokenEnd = p; pos = p
            tokenType = ZshrsTokenTypes.SIGIL
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

    private fun consumeWord() {
        var p = pos
        while (p < endOffset && (buf[p] == '_' || buf[p].isLetterOrDigit())) p++
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
        private val CONTROL_KEYWORDS = setOf(
            "if", "then", "else", "elif", "fi",
            "for", "foreach", "while", "until", "do", "done",
            "case", "esac", "select", "repeat",
            "in", "time", "coproc", "always", "nocorrect", "noglob",
        )
        private val DECL_KEYWORDS = setOf(
            "local", "typeset", "declare", "export", "readonly",
            "integer", "float", "private", "functions",
            "let", "set", "shift",
        )
        private val FN_KEYWORDS = setOf("function")
        private val LOOP_KEYWORDS = setOf("break", "continue", "return", "exit", "logout")
        private val MODIFIER_KEYWORDS = setOf(
            "alias", "unalias", "setopt", "unsetopt", "zstyle", "zmodload", "zle",
            "autoload", "bindkey", "compdef", "compinit", "compinstall", "compaudit",
            "zcompile", "zparseopts", "zformat", "zmv", "zcp", "zln",
            "zftp", "zcalc", "zsh-newuser-install", "ztcp", "zsystem",
            "fpath", "manpath", "path", "cdpath", "fignore",
        )
        private val IO_KEYWORDS = setOf(
            "source", ".", "eval", "exec", "trap",
            "echo", "print", "printf", "read", "readarray", "mapfile",
            "true", "false", ":",
        )
        private val BUILTINS = setOf(
            "cd", "pwd", "pushd", "popd", "dirs",
            "test", "[", "[[", "]]",
            "umask", "ulimit", "wait", "kill", "jobs", "fg", "bg", "suspend", "disown",
            "history", "fc", "r", "hash", "unhash", "rehash",
            "command", "type", "which", "whence", "where",
            "builtin", "enable", "disable", "noglob",
            "getopts", "getopt", "shift", "unset", "unfunction",
            "trap", "exit",
            "limit", "unlimit", "sched", "vared",
            "stat", "zstat", "zsocket",
            "compadd", "compset", "compcall", "compdescribe", "compfiles",
            "comparguments", "compgroups", "complist", "comppostfuncs", "comppreinit",
            "compstate", "comptags", "comptry", "compvalues",
            "true", "false", "null",
        )
    }
}
