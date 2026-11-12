echo "3 4" | read -l a b
echo $a/$b
function f
    echo "5 6" | read -l x y
    echo $x/$y
end
f
echo "7" | read p; echo $p
set -l c ""
switch $c
    case ""
        echo empty
    case "*"
        echo other
end
switch x
    case ""
        echo no
    case x
        echo yes
end
set -U | count | string match -qr "^\\d+\$"; echo $status
