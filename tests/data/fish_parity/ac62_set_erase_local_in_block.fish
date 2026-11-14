# a local erased inside a nested block (or behind and/or): $x reads the next
# scope out from then on, and a second erase reaches it
set -g x G
function f
    set -l x L
    if true
        set -e x
    end
    echo "f1 $x"
    set -e x
    echo "f2 $x $status"
end
f
set -g x G
function g
    set -l x L
    for i in 1 2
        echo "g$i $x"
        set -e x
        echo "g$i status $status"
    end
    echo "g3 [$x]"
end
g
set -g x G
function h
    set -l x L
    if true
        set -l x B
        set -e x
        echo "h1 $x"
    end
    echo "h2 $x"
end
h
function k
    set -l x L
    true; and set -e x
    echo "k1 [$x]"
    set -l x again
    echo "k2 $x"
end
k
function m
    set -l x L
    if false
        set -e x
    end
    echo "m1 $x"
end
m
set -l y L
begin
    set -e y
end
echo "top [$y]"
set -g y G2
echo "top2 $y"
