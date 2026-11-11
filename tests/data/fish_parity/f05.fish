echo one two | read -l x y
echo "$x|$y"
echo "  padded  " | read -l z
echo "[$z]"
printf "1\n2\n3\n" | while read -l n
    math $n \* 2
end
