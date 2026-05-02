#!/usr/bin/env bash

if [[ "$ZPWR_REMOTE" == false ]]; then

    zpwrIsZsh && zpwrLoadJenv >/dev/null

    eval $(perl -I ~/perl5/lib/perl5 -Mlocal::lib 2> /dev/null)

    zpwrCommandExists opam && eval $(opam env)

    export PATH="$HOMEBREW_PREFIX/anaconda3/bin:$PATH"
    return 0
fi
