path basename /a/b.txt /c/d
path dirname /a/b.txt
path extension a.tar.gz
path change-extension .md a.txt
path normalize /a/../b//c/.
path sort -r b a c
path filter -f /etc/hosts /nonexistent
path is -d /tmp; echo $status
path basename -E a.txt
path sort --key=basename /z/a /a/z
string shorten -m 5 abcdefghij
string shorten -m 5 -l abcdefghij
string pad -w 6 -r ab
string pad -c x -w 5 ab
string sub -s 2 -l 3 abcdef
string sub -e -2 abcdef
string split -m1 . a.b.c
string split -r -m1 . a.b.c
string split0 </dev/null | count
string collect -a a b
string escape --style=var 'a b-c'
string escape --style=url 'a b/c'
string unescape --style=script 'a\ b'
string repeat -n 3 -m 7 ab
string match -a -r '(\d)(\w)' 1a2b
string match -g -r 'x(\d)' x1 x2
string replace -f -r 'a(.)' '$1$1' abab
string length --visible abc
string trim -c ab -l abcab
string upper -q a; echo $status
string lower ÀB
