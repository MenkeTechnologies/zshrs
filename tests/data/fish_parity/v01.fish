echo - | string replace -- "" ","
echo -
echo - x
printf "1\nHUP\n" | read --line number name; echo "[$number][$name]"
printf "a\n" | read --line only; echo "[$only]"
printf %s\\n {+,-}{a\t'always x',b\tverb}
string split -f 1,4 : -- "admin:*:80:root" | string join \t
string split -f 1,4 : -- "##"; echo $status
string split -f 2-3 : -- "a:b:c:d" | string join ,
string split -f 2 : -- "a:b:c"
for f in u P
    set -q _flag_$f; and echo set:$f
end
set -g _flag_P 1
for f in u P
    set -q _flag_$f; and echo again:$f
end
