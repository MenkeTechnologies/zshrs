for i in (seq 3)
    if test $i -eq 2
        continue
    end
    echo $i
end
