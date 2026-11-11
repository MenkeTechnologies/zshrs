set -l a x y z
echo (count $a) $a[1] $a[-1]
set -l idx (contains -i y $a)
echo idx=$idx
if contains -- z $a
    echo has-z
end
echo (string join , $a)
printf "%s|" $a; echo
printf "%d %s\n" 5 five 6 six
