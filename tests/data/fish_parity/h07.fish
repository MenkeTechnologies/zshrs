set -l a (printf "b\na\nc\n" | sort)
echo $a
set -l b (printf "3\n1\n2\n" | sort -n | head -2)
echo $b
printf "%s\n" $a | grep -c .
