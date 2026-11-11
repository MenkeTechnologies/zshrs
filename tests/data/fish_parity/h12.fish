set -l x one
set -l y two
echo "$x $y" | read -l a b
echo $b $a
echo $x$y
echo "$x"$y
echo {$x,$y}
echo pre{$x,$y}post
