set s "hello world"
string length $s
string upper $s
string sub -s 1 -l 5 $s
string replace world there $s
string split " " $s
string join - a b c
string trim "  x  "
string match -r "w(or)ld" $s
string repeat -n 3 ab
