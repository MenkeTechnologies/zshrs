set -l list a b c
echo $list[2..-1]
echo $list[1..2]
echo $list[1 3]
echo $list[-2]
echo (count $list[2..3])
echo $list[4]x
echo "[$list[4]]"
