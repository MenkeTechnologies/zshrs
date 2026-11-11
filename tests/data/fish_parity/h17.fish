set -l n 0
function incr -S
    set n (math $n + 1)
end
incr; incr
echo $n
