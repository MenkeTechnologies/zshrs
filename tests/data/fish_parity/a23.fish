set -l list one two
for i in $list
    echo (string upper $i)
end
echo $list[1]$list[2]
