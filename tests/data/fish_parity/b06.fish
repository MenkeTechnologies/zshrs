echo one | read -l a
echo "[$a]"
printf "x y z\n" | read -l p q r
echo "[$p][$q][$r]"
printf "l1\nl2\n" | while read -l ln; echo ">$ln<"; end
