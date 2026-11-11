set -l n 0
while test $n -lt 3
    set n (math $n + 1)
    echo n=$n
end
