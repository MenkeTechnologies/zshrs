test 5 -gt 3 -a 2 -lt 4; echo $status
test -n; echo $status
test -z; echo $status
test a; echo $status
test ""; echo $status
test ! a; echo $status
test ! ""; echo $status
test 1 = 1 -a ! 2 = 3; echo $status
test \( 1 = 1 \) -o 2 = 3; echo $status
test abc = abc; echo $status
test abc != abd; echo $status
test 10 -eq 10.0; echo $status
test 1.5 -gt 1; echo $status
test -e /etc/hosts -a -f /etc/hosts; echo $status
test -d /etc -o -L /nonexist; echo $status
test -s /etc/hosts; echo $status
test -r /etc/hosts -a -w /nonexistent; echo $status
test -x /bin/sh; echo $status
test -h /bin; echo $status
test -p /dev/null; echo $status
test -c /dev/null; echo $status
test -b /dev/null; echo $status
test -S /dev/null; echo $status
test -t 0; echo $status
test /etc/hosts -nt /nonexistent; echo $status
test /etc/hosts -ef /etc/hosts; echo $status
test 1 -eq a 2>/dev/null; echo $status
test 1 -ne 2 -a 3 -ge 3 -a 4 -le 4; echo $status
test -n "a b"; echo $status
test x -a y; echo $status
test x -o ""; echo $status
test -f; echo $status
test -f -a -d; echo $status
test --version 2>/dev/null; echo $status
[ 1 -eq 1 ]; echo $status
[ -n abc ]; echo $status
[ a = b ] || echo no
[ ! -e /nonexistent ]; echo $status
test 0x10 -eq 16; echo $status
test -1 -lt 0; echo $status
test 2 -gt -1; echo $status
test "" = ""; echo $status
test = =; echo $status
test -n -n; echo $status
set -l a 5; test $a -gt 3 -a $a -lt 10; echo $status
set -l e; test -n "$e"; echo $status
test -n $e; echo $status
test -z $e; echo $status
test $e = ""; echo $status
