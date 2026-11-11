set -l f (mktemp)
printf "l1\nl2\nl3\n" > $f
while read -l line
    echo got:$line
end < $f
cat < $f | wc -l | string trim
rm $f
