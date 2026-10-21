#!/bin/sh
# Build the bash and ksh93 references the emulation suites diff `--bash` and
# `--ksh` against (tests/emulation_parity.rs, tests/builtin_output_parity.rs)
# into a prefix whose bin/ goes first on PATH.
#
#   scripts/build_ref_shells.sh [prefix]
#
# zshrs advertises bash 5.3.20 (dash_mode::BASH_VERSION_*) and follows
# ksh93u+m 1.0.10; distro packages lag both (Ubuntu 24.04: bash 5.2.21,
# ksh93u+m 1.0.8), and the rows where those releases answer differently are
# not zshrs defects. macOS CI gets the same two versions from Homebrew; the
# pins and checksums below are Homebrew's, so both runners measure one release.
set -eu
prefix=${1:-$HOME/.cache/zshrs/ref-shells}
work=$(mktemp -d "${TMPDIR:-/tmp}/ref-shells.XXXXXX")
trap 'rm -rf "$work" 2>/dev/null || :' EXIT
jobs=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)

fetch() { # url sha256 out
    curl -fsSL --retry 5 -o "$3" "$1"
    if command -v sha256sum >/dev/null; then sum=sha256sum; else sum='shasum -a 256'; fi
    echo "$2  $3" | $sum -c - >/dev/null
}

# bash 5.3 + the official patches 001-020 (= 5.3.20).
gnu=https://ftp.gnu.org/gnu/bash
fetch "$gnu/bash-5.3.tar.gz" 0d5cd86965f869a26cf64f4b71be7b96f90a3ba8b3d74e27e8e9d9d5550f31ba "$work/bash.tar.gz"
tar -xzf "$work/bash.tar.gz" -C "$work"
while read -r n sum; do
    fetch "$gnu/bash-5.3-patches/bash53-$n" "$sum" "$work/bash53-$n"
    patch -s -p0 -d "$work/bash-5.3" < "$work/bash53-$n"
done <<'EOF'
001 1f608434364af86b9b45c8b0ea3fb3b165fb830d27697e6cdfc7ac17dee3287f
002 e385548a00130765ec7938a56fbdca52447ab41fabc95a25f19ade527e282001
003 f245d9c7dc3f5a20d84b53d249334747940936f09dc97e1dcb89fc3ab37d60ed
004 9591d245045529f32f0812f94180b9d9ce9023f5a765c039b852e5dfc99747d0
005 cca1ef52dbbf433bc98e33269b64b2c814028efe2538be1e2c9a377da90bc99d
006 29119addefed8eff91ae37fd51822c31780ee30d4a28376e96002706c995ff10
007 c0976bbfffa1453c7cfdd62058f206a318568ff2d690f5d4fa048793fa3eb299
008 097cd723cbfb8907674ac32214063a3fd85282657ec5b4e544d2c0f719653fb4
009 eee30fe78a4b0cb2fe20e010e00308899cfc613e0774ebb3c8557a1552f24f8c
010 cf76f1cce2ea300c18bff9f002d21f280cc931acd17c28518110b93fe6e72569
011 0298df8f5ea2a31d3be43ed7d269c5b3c7c342dd5b570bea7f64d66dcbbe7531
012 d71379b39bebaedaf123414414e77fb458a0a43b9ad3116594c6df7ca6754573
013 042f9cda967e24bf4211944697441e93d06ff42b4b998629a98a1b249279f200
014 bd4360b401d38507e358783dcad8536a99c6789f0d3a5bd0cfb8c4a34144696c
015 55b79ceee2fc27f6767eed697e939a7eb2fe2a28c01556bd75f18d581014f46e
016 9ea29b266b7d24cb34d0ff3f1c4631e4d527bfe2d1ef15d17cdb924bf31ef767
017 443b927b45c1558ca72052410f8b8f6e5152b617ed707061a2781d4375b0d1c3
018 ae715d76c50341d7d7095e9a8d2eeed1ca9546152c2ac7289206f90cf30ac697
019 a25c581e4d0057dea3833918438a930e2e86ee4c6dc17fe15267b7f04cbc4e3d
020 df217ed3a9122aa2286d9b67bbe348661b6a9db262b580c29150dae55d532896
EOF
(cd "$work/bash-5.3" && ./configure -q --prefix="$prefix" --without-bash-malloc >/dev/null && make -s -j"$jobs" >/dev/null && make -s install >/dev/null)

# ksh93u+m 1.0.10, statically linked: a dynamic build installs libshell.so
# into a lib/ the dynamic loader does not search.
git clone -q --depth 1 --branch v1.0.10 https://github.com/ksh93/ksh.git "$work/ksh"
(cd "$work/ksh" && AST_NO_DYLIB=y bin/package make >/dev/null && bin/package install "$prefix" >/dev/null)

"$prefix/bin/bash" --version | head -1
"$prefix/bin/ksh" -c 'echo "$KSH_VERSION"'
