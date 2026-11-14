# set's scope flags: a script local beside a global of the same name, erase and query
# per scope, function scope (-f) and the unscoped set in a function
function show
    echo "show[$x]"
end
set -l x loc
set -g x glob
echo "[$x]"
show
set -q -g x; echo "q-g $status"
set -q -l x; echo "q-l $status"
set -g -a x more
echo "[$x]"
show
set -e x; echo "e $status [$x]"
set -e x; echo "e $status [$x]"
set -e x; echo "e $status"
set -l y loc
set -g y glob
set -e -g y; echo "e-g $status [$y]"
set -e -l y; echo "e-l $status [$y]"
set -e -l y; echo "e-l $status"
if true
    set -l z blk
    set -g z glob
    echo "blk[$z]"
end
echo "top[$z]"
function f1
    set v inner
    echo "f1[$v]"
end
f1
echo "after f1 [$v]"; set -q v; echo "q $status"
set -g w g0
function f2
    set w changed
    set -a w more
end
f2
echo "w[$w]"
function f3
    set -l a one
    set -g a two
    echo "f3[$a]"
    show_a
end
function show_a
    echo "show_a[$a]"
end
f3
echo "a[$a]"
function f4
    if true
        set -l b one
        set -f b two
        echo "inner[$b]"
    end
    echo "f4[$b]"
end
f4
function rec
    set depth $argv[1]
    if test $argv[1] -lt 2
        rec (math $argv[1] + 1)
    end
    echo "rec $argv[1] depth=$depth"
end
rec 0
