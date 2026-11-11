function t
    for i in 1 2 3
        if test $i -eq 2
            return 7
        end
    end
end
t
echo $status
function u; echo $status; end
false; u
