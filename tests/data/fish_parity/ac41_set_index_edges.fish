# set NAME[...] and set -e NAME[...]: padding, negative and out-of-range indexes,
# ranges either way, errors and their status
set x[1 3] N1 N2; echo (count $x) $x
set -e x
set y[-1] NEW 2>/dev/null; echo "st $status"; count $y
set e
set -e e[2]; echo "st $status"; count $e
set -e nope[2]; echo "st $status"
set l a b c
set -e l[5]; echo "st $status" $l
set -e l[-5]; echo "st $status" $l
set l a b c; set -e l[3..1]; echo "st $status"; count $l
set l a b c; set -e l[-2..]; echo $l
set l a b c d e; set -e l[1 -1]; echo $l
set l a b c; set l[5] E; printf '<%s>' $l; echo
set l a b c; set l[-1] Z; echo $l
set l a b c; set l[-4] Z 2>/dev/null; echo "st $status" $l
set l a b c; set l[2..3] P 2>/dev/null; echo "st $status" $l
set l a b c; set l[3..2] P Q; echo $l
set l a b c; set l[-1..1] P Q R; echo $l
set l a b c; set l[2..] P Q; echo $l
set l a b c; set l[1 1] P Q; echo $l
set l a b c; set -a l[2] P 2>/dev/null; echo "st $status" $l
set l a b c; set l[2x] P 2>/dev/null; echo "st $status" $l
set -l s a b c
set -g s[2] G
echo $s
function f
    printf 'f<%s>' $s; echo
end
f
set -e -g s[2]; echo "st $status"
set -e -g s[9]; echo "st $status"
