set -l list 1 2 3 4 5
echo $list[2..4]
echo $list[-3..-1]
echo (count $list[1..2])
set list[2] X
echo $list
set -e list[-1]
echo $list
