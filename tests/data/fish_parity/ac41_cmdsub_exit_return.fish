function f
    echo (return 4)
    echo not-reached
end
f
echo status $status
function g
    set -l v (echo a; return 2; echo b)
    echo "v=$v"
end
g
echo status $status
echo (false; echo x) $status
set x (echo out; exit 3)
echo not-reached-either
