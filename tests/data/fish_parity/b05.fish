function fact -a n
    if test $n -le 1
        echo 1
    else
        math $n \* (fact (math $n - 1))
    end
end
fact 5
