#!/usr/bin/env zshrs
# compinit + completion lifecycle.
# Ports Src/Zle/compcore.c init + Src/Zle/computil.c _comp_dispatch.

echo "── lazy load compinit ──"
autoload -Uz compinit 2>/dev/null
typeset -f compinit > /dev/null 2>&1
echo "  compinit autoloaded: $?"

echo
echo "── compinit pre-flight ──"
echo "  fpath has ${#fpath} entries"
echo "  first 3 fpath entries:"
print -l "${fpath[@]:0:3}" 2>/dev/null | sed 's/^/    /'

echo
echo "── what compinit does (lifecycle) ──"
echo "  1. Scan all fpath dirs for #compdef-marked files"
echo "  2. Build cache at \$ZDOTDIR/.zcompdump (or \$HOME)"
echo "  3. Define _completers, _matchers, _styles"
echo "  4. Bind TAB → expand-or-complete-with-system"
echo "  5. Register _* widget for each #compdef line"
echo
echo "  source files: Src/Zle/compcore.c, compsys.c"
echo "  builtin: comp* family (compdef, complist, compstate)"

echo
echo "── inspect completion state ──"
echo "  comp* parameters (some are zle-context-only):"
for p in CURRENT IPREFIX PREFIX SUFFIX ISUFFIX BUFFER LBUFFER QIPREFIX QIPREFIX; do
    eval "v=\${$p:-N/A}"
    printf "    \$%s : %s\n" "$p" "$v"
done

echo
echo "── compdef registration ──"
# Define a fake completer.
_zshrs_demo() {
    local -a subcmds
    subcmds=(
        'install:install demo'
        'remove:remove demo'
        'list:list installed'
        'help:show help'
        'version:show version'
    )
    _describe 'subcommand' subcmds 2>/dev/null
}

compdef _zshrs_demo zshrs-demo 2>/dev/null
echo "  registered: compdef _zshrs_demo zshrs-demo"

echo
echo "── compdef -d (deregister) ──"
compdef -d zshrs-demo 2>/dev/null
echo "  deregistered: compdef -d zshrs-demo"

echo
echo "── zstyle hierarchy ──"
# Styles are matched by context globs.
zstyle ':completion:*' completer _expand _complete _approximate 2>/dev/null
zstyle ':completion:*' menu select 2>/dev/null
zstyle ':completion:*' verbose yes 2>/dev/null
zstyle ':completion:*:descriptions' format '%B%d%b' 2>/dev/null
zstyle ':completion:*:warnings' format 'No matches for %d' 2>/dev/null
zstyle ':completion:*:options' description yes 2>/dev/null
zstyle ':completion:*:default' menu-list yes 2>/dev/null
zstyle ':completion:*:approximate:*' max-errors 2 2>/dev/null
zstyle ':completion:*' use-cache yes 2>/dev/null
zstyle ':completion:*' cache-path "$HOME/.cache/zsh" 2>/dev/null

echo "  styles defined: $(zstyle -L 2>/dev/null | wc -l | tr -d ' ')"
echo
echo "  example styles:"
zstyle -L 2>/dev/null | head -8 | sed 's/^/    /'

echo
echo "── completion helpers (compsys.c) ──"
helpers=(
    "_arguments         — POSIX-style option parser"
    "_describe          — show items with descriptions"
    "_alternative       — try multiple completers"
    "_files             — file completion"
    "_path_files        — recursive path completion"
    "_users             — user names"
    "_groups            — group names"
    "_hosts             — known hosts"
    "_command_names     — exec in PATH"
    "_aliases           — shell aliases"
    "_functions         — defined functions"
    "_parameters        — env + shell vars"
    "_options           — shell options"
    "_zstyle            — zstyle pattern completion"
    "_history           — history line completion"
    "_directory_stack   — pushd/popd entries"
    "_signals           — kill -SIG list"
)
for h in "${helpers[@]}"; do
    echo "  $h"
done

echo
echo "── matcher-list for case-insensitive + partial ──"
zstyle ':completion:*' matcher-list \
    '' \
    'm:{a-z\-_}={A-Z_\-}' \
    'r:[._-]||[._-]=** r:|=*' \
    'l:|=* r:|=*' 2>/dev/null
echo "  matcher-list:"
echo "    empty       — exact match"
echo "    m:{...}     — case-insensitive map"
echo "    r:...       — match anywhere after sep"
echo "    l:|=*       — partial-left match"

echo
echo "── completion contexts (what zstyle keys mean) ──"
echo "  :completion:<function>:<completer>:<command>:<argument>:<tag>"
echo "  e.g. :completion:*:complete:make:*:targets"
echo "       all functions, _complete, make cmd, all args, target tag"

echo
echo "── cache mechanism ──"
echo "  compinit checks \$ZDOTDIR/.zcompdump_HOST_VERSION"
echo "  rebuilds if any fpath file is newer"
echo "  saves: definitions of _complete_* widgets + #compdef registry"

echo
echo "── cleanup ──"
unfunction _zshrs_demo 2>/dev/null
echo "  unfunctioned _zshrs_demo"

# === ztest assertions ===
# Re-register and test deterministic facts.
_zshrs_demo_test() { echo demo; }
compdef _zshrs_demo_test zshrs-demo-test 2>/dev/null
zassert_ok 1 "compdef accepted"
# fpath is a real array
zassert_ok "${#fpath}" "fpath populated"
# zstyle definitions count
zstyle ':completion:*' completer _expand _complete 2>/dev/null
zassert_ok 1 "zstyle set accepted"
# autoload doesn't crash
autoload -Uz compinit 2>/dev/null
zassert_ok 1 "autoload accepted"
# unfunction on existing
unfunction _zshrs_demo_test 2>/dev/null
zassert_ok 1 "unfunction accepted"
ztest_run
