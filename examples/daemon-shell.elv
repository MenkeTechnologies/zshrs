# daemon-shell.elv — elvish client for zshrs-daemon's HTTP service.
#
# Source from your ~/.config/elvish/rc.elv:
#
#     use ./daemon-shell.elv
#     daemon-shell:ping
#     daemon-shell:record-alias gst 'git status'
#     daemon-shell:defs-query &kind=alias &shell-id=elvish
#
# Or load into the global namespace:
#
#     eval (slurp < /path/to/daemon-shell.elv)
#
# Endpoint and auth from env (in your rc.elv):
#
#     set-env DAEMON_URL       http://127.0.0.1:7733
#     set-env DAEMON_TOKEN     <bearer-token>             # optional
#     set-env DAEMON_SHELL_ID  elvish                     # see docs/SHELL_IDS.md

use os
use str

if (not (has-env DAEMON_URL))      { set-env DAEMON_URL      'http://127.0.0.1:7733' }
if (not (has-env DAEMON_TOKEN))    { set-env DAEMON_TOKEN    '' }
if (not (has-env DAEMON_SHELL_ID)) { set-env DAEMON_SHELL_ID 'elvish' }

# ---- Internal helpers -----------------------------------------------------

# Run curl with the right auth header. Elvish doesn't ship an HTTP
# client, so we shell out — same as zsh/bash/fish wrappers.
fn _curl {|@args|
    if (not-eq $E:DAEMON_TOKEN '') {
        curl -sS -f -H 'Authorization: Bearer '$E:DAEMON_TOKEN $@args
    } else {
        curl -sS -f $@args
    }
}

# POST /op/<name> with raw JSON body.
fn _post {|op body|
    _curl -H 'Content-Type: application/json' --data-raw $body $E:DAEMON_URL'/op/'$op
}

fn _get {|endpoint|
    _curl $E:DAEMON_URL$endpoint
}

# JSON-string escape: backslash, quote, tab, CR, LF — anything else
# passes through. Uses elvish's str module so we don't shell out for
# a per-call hot path.
fn _json-str {|s|
    var out = $s
    set out = (str:replace-all '\' '\\' $out)
    set out = (str:replace-all '"' '\"' $out)
    set out = (str:replace-all "\t" '\t' $out)
    set out = (str:replace-all "\r" '\r' $out)
    set out = (str:replace-all "\n" '\n' $out)
    put $out
}

# Build a JSON object from a [&key=val ...] map. Skip empty-string
# values so optional fields drop out cleanly.
fn _json-obj {|m|
    var parts = [(keys $m | each {|k|
        var v = $m[$k]
        if (not-eq $v '') {
            put '"'$k'":"'(_json-str $v)'"'
        }
    })]
    put '{'(str:join ',' $parts)'}'
}

# ---- Public commands ------------------------------------------------------

fn health { _get /health }
fn ops    { _get /ops }
fn info   { _post info '{}' }

fn ping {|@echo|
    if (== (count $echo) 0) {
        _post ping '{}'
    } else {
        _post ping '{"echo":"'(_json-str (str:join ' ' $echo))'"}'
    }
}

fn call {|op &body='{}'|
    _post $op $body
}

# ---- Federated recorder (definitions.*) ----------------------------------

fn _emit {|kind name &value='' &file='' &line='' &fn-chain=''|
    var body = '{"shell_id":"'$E:DAEMON_SHELL_ID'","kind":"'$kind'","name":"'(_json-str $name)'"'
    if (not-eq $value '') { set body = $body',"value":"'(_json-str $value)'"' }
    if (not-eq $file '')  { set body = $body',"file":"'(_json-str $file)'"' }
    if (not-eq $line '')  { set body = $body',"line":'$line }
    if (not-eq $fn-chain '') { set body = $body',"fn_chain":"'(_json-str $fn-chain)'"' }
    set body = $body'}'
    _post definitions_emit $body
}

fn record-alias       {|name body|       _emit alias    $name &value=$body }
fn record-galias      {|name body|       _emit galias   $name &value=$body }
fn record-salias      {|name body|       _emit salias   $name &value=$body }
fn record-function    {|name body|       _emit function $name &value=$body }
fn record-export      {|name value|      _emit env      $name &value=$value }
fn record-param       {|name value|      _emit params   $name &value=$value }
fn record-bindkey     {|seq widget|      _emit bindkey  $seq  &value=$widget }
fn record-compdef     {|cmd completer|   _emit compdef  $cmd  &value=$completer }
fn record-zstyle      {|pattern style|   _emit zstyle   $pattern &value=$style }
fn record-zmodload    {|module|          _emit zmodload $module }
fn record-setopt      {|opt|             _emit setopt   $opt   &value=on }
fn record-unsetopt    {|opt|             _emit setopt   $opt   &value=off }
fn record-source      {|path|            _emit source   $path }
fn record-path        {|dir|             _emit path     $dir }
fn record-fpath       {|dir|             _emit fpath    $dir }
fn record-zle         {|widget &body=''| _emit zle      $widget &value=$body }
fn record-trap        {|signal handler|  _emit trap     $signal &value=$handler }
fn record-named-dir   {|name path|       _emit named_dir $name &value=$path }
fn record-completion  {|cmd &path=''|    _emit completion $cmd &value=$path }

# ---- Federated catalog query / diff --------------------------------------

# `daemon-shell:defs-query &kind=K &name=N &prefix=P &shell-id=S &limit=N`
fn defs-query {|&kind='' &name='' &prefix='' &shell-id='' &limit=''|
    var parts = []
    if (not-eq $kind '')     { set parts = [$@parts '"kind":"'$kind'"'] }
    if (not-eq $name '')     { set parts = [$@parts '"name":"'$name'"'] }
    if (not-eq $prefix '')   { set parts = [$@parts '"prefix":"'$prefix'"'] }
    if (not-eq $shell-id '') { set parts = [$@parts '"shell_id":"'$shell-id'"'] }
    if (not-eq $limit '')    { set parts = [$@parts '"limit":'$limit] }
    _post definitions_query '{'(str:join ',' $parts)'}'
}

fn defs-kinds { _post definitions_kinds '{}' }

# `daemon-shell:defs-diff SHELL_A SHELL_B [&kind=K]`
fn defs-diff {|shell-a shell-b &kind=''|
    var body = '{"shell_a":"'$shell-a'","shell_b":"'$shell-b'"'
    if (not-eq $kind '') { set body = $body',"kind":"'$kind'"' }
    set body = $body'}'
    _post definitions_diff $body
}

# ---- Pubsub --------------------------------------------------------------

fn watch {|dir &recursive=$false|
    var rec = (if $recursive { put true } else { put false })
    _curl -N $E:DAEMON_URL'/stream/watch?path='$dir'&recursive='$rec
}

fn events {|&channel='*.*'|
    _curl -N $E:DAEMON_URL'/stream/events?channel='$channel
}

fn publish {|topic data|
    _post publish '{"topic":"'$topic'","data":'$data'}'
}
