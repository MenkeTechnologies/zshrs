#!/bin/sh
# Build the reference zsh the parity suite compares against
# (tests/parity/oracle.rs): the development tree the port follows, installed
# with its loadable modules (zsh/pcre needs pcre2-config on PATH) into
# ~/.cache/zshrs/zsh-oracle.
#
#   scripts/build_zsh_oracle.sh [zsh-source-tree]   # default ~/forkedRepos/zsh
set -eu
src=${1:-$HOME/forkedRepos/zsh}
prefix=$HOME/.cache/zshrs/zsh-oracle
work=$(mktemp -d "${TMPDIR:-/tmp}/zsh-oracle.XXXXXX")
trap 'rm -rf "$work"' EXIT
# A full clone: $ZSH_PATCHLEVEL comes from `git describe` and needs the tags.
git clone -q "$src" "$work/zsh"
cd "$work/zsh"
./Util/preconfig >/dev/null
./configure -q --prefix="$prefix" --enable-multibyte --enable-pcre
make -s -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
make -s install.bin install.modules
"$prefix/bin/zsh" -f -c 'print -r -- "zsh oracle: $ZSH_VERSION at $0"'
