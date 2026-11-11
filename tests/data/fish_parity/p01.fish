set a x; echo "[$a[2..-1]]"; echo "[$a[1..-2]]"
set b x y z; echo "[$b[-1..1]]" "[$b[3..1]]" "[$b[2..-2]]" "[$b[1..-4]]" "[$b[-5..2]]"
echo $b[2..] $b[..2] $b[1..10] $b[4..5]
set one x; echo "[$one[2..]]"
echo (count $b[2..-1])
