# An index past either end of a list selects nothing: unquoted it is no word at all,
# quoted it is one word joined from only the elements that exist.
set e
set s one two three
echo a $e[-2] /
echo b $e[1 3] /
echo c "$e[1 3]" /
echo d $s[1 9 2] / "$s[1 9 2]" / "$s[9 9]" /
echo e $u[1] / "$u[1]" / x$e[1]y /
set n (count $s[5] $e[1] $s[-4]); echo f $n
for w in $s[7] $s[-1] $e[2]; echo g $w; end
function f
    echo h [$argv[-1]] $argv[-1] / "[$argv[-1]]" / $argv[2..] /
end
f; f a; f a b c
echo i (printf '%s\n' p q)[1 5 2] / (printf '%s\n' p)[3] /
set -g P2PATH /c /d
echo j $P2PATH[3] / "$P2PATH[2 3 1]" /
false; set y $status $s[2] $e[1]; echo k $y
false; echo l $status $s[1] $s[9]
