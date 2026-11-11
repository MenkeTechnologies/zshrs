function h
    set -l n 0
    for i in 1 2 3
        set -l n (math $n + $i)
    end
    echo $n
end
h
