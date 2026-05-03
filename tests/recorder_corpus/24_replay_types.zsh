# Replay-grade type tracking. Every assignment must carry enough
# structured info on the wire (attrs bitset + value_array / value_assoc
# payloads) for the daemon to reconstruct the EXACT typed declaration
# on a future replay — `typeset -<flags> NAME=val`, `name=(elem ...)`,
# `name=(k1 v1 ...)` — without guessing.
#
# This corpus exercises the matrix: scalar / scalar+export / array /
# array-append / assoc-bulk-init / assoc-element-set / integer / float
# / readonly. Every event the harness pulls from the summary should be
# accounted for here.

# Plain scalar
PROJECT=zshrs

# Scalar then re-export — second event must carry [scalar,export]
typeset -gx EDITOR=vim

# Plain indexed array (replay needs ordered elements)
arr=(alpha beta gamma)

# Array append (replay needs APPEND attr bit)
arr+=(delta)

# Assoc with bulk init (replay needs ordered key/value pairs)
typeset -A h=(k1 v1 k2 v2)

# Assoc subscript add
h[k3]=v3

# Integer (replay needs INTEGER attr)
integer count=42

# Float
float pi=3.14

# Readonly scalar
readonly RO=ro_val
