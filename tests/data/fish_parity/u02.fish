set x "if true; echo y; end"
eval $x
eval "echo hi; echo there"
set -l n 3
eval echo n=\$n
