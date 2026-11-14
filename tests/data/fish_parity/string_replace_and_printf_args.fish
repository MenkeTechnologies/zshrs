# a replacement naming a group the pattern lacks is a substitute error, status 2
string replace -r '' '$2_$1' 'a b' c; echo $status
string replace -r 'b' '$2' a b; echo $status
string replace -r z '$2_$1' abc; echo $status
string replace -r '(a)' '$1b' a; echo $status
string replace -r '(a)' '${1}0' a; echo $status
string replace -r '(a)|(b)' '[$2]' a; echo $status
string replace -r a 'x$-' a; echo $status
# -i lowercases the text before each match (replace.rs StringReplacer::Literal)
string replace -i o '$2_$1' Hello_World; echo $status
string replace -ia O X HELLO_World; echo $status
string replace -i z X HELLO_World; echo $status
string replace -m 1 a b a a a; echo $status
# printf %c of an empty or missing argument is a NUL (printf.rs)
printf '[%c][%c]' x | od -An -c
printf '[%3c][%-3c]\n' '' '' | od -An -c
printf '%03d %x|' '' 4 ''; echo $status
printf '%5.2f|' 4 ''; echo $status
printf '%d|%d|' 1 x 3; echo $status
