#!/bin/sh
# Build the reference zsh the parity suite compares against
# (tests/parity/oracle.rs): the development tree the port follows, installed
# with its loadable modules (zsh/pcre needs pcre2-config on PATH) and its
# autoloadable function library (Functions/, Completion/ — the default $fpath) into
# ~/.cache/zshrs/zsh-oracle.
#
#   scripts/build_zsh_oracle.sh [zsh-source-tree [configure-args...]]
#
# The source tree defaults to ~/forkedRepos/zsh. Extra arguments go to
# configure — e.g. `--enable-etcdir=/etc/zsh` on Debian/Ubuntu, so the oracle
# reads the same system zshenv the platform's zsh (and zshrs, see
# src/extensions/global_rc.rs) reads.
set -eu
src=${1:-$HOME/forkedRepos/zsh}
[ $# -gt 0 ] && shift
prefix=$HOME/.cache/zshrs/zsh-oracle
work=$(mktemp -d "${TMPDIR:-/tmp}/zsh-oracle.XXXXXX")
trap 'rm -rf "$work"' EXIT
# A full clone: $ZSH_PATCHLEVEL comes from `git describe` and needs the tags.
git clone -q "$src" "$work/zsh"
cd "$work/zsh"
./Util/preconfig >/dev/null
./configure -q --prefix="$prefix" --enable-multibyte --enable-pcre "$@"
make -s -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
make -s install.bin install.modules install.fns
"$prefix/bin/zsh" -f -c 'print -r -- "zsh oracle: $ZSH_VERSION at $0"'
