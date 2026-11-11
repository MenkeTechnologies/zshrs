set -g v a b
set -gx v c
echo $v
function f
    set -g w x y
    set -x w z
    echo $w
    set -lx v --all
    echo $v
end
f
echo $v $w
sh -c 'echo $v/$w'
