function maybe
    if test (count $argv) -eq 0
        echo none
        return 1
    end
    echo $argv[1]
end
maybe; echo $status
maybe x; echo $status
