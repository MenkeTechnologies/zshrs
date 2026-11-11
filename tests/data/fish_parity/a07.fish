set -g G 1
function f
    set -l L 2
    set G 3
    echo $L $G
end
f
echo $G
set -q L; echo $status
