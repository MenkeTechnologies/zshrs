# a scopeless set in a function updates a global that exists by then, even when
# the first such set written in the function never ran
function f
    if false
        set -a q skipped
        return
    end
    set -g q
    set -a q a b
    echo "f1 $q"
    set -p q z
    echo "f2 $q"
end
f
echo "top $q"
function g
    if false
        set r x
    end
    set -g r G
    echo "g1 $r"
    set -e r
    echo "g2 $status [$r]"
    set -e r
    echo "g3 $status"
end
g
echo "top [$r]"
function h
    if false
        set s x
    end
    set s L
    echo "h1 $s"
end
h
set -q s; echo "top s $status"
