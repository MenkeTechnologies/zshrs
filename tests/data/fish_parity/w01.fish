set -xn | string match -q HOME; echo $status
set -gx ZZX 1 2; set -xn | string match -q ZZX; echo $status
set -x | string match "ZZX*"
set -ln | count
