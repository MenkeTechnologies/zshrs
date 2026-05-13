#!/usr/bin/env zsh
# Regenerate the .tokens reference files committed alongside each
# corpus entry. The lexer parity harness reads these directly so it
# can run without needing the C-side zsh + zshrs_dump module on every
# invocation.
#
# Run after making changes to:
#   - any corpus *.sh / *.zsh (new test cases)
#   - the C-side dump module (zshrs_dump.c at the repo root)
#   - any code path that legitimately changes the C lexer's output
#
# Requires:
#   - The zshrs_dump module built and placed at one of
#     `src/zsh/Src/Modules/zshrs_dump.{so,bundle}` (and a matching
#     copy under `src/zsh/Src/Modules/zsh/zshrs_dump.bundle` for brew
#     zsh's `module_path/zsh/<name>.bundle` lookup).
#   - `zsh` on PATH (5.9+).
#
# Note: a sibling `dumpwordcode` builtin exists for ad-hoc debugging
# of C wordcode output, but is NOT used for parity. zshrs's runtime
# IR is fusevm bytecode, not wordcode, so byte-for-byte parity at
# that layer is meaningless. Lock parser fidelity via execution
# parity or AST tree-shape parity instead.

set -e

CORPUS_DIR=${0:A:h}
MODULE_DIR=${CORPUS_DIR:h:h}/src/zsh/Src/Modules

if [[ ! -e $MODULE_DIR/zshrs_dump.so && ! -e $MODULE_DIR/zshrs_dump.bundle ]]; then
    print -u2 "zshrs_dump module not found in $MODULE_DIR"
    print -u2 "Build it from the in-tree zsh + zshrs_dump.c at the repo root."
    exit 1
fi

count=0
# Explicit numbered-prefix glob skips regen.sh (which would match *.sh).
for f in $CORPUS_DIR/[0-9]*.sh $CORPUS_DIR/[0-9]*.zsh; do
    zsh -fc "module_path=($MODULE_DIR); zmodload zsh/zshrs_dump && dumptokens '$f'" > "$f.tokens"
    (( count += 1 ))
done

print "Regenerated $count token files in $CORPUS_DIR"
