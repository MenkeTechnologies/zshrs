set -l a
count $a
set -l b ""
count $b
echo (count $a) (count $b)
test -z "$a"; echo $status
test -n "$b"; echo $status
set -l c "" ""
count $c
