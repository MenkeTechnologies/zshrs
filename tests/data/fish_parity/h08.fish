set -g g1 outer
function f
    set -l g1 inner
    g
end
function g
    echo $g1
end
f
echo $g1
