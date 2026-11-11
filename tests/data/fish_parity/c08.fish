set -l a 1
set -l b 2
test $a -lt $b; and echo lt
test $a -eq 1 -o $b -eq 5; and echo or
test ! $a -eq 2; and echo not
[ $a -eq 1 ]; and echo bracket
