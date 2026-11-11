set -l n 10
while true
    set n (math $n - 3)
    if test $n -lt 0
        break
    end
    echo $n
end
echo final $n
