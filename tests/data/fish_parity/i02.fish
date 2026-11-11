read -l x < /dev/null
echo "[$x]"
function f; echo [$x]; end
f
printf "v\n" | read -l y
echo $y
