//! Port of `_canonical_paths` from `Completion/Unix/Type/_canonical_paths`.
//!
//! Full upstream body (123 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # This completion function completes all paths given to it, and also tries to
//! sh:  4  # offer completions which point to the same file as one of the paths given
//! sh:  5  # (relative path when an absolute path is given, and vice versa; when ..'s are
//! sh:  6  # present in the word to be completed, and some paths got from symlinks).
//! sh:  7
//! sh:  8  # Usage: _canonical_paths [-A var] [-N] [-MJV12onfX] tag desc [paths...]
//! sh:  9
//! sh: 10  # -A, if specified, takes the paths from the array variable specified. Paths
//! sh: 11  # can also be specified on the command line as shown above. -N, if specified,
//! sh: 12  # prevents canonicalizing the paths given before using them for completion, in
//! sh: 13  # case they are already so. `tag' and `desc' arguments are well, obvious :) In
//! sh: 14  # addition, the options -M, -J, -V, -1, -2, -o, -n, -F, -x, -X are passed to
//! sh: 15  # compadd.
//! sh: 16
//! sh: 17  _canonical_paths_add_paths () {
//! sh: 18    # origpref = original prefix
//! sh: 19    # expref = expanded prefix
//! sh: 20    # curpref = current prefix
//! sh: 21    # canpref = canonical prefix
//! sh: 22    # rltrim = suffix to trim and readd
//! sh: 23    local origpref=$1 expref rltrim curpref canpref subdir
//! sh: 24    [[ $2 != add ]] && matches=()
//! sh: 25    expref=${~origpref} 2>/dev/null
//! sh: 26    [[ $origpref == (|*/). ]] && rltrim=.
//! sh: 27    curpref=${${expref%$rltrim}:-./}
//! sh: 28    canpref=$curpref:P
//! sh: 29    [[ $curpref == */ && $canpref == *[^/] ]] && canpref+=/
//! sh: 30    canpref+=$rltrim
//! sh: 31    [[ $expref == *[^/] && $canpref == */ ]] && origpref+=/
//! sh: 32
//! sh: 33    # Append to $matches the subset of $files that matches $canpref.
//! sh: 34    if [[ $canpref == $origpref ]]; then
//! sh: 35      # This codepath honours any -M matchspec parameters.
//! sh: 36      () {
//! sh: 37        local -a tmp_buffer
//! sh: 38        compadd -A tmp_buffer "$__gopts[@]" -a files
//! sh: 39        matches+=( "${(@)tmp_buffer/$canpref/$origpref}" )
//! sh: 40      }
//! sh: 41    else
//! sh: 42      # ### Ideally, this codepath would do what the 'if' above does,
//! sh: 43      # ### but telling compadd to pretend the "word on the command line"
//! sh: 44      # ### is ${"the word on the command line"/$origpref/$canpref}.
//! sh: 45      # ### The following approximates that.
//! sh: 46      matches+=(${(q)${(M)files:#$canpref*}/$canpref/$origpref})
//! sh: 47    fi
//! sh: 48
//! sh: 49    for subdir in $expref?*(@); do
//! sh: 50      _canonical_paths_add_paths ${subdir/$expref/$origpref} add
//! sh: 51    done
//! sh: 52  }
//! sh: 53
//! sh: 54  _canonical_paths() {
//! sh: 55    # The following parameters are used by callee functions:
//! sh: 56    #    __gopts
//! sh: 57    #    matches
//! sh: 58    #    files
//! sh: 59    #    (possibly others)
//! sh: 60
//! sh: 61    local __index
//! sh: 62    typeset -a __gopts __opts
//! sh: 63
//! sh: 64    zparseopts -D -a __gopts M+: J+: V+: o+: 1 2 n F: x+: X+: A:=__opts N=__opts
//! sh: 65
//! sh: 66    : ${1:=canonical-paths} ${2:=path}
//! sh: 67
//! sh: 68    __index=$__opts[(I)-A]
//! sh: 69    (( $__index )) && set -- $@ ${(P)__opts[__index+1]}
//! sh: 70
//! sh: 71    local expl ret=1 tag=$1 desc=$2
//! sh: 72
//! sh: 73    shift 2
//! sh: 74
//! sh: 75    if ! zmodload -F zsh/stat b:zstat 2>/dev/null; then
//! sh: 76      _wanted "$tag" expl "$desc" compadd $__gopts $@ && ret=0
//! sh: 77      return ret
//! sh: 78    fi
//! sh: 79
//! sh: 80    typeset REPLY
//! sh: 81    typeset -a matches files
//! sh: 82
//! sh: 83    if (( $__opts[(I)-N] )); then
//! sh: 84      files=($@)
//! sh: 85    else
//! sh: 86      files+=($@:P)
//! sh: 87    fi
//! sh: 88
//! sh: 89    local base=$PREFIX
//! sh: 90    typeset -i blimit
//! sh: 91
//! sh: 92    _canonical_paths_add_paths $base
//! sh: 93
//! sh: 94    if [[ -z $base ]]; then
//! sh: 95      _canonical_paths_add_paths / add
//! sh: 96    elif [[ $base == ..(/.(|.))#(|/) ]]; then
//! sh: 97
//! sh: 98      # This style controls how many parent directory links (..) to chase searching
//! sh: 99      # for possible completions. The default is 8. Note that this chasing is
//! sh:100      # triggered only when the user enters at least a .. and the path completed
//! sh:101      # contains only . or .. components. A value of 0 turns off .. link chasing
//! sh:102      # altogether.
//! sh:103
//! sh:104      zstyle -s ":completion:${curcontext}:$tag" \
//! sh:105        canonical-paths-back-limit blimit || blimit=8
//! sh:106
//! sh:107      if [[ $base != */ ]]; then
//! sh:108        [[ $base != *.. ]] && base+=.
//! sh:109        base+=/
//! sh:110      fi
//! sh:111      until [[ $base.. -ef $base || blimit -le 0 ]]; do
//! sh:112        base+=../
//! sh:113        _canonical_paths_add_paths $base add
//! sh:114        blimit+=-1
//! sh:115      done
//! sh:116    fi
//! sh:117
//! sh:118    _wanted "$tag" expl "$desc" compadd $__gopts -Q -U -a matches && ret=0
//! sh:119
//! sh:120    return ret
//! sh:121  }
//! sh:122
//! sh:123  _canonical_paths "$@"
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
