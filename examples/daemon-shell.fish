# daemon-shell.fish — fish-shell client for zshrs-daemon's HTTP service.
#
# Mirrors examples/daemon-shell.zsh so fish users get the same set of
# wrappers around the universal daemon API. Source from your config:
#
#   source path/to/daemon-shell.fish
#
# Or drop into ~/.config/fish/conf.d/ to autoload.
#
# Endpoint and auth come from env (set in your config.fish):
#
#   set -gx DAEMON_URL    http://127.0.0.1:7733
#   set -gx DAEMON_TOKEN  <bearer-token>             # optional
#   set -gx DAEMON_SHELL_ID fish                     # see docs/SHELL_IDS.md
#
# Federated catalog model: every record sent through daemon-record-* is
# tagged with $DAEMON_SHELL_ID so cross-shell queries (daemon-defs-query
# --shell-id fish, daemon-defs-diff fish zshrs) can narrow / compare.

set -q DAEMON_URL;       or set -gx DAEMON_URL    http://127.0.0.1:7733
set -q DAEMON_TOKEN;     or set -gx DAEMON_TOKEN  ''
set -q DAEMON_SHELL_ID;  or set -gx DAEMON_SHELL_ID fish

# Internal: invoke curl with the right auth header. Branching here keeps
# quoting safe — no IFS-splitting hazard like the bash/zsh wrappers had
# to dodge (fish's command substitution doesn't word-split, so this is
# simpler than the POSIX equivalent).
function _daemon_curl
    if test -n "$DAEMON_TOKEN"
        curl -sS -f -H "Authorization: Bearer $DAEMON_TOKEN" $argv
    else
        curl -sS -f $argv
    end
end

function _daemon_post
    set -l op $argv[1]
    set -l body (test (count $argv) -ge 2; and echo $argv[2]; or echo '{}')
    _daemon_curl \
        -H 'Content-Type: application/json' \
        --data-raw "$body" \
        "$DAEMON_URL/op/$op"
end

function _daemon_get
    _daemon_curl "$DAEMON_URL$argv[1]"
end

# JSON-string escape. Backslash, double-quote, tab, CR, LF — anything
# else passed through. Uses fish's `string replace` so we don't depend
# on BSD vs GNU sed semantics for `\t` / `\n` in the regex.
function _json_str
    string replace --all '\\' '\\\\' -- $argv[1] \
        | string replace --all '"'  '\\"' \
        | string replace --all \t   '\\t' \
        | string replace --all \r   '\\r' \
        | string replace --all \n   '\\n'
end

# ---- Public commands ------------------------------------------------------

function daemon-health;  _daemon_get /health; end
function daemon-ops;     _daemon_get /ops;    end
function daemon-info;    _daemon_post info '{}'; end

function daemon-ping
    set -l body '{}'
    test (count $argv) -gt 0; and set body "{\"echo\":\"$argv\"}"
    _daemon_post ping "$body"
end

function daemon-call
    if test (count $argv) -lt 1
        echo 'usage: daemon-call OP [JSON]' >&2; return 2
    end
    set -l op $argv[1]
    set -l body (test (count $argv) -ge 2; and echo $argv[2]; or echo '{}')
    _daemon_post "$op" "$body"
end

# ---- Federated recorder (definitions.*) ----------------------------------

function _daemon_emit
    # _daemon_emit KIND NAME [VALUE] [FILE] [LINE] [FN_CHAIN]
    set -l kind  $argv[1]
    set -l name  $argv[2]
    set -l value (test (count $argv) -ge 3; and echo $argv[3]; or echo '')
    set -l file  (test (count $argv) -ge 4; and echo $argv[4]; or echo '')
    set -l line  (test (count $argv) -ge 5; and echo $argv[5]; or echo '')
    set -l chain (test (count $argv) -ge 6; and echo $argv[6]; or echo '')
    set -l body "{\"shell_id\":\"$DAEMON_SHELL_ID\",\"kind\":\"$kind\""
    set body $body",\"name\":\""(_json_str "$name")"\""
    test -n "$value"; and set body $body",\"value\":\""(_json_str "$value")"\""
    test -n "$file";  and set body $body",\"file\":\""(_json_str "$file")"\""
    test -n "$line";  and set body $body",\"line\":$line"
    test -n "$chain"; and set body $body",\"fn_chain\":\""(_json_str "$chain")"\""
    set body $body'}'
    _daemon_post definitions_emit "$body"
end

function daemon-record-alias
    if test (count $argv) -lt 2; echo 'usage: daemon-record-alias NAME BODY' >&2; return 2; end
    _daemon_emit alias $argv[1] $argv[2]
end
function daemon-record-galias
    if test (count $argv) -lt 2; echo 'usage: daemon-record-galias NAME BODY' >&2; return 2; end
    _daemon_emit galias $argv[1] $argv[2]
end
function daemon-record-salias
    if test (count $argv) -lt 2; echo 'usage: daemon-record-salias NAME BODY' >&2; return 2; end
    _daemon_emit salias $argv[1] $argv[2]
end
function daemon-record-function
    if test (count $argv) -lt 2; echo 'usage: daemon-record-function NAME BODY' >&2; return 2; end
    _daemon_emit function $argv[1] $argv[2]
end
function daemon-record-export
    if test (count $argv) -lt 2; echo 'usage: daemon-record-export NAME VALUE' >&2; return 2; end
    _daemon_emit env $argv[1] $argv[2]
