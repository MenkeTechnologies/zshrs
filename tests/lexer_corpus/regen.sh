#!/usr/bin/env zsh
# Regenerate the .tokens AND .wordcode reference files committed
# alongside each corpus entry. Two parity harnesses consume these
# offline (no need for zsh/zshrs_dump on every invocation):
#
#   - tests/lexer_parity.rs    reads `<basename>.tokens`
#   - tests/wordcode_parity.rs reads `<basename>.wordcode`
#
# Run after making changes to:
#   - any corpus *.sh / *.zsh (new test cases)
#   - the C-side dump module (zshrs_dump.c at the repo root)
#   - any code path that legitimately changes the C lexer / parser
#
# Requires:
#   - The zshrs_dump module built and placed at one of
#     `src/zsh/Src/Modules/zshrs_dump.{so,bundle}` (and a matching
#     copy under `src/zsh/Src/Modules/zsh/zshrs_dump.bundle` for
#     brew zsh's `module_path/zsh/<name>.bundle` lookup).
#   - `zsh` on PATH (5.9+).

set -e

CORPUS_DIR=${0:A:h}
MODULE_DIR=${CORPUS_DIR:h:h}/src/zsh/Src/Modules

if [[ ! -e $MODULE_DIR/zshrs_dump.so && ! -e $MODULE_DIR/zshrs_dump.bundle ]]; then
    print -u2 "zshrs_dump module not found in $MODULE_DIR"
    print -u2 "Build it from the in-tree zsh + zshrs_dump.c at the repo root."
    exit 1
fi

tokens_count=0
wc_count=0
# Explicit numbered-prefix glob skips regen.sh (which would match *.sh).
for f in $CORPUS_DIR/[0-9]*.sh $CORPUS_DIR/[0-9]*.zsh; do
    zsh -fc "module_path=($MODULE_DIR); zmodload zsh/zshrs_dump && dumptokens   '$f'" > "$f.tokens"
    (( tokens_count += 1 ))
    zsh -fc "module_path=($MODULE_DIR); zmodload zsh/zshrs_dump && dumpwordcode '$f'" > "$f.wordcode"
    (( wc_count += 1 ))
done

print "Regenerated $tokens_count .tokens + $wc_count .wordcode files in $CORPUS_DIR"
