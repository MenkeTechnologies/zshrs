function f
    set -l a 1
    if true
        set -l a 2
        echo inner $a
    end
    echo outer $a
end
f
set -l t 1
if true
    set -l t 2
    echo blk $t
end
echo top $t
