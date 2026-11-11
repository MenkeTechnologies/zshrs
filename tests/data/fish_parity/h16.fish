begin
    set -l inner 5
    echo $inner
end
echo [$inner]
if true
    set -l blk 1
end
echo [$blk]
