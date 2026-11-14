# variables named at run time see the same scopes as the static names
set n x
set -l x L
set -g x G
echo "1 $x $$n"
function show; echo "show $x"; end
show
set $n V
echo "2 $x"
show
set -q $n; echo "3 $status"
set -e $n; echo "4 $status [$x]"
show
set -e $n; echo "5 $status [$x]"
set -e $n; echo "6 $status"
function f
    set -l nm y
    set -l y fl
    set -g y fg
    echo "f1 $y $$nm"
    set $nm fv
    echo "f2 $y"
    g
    set -e $nm
    echo "f3 $y"
end
function g; echo "g $y"; end
f
echo "top $y"
begin
    set -l z B
    set m z
    echo "b1 $$m"
end
echo "b2 [$z] [$$m]"
set -g w WG
function h
    set -l w WL
    eval 'echo h1 $w'
    set k w
    echo "h2 $$k"
    set -g $k G3
    echo "h3 $w $$k"
end
h
echo "h4 $w"
