echo "a b" | string split " " | count
echo a,b,c | string split , | string join +
echo abc | string sub -s 2
echo "x  y" | string replace -r "\\s+" "_"
echo -n abc | string length
printf "l1\nl2\n" | string upper
