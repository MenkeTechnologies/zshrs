# string shorten and string length -V skip terminal escape sequences; string unescape
# decodes fish's script escapes.
set -l red (printf '\e[31m')
set -l off (printf '\e[0m')
string shorten -m 3 $red"abcdef"$off
string shorten -m 4 -l $red"abcdef"$off
string shorten $red"xy"$off abcdef
string shorten -m 10 ab$off
string shorten -m 3 (printf 'a\e]0;title\abcdef')
string length -V $red"abc"$off 'ab
cde' x\ryz '' 日本
echo $status
string length -V ''
echo $status
string unescape 'a\nb' '\x41é' "'q u o'" '"d\$q"' '\101' '\cA' 'bad\q' '\xc3\xa9' "'open" '\x7f' 'a"b"c'
echo $status
for s in 'end\\' '"dq\\' '\x' '\u' '\U110000' '\Ufdd0' '\c' '\777'
    string unescape $s
    echo $status
end
