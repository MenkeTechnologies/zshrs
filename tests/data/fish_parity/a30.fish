function f; echo $argv[1]; echo $argv[-1]; end
f x y z
set -l idx 2
set -l arr p q r
echo $arr[$idx]
