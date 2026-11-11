while true
    echo once
    break
end
for i in 1 2 3
    if test $i = 2; break; end
    echo $i
end
