# RECORDER.md surface row: `setopt OPT` / `unsetopt OPT` / `set -o opt`.
# The third form (POSIX) goes through `builtin_set`, not
# `builtin_setopt`, so it needs its own hook. Emits the same `setopt`
# / `unsetopt` kind as the zsh-style forms.
set -o EXTENDED_GLOB
set -o GLOB_DOTS
set +o BEEP
set +o CASE_GLOB
