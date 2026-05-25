//! Port of `_combination` from `Completion/Base/Utility/_combination`.
//!
//! Full upstream body (102 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # Usage:
//! sh:  4  #   _combination [-s S] TAG STYLE \
//! sh:  5  #     Ki1[:Ni1]=Pi1 Ki2[:Ni2]=Pi2 ... Kim[:Nim]=Pim Kj[:Nj] EXPL...
//! sh:  6  #
//! sh:  7  #  STYLE should be of the form K1-K2-...-Kn.
//! sh:  8  #
//! sh:  9  # Example: telnet
//! sh: 10  #
//! sh: 11  #  Assume a user sets the style `users-hosts-ports' as for the my-accounts
//! sh: 12  #  tag:
//! sh: 13  #
//! sh: 14  #    zstyle ':completion:*:*:telnet:*:my-accounts' users-hosts-ports \
//! sh: 15  #      @host0: user1@host1: user2@host2:
//! sh: 16  #      @mail-server:{smtp,pop3}
//! sh: 17  #      @news-server:nntp
//! sh: 18  #      @proxy-server:8000
//! sh: 19  #
//! sh: 20  #
//! sh: 21  #  `_telnet' completes hosts as:
//! sh: 22  #
//! sh: 23  #    _combination my-accounts users-hosts-ports \
//! sh: 24  #      ${opt_args[-l]:+users=${opt_args[-l]:q}} \
//! sh: 25  #      hosts "$expl[@]"
//! sh: 26  #
//! sh: 27  #  This completes `host1', `host2', `mail-server', `news-server' and
//! sh: 28  #  `proxy-server' according to the user given with `-l' if it is exists.
//! sh: 29  #  And if it is failed, `_hosts' is called.
//! sh: 30  #
//! sh: 31  #  `_telnet' completes ports as:
//! sh: 32  #
//! sh: 33  #    _combination my-accounts users-hosts-ports \
//! sh: 34  #      ${opt_args[-l]:+users=${opt_args[-l]:q}} \
//! sh: 35  #      hosts="${line[2]:q}" \
//! sh: 36  #      ports "$expl[@]"
//! sh: 37  #
//! sh: 38  #  This completes `smtp', `pop3', `nntp' and `8000' according to the
//! sh: 39  #  host argument --- $line[2] and the user option argument if it is
//! sh: 40  #  exists. And if it is failed, `_ports' is called.
//! sh: 41  #
//! sh: 42  #  `_telnet' completes users for an argument of option `-l' as:
//! sh: 43  #
//! sh: 44  #    _combination my-accounts users-hosts-ports \
//! sh: 45  #      ${line[2]:+hosts="${line[2]:q}"} \
//! sh: 46  #      ${line[3]:+ports="${line[3]:q}"} \
//! sh: 47  #      users "$expl[@]"
//! sh: 48  #
//! sh: 49  #  This completes `user1' and `user2' according to the host argument and
//! sh: 50  #  the port argument if they are exist. And if it is failed, `_users' is
//! sh: 51  #  called.
//! sh: 52
//! sh: 53  local sep tag style keys pats key num tmp
//! sh: 54
//! sh: 55  if [[ "$1" = -s ]]; then
//! sh: 56    sep="$2"
//! sh: 57    shift 2
//! sh: 58  elif [[ "$1" = -s* ]]; then
//! sh: 59    sep="${1[3,-1]}"
//! sh: 60    shift
//! sh: 61  else
//! sh: 62    sep=:
//! sh: 63  fi
//! sh: 64
//! sh: 65  tag="$1"
//! sh: 66  style="$2"
//! sh: 67  shift 2
//! sh: 68
//! sh: 69  keys=( ${(s/-/)style} )
//! sh: 70  pats=( "${(@)keys/*/*}" )
//! sh: 71
//! sh: 72  while [[ "$1" = *=* ]]; do
//! sh: 73    tmp="${1%%\=*}"
//! sh: 74    key="${tmp%:*}"
//! sh: 75    if [[ $1 = *:* ]]; then
//! sh: 76      num=${tmp##*:}
//! sh: 77    else
//! sh: 78      num=1
//! sh: 79    fi
//! sh: 80    pats[$keys[(in:num:)$key]]="${1#*\=}"
//! sh: 81    shift
//! sh: 82  done
//! sh: 83
//! sh: 84  key="${1%:*}"
//! sh: 85  if [[ $1 = *:* ]]; then
//! sh: 86    num=${1##*:}
//! sh: 87  else
//! sh: 88    num=1
//! sh: 89  fi
//! sh: 90  shift
//! sh: 91
//! sh: 92  if zstyle -a ":completion:${curcontext}:$tag" "$style" tmp; then
//! sh: 93    eval "tmp=( \"\${(@M)tmp:#\${(j($sep))~pats}}\" )"
//! sh: 94    if (( keys[(in:num:)$key] != 1 )); then
//! sh: 95      eval "tmp=( \${tmp#\${(j(${sep}))~\${(@)\${(@)keys[2,(rn:num:)\$key]}/*/*}}${~sep}} )"
//! sh: 96    fi
//! sh: 97    tmp=( ${tmp%%${~sep}*} )
//! sh: 98
//! sh: 99    compadd "$@" -a tmp || { (( $+functions[_$key] )) && "_$key" "$@" }
//! sh:100  else
//! sh:101    (( $+functions[_$key] )) && "_$key" "$@"
//! sh:102  fi
//! ```
//!
//! The previous Rust stub took `specs: &[(&str, Vec<String>)]` and
//! emitted `key=value` strings — entirely wrong shape. Re-port from
//! scratch.
//!
//! Algorithm (mirrors shell:69-101):
//! 1. Split `style` by `-` → axis-key list (`users / hosts / ports`).
//! 2. Init patterns to `*` per axis (matches anything).
//! 3. Walk `K[:N]=Pattern` fixed-axis args, install Pattern at the
//! N-th occurrence of K in the axis list.
//! 4. Last positional `K[:N]` (no `=`) is the **target axis** —
//! this is what the user wants completed.
//! 5. Look up `zstyle ":completion:$curcontext:$tag" $style` for a
//! list of tuple strings.
//! 6. Keep only tuples where each axis matches its pattern (using
//! `${(j(sep))pats}` joined glob).
//! 7. Strip the first `(target_axis_position - 1)` axis fields from
//! each tuple (so what remains starts at the target axis).
//! 8. Take the first axis value of each remaining tuple — these are
//! the candidates.
//! 9. compadd them, or call `_$target_key` as fallback if nothing.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
