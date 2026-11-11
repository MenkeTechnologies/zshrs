for i in 1 2
    set -l v $i
    echo v=$v
end
echo (set -q v; echo $status)
begin
    set -l q 5
    while test $q -gt 3
        set q (math $q - 1)
        echo q=$q
    end
end
set -q q; echo $status
