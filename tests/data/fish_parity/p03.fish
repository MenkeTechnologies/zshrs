function f
    set -lx -a PATH /a /b
    echo $PATH[-2..]
end
f
echo $PATH[-1] | string match -q /b; echo $status
function g; set -lx -p PATH /pre; echo $PATH[1]; end
g
echo $PATH[1] | string match -q /pre; echo $status
