string match -n b abc
string match -n -a b abcb
string match --index -r "b+" abbc
string split0 < /dev/null | count
printf "a\0b\0" | string split0
string join0 a b | string split0
string sub --start=2 --length=2 abcdef
string replace --all --regex "[aeiou]" "" education
string match --regex --all "\\d" a1b2
string length --quiet ""; echo $status
