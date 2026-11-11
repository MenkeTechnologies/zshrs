test -n; echo $status
test -z; echo $status
test; echo $status
test "a" = "a" -a "b" = "b"; echo $status
test 3 -ge 3; echo $status
test -e /; echo $status
test ! -e /nonexistent; echo $status
[ -f /etc/hosts ]; echo $status
test "abc" = "abc"; echo $status
test 1 -eq 2 -o 2 -eq 2; echo $status
