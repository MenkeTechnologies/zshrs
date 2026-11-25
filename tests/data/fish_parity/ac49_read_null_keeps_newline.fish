printf 'a\nb\n' | read -z v; echo "[$v]"
printf 'a\0b\0' | while read -z -l v; echo "<$v>"; end
printf 'a\nb' | read -z v; echo "[$v]" $status
printf a | read v; echo $status $v
printf '' | read v; echo $status
printf 'a b' | read x y; echo $status $x $y
printf 'a b' | read -a l; echo $status $l
echo a | read v; echo $status
read -l w </dev/null; echo $status
