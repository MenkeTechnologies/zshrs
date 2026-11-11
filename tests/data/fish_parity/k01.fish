set -l v /nonexistent_zz/zz*
count $v < /dev/null
set -g y $PATH/zz-nomatch-*
echo (count $y)
function f
    set -l g /nonexistent_zz/*
    count $g < /dev/null
end
f
echo after
