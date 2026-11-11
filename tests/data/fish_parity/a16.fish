echo (printf "%s\n" a b c | string upper)
printf "%s\n" 1 2 3 | while read -l line
    echo got $line
end
