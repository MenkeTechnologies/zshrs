echo a >/dev/null 2>&1; echo $status
echo err 2>&1 >/dev/null | string length
begin; echo o; echo e >&2; end 2>/dev/null
echo hi >&2 2>/dev/null
echo x &>/dev/null; echo $status
echo y >>/dev/null
cat </dev/null; echo $status
echo a | tee /dev/null | string upper
echo a >? /dev/null 2>&1
set f (mktemp); echo one > $f; echo two >> $f; cat $f | count; rm -f $f
echo (echo a; echo b) | count
echo "(echo hi)"
echo '$HOME'
echo "$HOME" | string length -q; echo $status
echo \$HOME \\ \a | string escape
echo "a\nb" | count
echo 'a\nb' | count
echo "\x41"
echo \x41B\U00000043
echo -e 'a\tb' | string escape
echo -n a; echo b
echo -s a b
echo -- -n
echo -e '\x41\0102'
echo a\
b
echo "multi
line" | count
echo ab{c,d}ef
echo {1..3}
echo {a..c}
echo x{}y
echo {a}
echo {,a}
echo (printf '%s\n' c a b | sort)
echo $argv | count
echo foo(echo bar)baz
echo "foo"(echo bar)
set -l e; echo "[$e]" "[$e[1]]"
echo [a-c]
echo ?
echo '*'
echo *nope* 2>/dev/null; echo $status
echo (true)(false) | count
set v 'a  b'; echo $v; echo "$v"; echo $v[1]
set -q HOME[1]; echo $status
echo $HOME[1] | string length -q; echo $status
set -x ENVT 1; env | string match -q 'ENVT=1'; echo $status
set -gx PATHX /a /b; env | string match 'PATHX=*'
set -l list a b; echo $list[1]$list[2]
echo $list[1]$list
echo "$list"
echo "$list[1]"
echo $list" "x
count $list $list
