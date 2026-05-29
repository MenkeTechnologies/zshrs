#!/usr/bin/env zshrs
# Mini-make in pure zsh — target dispatch + dep tracking.

typeset -A TARGETS
typeset -A DEPS

define_target() {
    local name=$1
    local recipe=$2
    local deps=$3
    TARGETS[$name]=$recipe
    DEPS[$name]=$deps
}

built=()
build() {
    local target=$1
    # Check if already built.
    for b in $built; do
        [[ $b == $target ]] && return
    done
    if [[ -z ${TARGETS[$target]+x} ]]; then
        echo "no such target: $target"
        return 1
    fi
    # Build deps first.
    for dep in ${(s/ /)DEPS[$target]}; do
        build "$dep"
    done
    # Run recipe.
    echo "==> building $target"
    eval "${TARGETS[$target]}"
    built+=($target)
}

# Define a build graph:
define_target lex     "echo '  lex.o: compiling'"            ""
define_target parse   "echo '  parse.o: compiling'"          ""
define_target utils   "echo '  utils.o: compiling'"          ""
define_target lib     "echo '  lib.a: archiving'"            "lex parse utils"
define_target main    "echo '  main.o: compiling'"           "utils"
define_target app     "echo '  app: linking final binary'"   "main lib"
define_target test    "echo '  running tests'"               "app"
define_target clean   "echo '  rm -f *.o *.a app'"           ""
define_target install "echo '  cp app /usr/local/bin/'"      "app"

echo "── available targets ──"
print -l ${(ko)TARGETS}

echo "── build 'app' (chain of deps) ──"
build app

echo "── built order: ${built[@]} ──"

echo "── second 'build app' (no-op) ──"
build app

echo "── build 'install' (re-uses cached app) ──"
build install

echo "── clean (independent target) ──"
build clean
