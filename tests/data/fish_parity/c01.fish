set -l names alice bob carol
for i in (seq (count $names))
    echo $i: $names[$i]
end
