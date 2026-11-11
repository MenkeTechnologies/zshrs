set s abc
echo (string sub -s 2 $s)
echo (string length -q $s; echo $status)
set -l e ""
test -z "$e"; echo $status
test -n "$e"; echo $status
if test -z "$undefined"; echo empty; end
