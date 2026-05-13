/*
 * zshrs_dump.c — token-stream dumper for zshrs lex/parse parity testing.
 *
 * Builtin: dumptokens FILE
 *
 *   Reads FILE, runs zsh's zshlex() loop over its contents, prints one
 *   line per token to stdout in the form `TOKNAME\tTOKSTR\n`. The final
 *   line is `ENDINPUT\n` (clean EOF) or `LEXERR\n` (lex error encountered).
 *
 *   TOKNAME is the upper-case enum lextok name from zsh.h:304-336
 *   (NULLTOK, SEPER, NEWLIN, ..., TYPESET). TOKSTR is the lexer's tokstr
 *   for that token, with no escaping — newlines and tabs in tokstr will
 *   make output mis-line; restrict corpus to single-line tokens, or
 *   downstream consumers must handle.
 *
 * Setup follows lex.c:1717-1734 parsestrnoerr — zcontext_save, inpush,
 * strinbeg, drive zshlex, strinend, inpop, zcontext_restore.
 *
 * For consumption by zshrs's `tests/lexer_parity.rs` harness via:
 *   zsh -fc 'zmodload zsh/zshrs_dump; dumptokens FILE'
 */

#include "zshrs_dump.mdh"
#include "zshrs_dump.pro"

/* Order MUST match enum lextok in zsh.h:304-336. */
static const char *toknames[] = {
    "NULLTOK", "SEPER", "NEWLIN", "SEMI", "DSEMI",
    "AMPER", "INPAR", "OUTPAR", "DBAR", "DAMPER",
    "OUTANG", "OUTANGBANG", "DOUTANG", "DOUTANGBANG", "INANG",
    "INOUTANG", "DINANG", "DINANGDASH", "INANGAMP", "OUTANGAMP",
    "AMPOUTANG", "OUTANGAMPBANG", "DOUTANGAMP", "DOUTANGAMPBANG", "TRINANG",
    "BAR", "BARAMP", "INOUTPAR", "DINPAR", "DOUTPAR",
    "AMPERBANG", "SEMIAMP", "SEMIBAR", "DOUTBRACK", "STRING",
    "ENVSTRING", "ENVARRAY", "ENDINPUT", "LEXERR", "BANG",
    "DINBRACK", "INBRACE", "OUTBRACE", "CASE", "COPROC",
    "DOLOOP", "DONE", "ELIF", "ELSE", "ZEND",
    "ESAC", "FI", "FOR", "FOREACH", "FUNC",
    "IF", "NOCORRECT", "REPEAT", "SELECT", "THEN",
    "TIME", "UNTIL", "WHILE", "TYPESET",
};

#define NUM_TOKNAMES (sizeof(toknames) / sizeof(toknames[0]))

static const char *
tok_name(int t)
{
    if (t >= 0 && (size_t) t < NUM_TOKNAMES)
	return toknames[t];
    return "?";
}

