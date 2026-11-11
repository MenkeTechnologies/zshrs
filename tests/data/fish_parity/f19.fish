set -q HOME; echo $status
set -q NOPE_XX; echo $status
set -q HOME NOPE_XX; echo $status
set -q argv; echo $status
set -l e
set -q e; echo $status
set -q e[1]; echo $status
