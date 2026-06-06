#!/usr/bin/env zshrs
# compsys / compdef — completion system intro.
# Ports Src/Zle/compsys.c + Src/Zle/computil.c.

echo "── compsys availability ──"
autoload -Uz compinit 2>/dev/null
echo "  compinit available: $(typeset -f compinit > /dev/null 2>&1 && echo yes || echo no)"
zmodload zsh/complete 2>/dev/null
echo "  zsh/complete module: $(zmodload -L 2>/dev/null | grep -c complete) loaded"

echo
echo "── completion function definition ──"
# Define a completer for 'mytool'.
_mytool() {
    local -a commands
    commands=(
        'add:add an item'
        'remove:remove an item'
        'list:list all items'
        'help:show help'
        'version:show version'
    )
    local -a options
    options=(
        '-v[verbose]'
        '-q[quiet]'
        '-f[force]'
        '-h[help]'
        '--config=[config file]:file:_files'
    )
    _arguments \
        "${options[@]}" \
        '1:command:->cmd' \
        '*::arg:->args' 2>/dev/null
    case $state in
        cmd) _describe 'command' commands 2>/dev/null ;;
        args)
            case $words[1] in
                add)    _message 'item to add' 2>/dev/null ;;
                remove) _message 'item to remove' 2>/dev/null ;;
                list)   _arguments '-l[long format]' '-a[show all]' 2>/dev/null ;;
            esac
            ;;
    esac
}

# Register.
compdef _mytool mytool 2>/dev/null
echo "  registered: compdef _mytool mytool"
echo "  registered: compdef _mytool mytool-cli"
compdef _mytool mytool-cli 2>/dev/null

echo
echo "── _arguments style spec ──"
echo "  spec elements:"
echo "    '-v[verbose]'         — short flag with description"
echo "    '--config=[…]:file:_files' — long flag with value type"
echo "    '1:command:->state'   — positional arg w/ state transition"
echo "    '*::arg:->args'       — repeating args"
echo "    '(-a -b)-c'           — mutually exclusive"
echo "    ':arg:(opt1 opt2)'    — fixed value set"
echo "    ':arg:_files -g \"*.txt\"' — file glob"

echo
echo "── _describe / _alternative / _files / _path_files ──"
echo "  helpers (Src/Zle/compsys.c):"
echo "    _describe NAME ARRAY   — show items with descriptions"
echo "    _alternative           — try multiple completers"
echo "    _files                 — file completion"
echo "    _path_files            — path completion"
echo "    _users                 — user names"
echo "    _hosts                 — hostnames"
echo "    _command_names         — shell commands"

echo
echo "── completion contexts ──"
echo "  zstyle ':completion:<func>:<completer>:<command>:<argument>:<tag>' …"
echo "  examples:"
echo "    zstyle ':completion:*' completer _expand _complete _approximate"
echo "    zstyle ':completion:*' menu select"
echo "    zstyle ':completion:*' group-name ''"

echo
echo "── apply zstyle defaults ──"
zstyle ':completion:*' menu select 2>/dev/null
zstyle ':completion:*' group-name '' 2>/dev/null
zstyle ':completion:*:descriptions' format '%B%d%b' 2>/dev/null
zstyle ':completion:*:warnings' format 'No matches: %d' 2>/dev/null
zstyle ':completion:*' verbose yes 2>/dev/null
echo "  styles applied"

echo
echo "── show registered styles ──"
zstyle -L 2>/dev/null | head -10 | sed 's/^/  /'

echo
echo "── matcher list ──"
zstyle ':completion:*' matcher-list \
    'm:{a-z}={A-Z}' \
    'r:|[._-]=* r:|=*' \
    'l:|=* r:|=*' 2>/dev/null
echo "  case-insensitive + partial-word matchers configured"

echo
echo "── unregister ──"
compdef -d mytool 2>/dev/null
compdef -d mytool-cli 2>/dev/null
echo "  compdef -d mytool : deregistered"

unfunction _mytool 2>/dev/null

echo
echo "── what compsys does in interactive mode ──"
echo "  - User presses TAB after typing a command"
echo "  - zle calls the _command function (e.g. _mytool)"
echo "  - _mytool calls _arguments / _describe / _files etc."
echo "  - Compsys assembles completions, applies zstyle, displays menu"
echo "  - User picks or types more chars; menu narrows"
echo
echo "  All driven by:"
echo "    Src/Zle/complete.c   — completion dispatch"
echo "    Src/Zle/compsys.c    — main completion system"
echo "    Src/Zle/computil.c   — _arguments + helpers"
echo "    Src/Zle/zle_keymap.c — TAB binding"

# === ztest assertions ===
# Re-register and verify compdef + zstyle accept the calls
_ct_widget() { :; }
compdef _ct_widget ct_widget 2>/dev/null
zassert_eq "$?" "0"                            "compdef accepts registration"
compdef -d ct_widget 2>/dev/null
zassert_eq "$?" "0"                            "compdef -d accepts dereg"
unfunction _ct_widget 2>/dev/null
# zstyle accepts pattern + style
zstyle ':completion:ct:*' menu select 2>/dev/null
zassert_eq "$?" "0"                            "zstyle set"
# zstyle -L lists registered styles (or just exits 0 when none)
zstyle -L 2>/dev/null > /dev/null
zassert_eq "$?" "0"                            "zstyle -L exits cleanly"
# autoload + compinit available
typeset -f compinit > /dev/null 2>&1 && r=1 || r=0
zassert_eq "$r" "1"                            "compinit autoloaded"
ztest_run
