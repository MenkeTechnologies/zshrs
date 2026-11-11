function f
    echo $argv
    echo (count $argv)
    set -l first $argv[1]
    set -e argv[1]
    echo $first / $argv
end
f 1 2 3
f "a b" c
