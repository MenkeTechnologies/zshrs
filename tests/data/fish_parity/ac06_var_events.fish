set -x --path FOO a b c
echo $FOO
env | grep '^FOO='
set -S FOO
function h --on-variable bar; echo "bar: $argv"; end
set bar 1
set bar 2
set -e bar
function g --on-event myev; echo "ev $argv"; end
emit myev x y
