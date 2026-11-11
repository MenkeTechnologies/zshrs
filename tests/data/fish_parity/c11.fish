set x 1
function f
    set x 2
end
f
echo $x
function g
    set -l x 3
end
g
echo $x