end
function daemon-record-param
    if test (count $argv) -lt 2; echo 'usage: daemon-record-param NAME VALUE' >&2; return 2; end
    _daemon_emit params $argv[1] $argv[2]
end
function daemon-record-bindkey
    if test (count $argv) -lt 2; echo 'usage: daemon-record-bindkey SEQ WIDGET' >&2; return 2; end
    _daemon_emit bindkey $argv[1] $argv[2]
end
function daemon-record-compdef
    if test (count $argv) -lt 2; echo 'usage: daemon-record-compdef CMD COMPLETER' >&2; return 2; end
    _daemon_emit compdef $argv[1] $argv[2]
end
function daemon-record-zstyle
    if test (count $argv) -lt 2; echo 'usage: daemon-record-zstyle PATTERN STYLE' >&2; return 2; end
    _daemon_emit zstyle $argv[1] $argv[2]
end
function daemon-record-zmodload
    if test (count $argv) -lt 1; echo 'usage: daemon-record-zmodload MODULE' >&2; return 2; end
    _daemon_emit zmodload $argv[1]
end
function daemon-record-setopt
    if test (count $argv) -lt 1; echo 'usage: daemon-record-setopt OPT' >&2; return 2; end
    _daemon_emit setopt $argv[1] on
end
function daemon-record-unsetopt
    if test (count $argv) -lt 1; echo 'usage: daemon-record-unsetopt OPT' >&2; return 2; end
    _daemon_emit setopt $argv[1] off
end
function daemon-record-source
    if test (count $argv) -lt 1; echo 'usage: daemon-record-source PATH' >&2; return 2; end
    _daemon_emit source $argv[1]
end
function daemon-record-path
    if test (count $argv) -lt 1; echo 'usage: daemon-record-path DIR' >&2; return 2; end
    _daemon_emit path $argv[1]
end
function daemon-record-fpath
    if test (count $argv) -lt 1; echo 'usage: daemon-record-fpath DIR' >&2; return 2; end
    _daemon_emit fpath $argv[1]
end
function daemon-record-zle
    if test (count $argv) -lt 1; echo 'usage: daemon-record-zle WIDGET [BODY]' >&2; return 2; end
    set -l body (test (count $argv) -ge 2; and echo $argv[2]; or echo '')
    _daemon_emit zle $argv[1] "$body"
end
function daemon-record-trap
    if test (count $argv) -lt 2; echo 'usage: daemon-record-trap SIGNAL HANDLER' >&2; return 2; end
    _daemon_emit trap $argv[1] $argv[2]
end
function daemon-record-named-dir
    if test (count $argv) -lt 2; echo 'usage: daemon-record-named-dir NAME PATH' >&2; return 2; end
    _daemon_emit named_dir $argv[1] $argv[2]
end
function daemon-record-completion
    if test (count $argv) -lt 1; echo 'usage: daemon-record-completion CMD [PATH]' >&2; return 2; end
    set -l p (test (count $argv) -ge 2; and echo $argv[2]; or echo '')
    _daemon_emit completion $argv[1] "$p"
end

# ---- Federated catalog query / diff --------------------------------------

# daemon-defs-query [--kind K] [--name N] [--prefix P] [--shell-id S] [--limit N]
function daemon-defs-query
    argparse 'kind=' 'name=' 'prefix=' 'shell-id=' 'limit=' -- $argv
    or return 2
    set -l body '{'
    set -l sep ''
    set -q _flag_kind;     and set body $body"$sep\"kind\":\"$_flag_kind\""        ; and set sep ','
    set -q _flag_name;     and set body $body"$sep\"name\":\"$_flag_name\""        ; and set sep ','
    set -q _flag_prefix;   and set body $body"$sep\"prefix\":\"$_flag_prefix\""    ; and set sep ','
    set -q _flag_shell_id; and set body $body"$sep\"shell_id\":\"$_flag_shell_id\""; and set sep ','
    set -q _flag_limit;    and set body $body"$sep\"limit\":$_flag_limit"          ; and set sep ','
    set body $body'}'
    _daemon_post definitions_query "$body"
end

function daemon-defs-kinds
    _daemon_post definitions_kinds '{}'
end

# daemon-defs-diff SHELL_A SHELL_B [KIND]
function daemon-defs-diff
    if test (count $argv) -lt 2
        echo 'usage: daemon-defs-diff SHELL_A SHELL_B [KIND]' >&2; return 2
    end
    set -l body "{\"shell_a\":\"$argv[1]\",\"shell_b\":\"$argv[2]\""
    test (count $argv) -ge 3; and set body $body",\"kind\":\"$argv[3]\""
    set body $body'}'
    _daemon_post definitions_diff "$body"
end

# ---- Pubsub / streaming --------------------------------------------------

function daemon-watch
    if test (count $argv) -lt 1
        echo 'usage: daemon-watch DIR [--recursive]' >&2; return 2
    end
    set -l dir $argv[1]
    set -l recursive false
    test (count $argv) -ge 2; and test "$argv[2]" = '--recursive'; and set recursive true
    _daemon_curl -N "$DAEMON_URL/stream/watch?path=$dir&recursive=$recursive"
end

function daemon-events
    set -l pat (test (count $argv) -ge 1; and echo $argv[1]; or echo '*.*')
    _daemon_curl -N "$DAEMON_URL/stream/events?channel=$pat"
end

function daemon-publish
    if test (count $argv) -ne 2
        echo 'usage: daemon-publish TOPIC JSON_DATA' >&2; return 2
    end
    _daemon_post publish "{\"topic\":\"$argv[1]\",\"data\":$argv[2]}"
end
