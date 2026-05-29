#!/usr/bin/env zshrs
# Directory stack — pushd/popd/dirs.
# Ported from Src/builtin.c bin_pushd / bin_popd / bin_dirs.

start=$PWD
echo "── starting dir: $start ──"

mkdir -p /tmp/zshrs_ds_$$/{a,b/c,d}
cd /tmp/zshrs_ds_$$

echo "── pushd ──"
pushd a
echo "  pwd: $PWD"
dirs

pushd ../b/c
echo "  pwd: $PWD"
dirs

pushd ../../d
echo "  pwd: $PWD"
dirs

echo "── popd ──"
popd
echo "  pwd: $PWD"
dirs

popd
echo "  pwd: $PWD"
dirs

popd
echo "  pwd: $PWD"

echo "── dirs -v (numbered) ──"
pushd a >/dev/null
pushd ../b >/dev/null
pushd ../d >/dev/null
dirs -v

echo "── jump to numbered entry ──"
cd ~2 2>/dev/null && echo "  jumped: $PWD"

# Restore
cd "$start"
rm -rf /tmp/zshrs_ds_$$

echo "── back: $PWD ──"
