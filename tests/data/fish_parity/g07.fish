for i in 1 2 3
    switch $i
        case 1
            continue
        case 3
            break
    end
    echo $i
end
echo done
