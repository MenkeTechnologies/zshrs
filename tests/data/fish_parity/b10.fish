set v 5
if test $v -gt 3 -a $v -lt 10
    echo in-range
end
if test "$v" = 5; echo eq; end
if test -d /tmp; echo dir; end
if not test -f /nonexistent; echo nofile; end
test "a" != "b"; and echo ne
