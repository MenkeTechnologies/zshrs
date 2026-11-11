function f
    return 0
end
function g
    f
    echo $status
    false
    return
end
g
echo $status
