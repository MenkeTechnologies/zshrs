# RECORDER.md surface row: `typeset` / `declare` / `readonly` /
# `integer` / `float` / `local`. All five (well, four — `local` is
# function-only and intentionally not at top level) emit the same
# `typeset` kind, with the per-builtin attribute encoded in the
# value field (`-r` for readonly, `-i` for integer, `-F`/`-E` for
# float).
typeset t_t=plain
declare t_d=declare
readonly t_r=readonly
integer t_i=42
float t_f=3.14
