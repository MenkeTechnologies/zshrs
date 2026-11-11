function rec -a n
    if test $n -le 0
        return
    end
    echo $n
    rec (math $n - 1)
end
rec 3
