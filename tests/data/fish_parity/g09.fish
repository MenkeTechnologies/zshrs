string pad -c "*" -w 6 ab
string pad -r -c "-" -w 6 ab
string shorten -m 4 abcdefgh
string shorten -m 10 abc
string trim -r "x  "
string trim -c x xxaxx
string escape --style=var "a b"
string split -r -m1 / a/b/c
string split --max 1 , a,b,c
string replace -a "" x abc 2>/dev/null; echo $status
