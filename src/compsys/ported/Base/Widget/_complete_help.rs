//! Port of `_complete_help` from `Completion/Base/Widget/_complete_help`.
//!
//! Full upstream body (92 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xh
//! sh: 2
//! sh: 3  _complete_help() {
//! sh: 4    eval "$_comp_setup"
//! sh: 5
//! sh: 6    local _sort_tags=_help_sort_tags text i j k tmp
//! sh: 7    typeset -A help_funcs help_tags help_sfuncs help_styles
//! sh: 8
//! sh: 9    local -H _help_scan_funcstack="main_complete|complete|approximate|normal"
//! sh:10    local -H _help_filter_funcstack="alternative|call_function|describe|dispatch|wanted|requested|all_labels|next_label"
//! sh:11
//! sh:12    {
//! sh:13      _shadow compadd compcall zstyle
//! sh:14      compadd() { return 1 }
//! sh:15      compcall() { _help_sort_tags use-compctl }
//! sh:16      zstyle() {
//! sh:17        local _f="${${(@)${(@)funcstack[2,(i)_($~_help_scan_funcstack)]}:#(_($~_help_filter_funcstack)|\((eval|anon)\))}% *}"
//! sh:18
//! sh:19        [[ -z "$_f" ]] && _f="${${(@)funcstack[2,(i)_($~_help_scan_funcstack)]}:#(_($~_help_filter_funcstack)|\((eval|anon)\))}"
//! sh:20
//! sh:21        if [[ "$help_sfuncs[$2]" != *${_f}* ||
//! sh:22              "$help_styles[${2}${_f}]" != *${3}* ]]; then
//! sh:23
//! sh:24          [[ "$help_sfuncs[$2]" != *${_f}* ]] && help_sfuncs[$2]+=$'\0'"${_f}"
//! sh:25          local _t
//! sh:26
//! sh:27          case "$1" in
//! sh:28          -s) _t='[string] ';;
//! sh:29          -a) _t='[array]  ';;
//! sh:30          -h) _t='[assoc]  ';;
//! sh:31          *)  _t='[boolean]';;
//! sh:32          esac
//! sh:33          help_styles[${2}${_f}]+=",${_t} ${3}:${_f}"
//! sh:34        fi
//! sh:35
//! sh:36        # No need to call the completers more than once with different match specs.
//! sh:37
//! sh:38        if [[ "$3" = matcher-list ]]; then
//! sh:39          set -A "$4" ''
//! sh:40        else
//! sh:41          builtin zstyle "$@"
//! sh:42        fi
//! sh:43      }
//! sh:44
//! sh:45      ${1:-_main_complete}
//! sh:46    } always {
//! sh:47      _unshadow compadd compcall zstyle
//! sh:48    }
//! sh:49
//! sh:50    for i in "${(@ok)help_funcs}"; do
//! sh:51      text+=$'\n'"tags in context :completion:${i}:"
//! sh:52      tmp=()
//! sh:53      for j in "${(@ps.\0.)help_funcs[$i][2,-1]}"; do
//! sh:54        tmp+=( "${(@s.,.)help_tags[${i}${j}][2,-1]}" )
//! sh:55      done
//! sh:56      zformat -a tmp '  (' "$tmp[@]"
//! sh:57      tmp=( $'\n    '${^tmp}')' )
//! sh:58      text+="${tmp}"
//! sh:59    done
//! sh:60
//! sh:61    if [[ ${NUMERIC:-1} -ne 1 ]]; then
//! sh:62      text+=$'\n'
//! sh:63      for i in "${(@ok)help_sfuncs}"; do
//! sh:64        text+=$'\n'"styles in context ${i}"
//! sh:65        tmp=()
//! sh:66        for j in "${(@ps.\0.)help_sfuncs[$i][2,-1]}"; do
//! sh:67          tmp+=( "${(@s.,.)help_styles[${i}${j}][2,-1]}" )
//! sh:68        done
//! sh:69        zformat -a tmp '  (' "$tmp[@]"
//! sh:70        tmp=( $'\n    '${^tmp}')' )
//! sh:71        text+="${tmp}"
//! sh:72      done
//! sh:73    fi
//! sh:74    compstate[list]='list force'
//! sh:75    compstate[insert]=''
//! sh:76
//! sh:77    compadd -UX "$text[2,-1]" -n ''
//! sh:78  }
//! sh:79
//! sh:80  _help_sort_tags() {
//! sh:81    local f="${${(@)${(@)funcstack[3,(i)_($~_help_scan_funcstack)]}:#(_($~_help_filter_funcstack)|\((eval|anon)\))}% *}"
//! sh:82
//! sh:83    if [[ "$help_funcs[$curcontext]" != *${f}* ||
//! sh:84          "$help_tags[${curcontext}${f}]" != *(${(j:|:)~argv})* ]]; then
//! sh:85      [[ "$help_funcs[$curcontext]" != *${f}* ]] &&
//! sh:86          help_funcs[$curcontext]+=$'\0'"${f}"
//! sh:87      help_tags[${curcontext}${f}]+=",${argv}:${f}"
//! sh:88      comptry "$@" 2>/dev/null
//! sh:89    fi
//! sh:90  }
//! sh:91
//! sh:92  _complete_help "$@"
//! ```
//!
//! Strict Rust port: two entry points.
//!
//! 1. `_complete_help(state, entries)` — caller passes
//! pre-collected `(topic, description)` pairs and we emit each
//! with `topic -- desc` disp formatting under group `help`.
//! Used when the caller already has the entries (e.g. tag list).
//!
//! 2. `_complete_help_shadow(state, completer, label)` — runs
//! `completer` under `_shadow`, captures everything it would
//! have added, and renders the capture as topic+desc rows. This
//! is the closer analog of what the shell widget does: shadow
//! `compadd`/`zstyle` to RECORD what a completer would do
//! without polluting live state.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
