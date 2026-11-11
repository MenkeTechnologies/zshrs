set -l files a.txt b.txt c.md
for f in $files
    switch $f
        case "*.txt"
            echo txt: $f
        case "*"
            echo other: $f
    end
end
set -l n (count $files)
echo n=$n
