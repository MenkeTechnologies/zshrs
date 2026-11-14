# string repeat -n/-m/-N and positional count (repeat.rs)
string repeat -n 0 -m 5 -- a; echo $status
string repeat -n 0 -- a; echo $status
string repeat -m 5 -- ab; echo $status
string repeat -n 2 -m 0 -- ab; echo $status
string repeat -n 3 -N -- ab x; echo $status
string repeat -n 2 '' ''; echo $status
string repeat 3 xy; echo $status
# string split -f: a missing field ends the command with status 1 (split.rs)
string split -f1,3 -- ' ' 'x y'; echo $status
string split -f1,3 -- '' 42; echo $status
string split -f1,3 -- ' ' 'x y z' 'p q'; echo $status
string split -f3-1 , a,b,c; echo $status
string split -a -f1,3 ' ' 'x y'; echo $status
string split -m1 -f2 , a,b,c; echo $status
printf 'a\0' | string split0; echo $status
