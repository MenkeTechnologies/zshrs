set -l t top
set g global
function f
    echo [$t] [$g]
end
f
echo $t
set t changed
echo $t
begin
    set -l t blk
    f
    echo $t
end
echo $t
set -e t
set -q t; echo $status
