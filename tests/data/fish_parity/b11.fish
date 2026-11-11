set -l list
set -a list x
set -a list y z
echo (count $list) $list
set list
echo (count $list)
set -q list; echo $status
