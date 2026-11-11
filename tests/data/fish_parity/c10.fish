set -l i 0
while test $i -lt 5
    set i (math $i + 1)
    if test (math "$i % 2") -eq 0
        continue
    end
    echo odd $i
end