/**/
static int
bin_dumptokens(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))
{
    char *path = args[0];
    int fd;
    struct stat st;
    char *buf;
    ssize_t n;

    if (!path) {
	zwarnnam(nam, "missing FILE argument");
	return 1;
    }

    fd = open(path, O_RDONLY);
    if (fd < 0) {
	zwarnnam(nam, "%s: %s", path, strerror(errno));
	return 1;
    }
    if (fstat(fd, &st) < 0) {
	zwarnnam(nam, "%s: stat: %s", path, strerror(errno));
	close(fd);
	return 1;
    }
    buf = (char *) zalloc(st.st_size + 1);
    n = read(fd, buf, st.st_size);
    close(fd);
    if (n < 0) {
	zwarnnam(nam, "%s: read: %s", path, strerror(errno));
	zfree(buf, st.st_size + 1);
	return 1;
    }
    buf[n] = '\0';

    /* Mirror lex.c:1717-1734 parsestrnoerr setup, BUT metafy the buffer
     * first. inpush expects already-metafied input (the shell's normal
     * input path metafies on read in input.c). Without this step, raw
     * 8-bit bytes from a file (e.g. UTF-8 continuation bytes) are seen
     * by the lexer as token markers (0x84-0xa3 fall in the token range)
     * and stripped — the user's `━` (UTF-8 e2 94 81) loses the 0x94
     * byte because lexer treats it as Inang. metafy converts each high
     * byte X to `\x83 (X^0x20)` so token-range collisions don't occur. */
    char *meta_buf = metafy(buf, n, META_DUP);
    zcontext_save();
    inpush(meta_buf, 0, NULL);
    strinbeg(0);

    /* Drive the lexer via `ctxtlex` (lex.c:317), NOT bare `zshlex`.
     * ctxtlex wraps zshlex with the per-token `incmdpos` update logic
     * that zsh's parser would otherwise apply between calls. zshrs's
     * `ZshLexer::zshlex` collapses both — it updates incmdpos itself —
     * so for parity we must use ctxtlex on this side. With bare zshlex,
     * incmdpos would never decrement (no parser running), and reserved
     * words like `typeset` after a STRING would wrongly stay promoted.
     *
     * tokstr is stored in zsh's internal tokenized + metafied form (see
     * zsh.h `Meta '\x83'` + Snull/Bnull/Star/etc. tokens at zsh.h:159-).
     * We call `untokenize` (exec.c:2077) to convert tokens back to source
     * bytes, then `unmetafy` (utils.c:4954) to strip the Meta prefix
     * bytes. Result is plain UTF-8 source — same form zshrs's lexer
     * produces after its own untokenize, enabling byte-equal comparison.
     */
    ctxtlex();
    while (tok != ENDINPUT && tok != LEXERR) {
	if (tokstr) {
	    /* tokstr is METAFIED but NOT tokenized in the way untokenize
	     * expects. The lexer applies Meta-encoding for high bytes so
	     * that token bytes (Pound..Nularg, 0x84-0xa3) are unambiguous
	     * markers. unmetafy reverses Meta-encoding; once that's done,
	     * any remaining 0x84-0xa3 bytes ARE genuine token markers
	     * (e.g. Dnull=0x9e for `"`-quoted regions). Then untokenize
	     * maps them back to source chars via the ztokens[] table.
	     *
	     * Order: unmetafy → untokenize. Reversed order would treat
	     * Meta+(byte^0x20) sequences as 2 separate bytes and risk
	     * collapsing the 2nd one as a token. */
	    char *plain = ztrdup(tokstr);
	    untokenize(plain);
	    unmetafy(plain, NULL);
	    printf("%s\t%s\n", tok_name(tok), plain);
	    zsfree(plain);
	} else {
	    printf("%s\t\n", tok_name(tok));
	}
	ctxtlex();
    }
    if (tok == LEXERR)
	printf("LEXERR\n");
    else
	printf("ENDINPUT\n");
    fflush(stdout);

    strinend();
    inpop();
    zcontext_restore();

    zfree(buf, st.st_size + 1);
    return 0;
}

static struct builtin bintab[] = {
    BUILTIN("dumptokens", 0, bin_dumptokens, 1, 1, 0, NULL, NULL),
};

static struct features module_features = {
    bintab, sizeof(bintab) / sizeof(*bintab),
    NULL, 0,
    NULL, 0,
    NULL, 0,
    0
};

/**/
int
setup_(UNUSED(Module m))
{
    return 0;
}

/**/
int
features_(Module m, char ***features)
{
    *features = featuresarray(m, &module_features);
    return 0;
}

/**/
int
enables_(Module m, int **enables)
{
    return handlefeatures(m, &module_features, enables);
}

/**/
int
boot_(UNUSED(Module m))
{
    return 0;
}

/**/
int
cleanup_(Module m)
{
    return setfeatureenables(m, &module_features, NULL);
}

/**/
int
finish_(UNUSED(Module m))
{
    return 0;
}

