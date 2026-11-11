abbr -a gco git checkout
abbr --show | string match "*gco*"
abbr -l | string match gco
abbr -q gco; echo $status
abbr -q nope; echo $status
abbr -e gco
abbr -q gco; echo $status
echo $_ | count
echo $CMD_DURATION $fish_kill_signal
set -l a 1; set -S a
set -g b; set -S b
set -gx c 1 2; set -S c
set -l e ""; set -S e
