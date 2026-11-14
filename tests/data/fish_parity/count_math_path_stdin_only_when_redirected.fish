# count, math and path read stdin only when their own stdin is redirected (not first in
# the pipeline, or a < of their own); inside a block or command substitution they do not.
echo a | count x y
echo a | begin
    count
    set -l n (count)
    echo cmdsub $n
end
printf 'a\nb' | count
count a b </dev/null
echo 1 | math 2
echo 3 | builtin math 4
math
echo math $status
echo /a/b | path basename /c/d
echo path $status
echo a.txt | path change-extension md
path change-extension md
echo chext $status
echo x | begin
    path basename
    echo path-block $status
end
