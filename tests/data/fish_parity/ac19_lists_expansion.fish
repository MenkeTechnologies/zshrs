set l 1 2 3 4 5
echo $l[2..4] $l[-2..-1] $l[4..2] $l[1 3]
echo $l[(count $l)]
echo (count $l) (count) (count a b)
set -q l[2]; echo $status
set -q l[9]; echo $status
set l[2] x; echo $l
set l[7] y; echo $l
set -e l[1]; echo $l
echo $l[10]
echo "$l[1]-$l[2]"
echo {a,b}{1,2}
echo {a,b}$l[1]
echo (echo a b | string split ' ')[2]
set x (printf 'a\nb\n'); echo (count $x)
set y "$(printf 'a\nb\n')"; echo (count $y)
echo $x[1]$x[2]
set -l z; echo (count $z)
echo $z[1]x
echo "a$z"b
echo ~ | string match -q "$HOME"; echo $status
echo ~root
echo *.nomatch; echo $status
echo "*.nomatch"
contains -i b a b c
contains -- -x -x; echo $status
contains x; echo $status
random choice a; 
set s 'a b'; echo $s[1] (count $s)
for i in (seq 3 -1 1); echo $i; end
seq 1 0.5 2
seq 3
seq 2 4
echo (seq 5)[2..3]
