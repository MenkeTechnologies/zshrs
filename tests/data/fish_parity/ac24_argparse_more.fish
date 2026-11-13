function t6; argparse -x r,q 'r/req' 'q/quiet' -- $argv; echo s=$status; end; t6 -r; t6 -r -q
function t3; argparse --min-args=1 --max-args=2 'a' -- $argv; echo s=$status; end; t3; t3 x; t3 x y z; t3 -a
function t3b; argparse -N 2 -X 3 'a' -- $argv; echo s=$status; end; t3b x; t3b x y;
function t4; argparse 'a=!test $_flag_value -gt 3' -- $argv; echo s=$status a=$_flag_a; end; t4 -a 5; t4 -a 1
function t8; argparse 'V#version' 'x' -- $argv; echo s=$status v=$_flag_V argv=$argv; end; t8 -5 -x; t8 -x -12 foo; t8 --version=3; t8 -5 -3
function t9; argparse 'x/xx' -- $argv; echo s=$status; end; t9 -x -x
function t11; argparse 'n/name=?' -- $argv; echo "s=$status [$_flag_n] c=(count $_flag_n)" argv=$argv; end; t11 -n x; t11 -nx; t11 -n; t11 --name; t11 --name=y; t11 --name y
function t1; argparse -n myf 'h/help' 'v/verbose' 'o/out=' 'n/num=?' 'x/exc=+' -- $argv; or return; echo h=$_flag_h v=$_flag_v o=$_flag_o n=$_flag_n x=$_flag_x argv=$argv; end
t1 -h -v -o file --num=3 -x a -x b rest more
t1 --out=f -vh -- -z
t1 --bogus; echo s=$status
t1 -o; echo s=$status
function t2; argparse --ignore-unknown 'a' -- $argv; echo a=$_flag_a argv=$argv; end; t2 -a -q x
function t3; argparse --min-args=1 --max-args=2 'a' -- $argv; echo s=$status; end; t3 2>/dev/null; t3 x; t3 x y z 2>/dev/null
function t4; argparse 'a=!test $_flag_value -gt 3' -- $argv; echo s=$status a=$_flag_a; end; t4 -a 5; t4 -a 1 2>/dev/null
function t5; argparse -s 'a' 'b' -- $argv; echo a=$_flag_a b=$_flag_b argv=$argv; end; t5 -a x -b
function t7; argparse 'long-only' -- $argv; echo $_flag_long_only; end; t7 --long-only
function t9; argparse 'a/aa' -- $argv; echo $_flag_a $_flag_aa; set -q _flag_a; echo q=$status; end; t9 --aa; t9
function t10; argparse -i 'a' -- $argv; echo $argv; end; t10 -a -b -c d
function t11; argparse 'a/aa=+' -- $argv; echo (count $_flag_a); end; t11 -a 1 -a 2 -a 3
string match -r '(?<year>\d+)-(?<mon>\d+)' 2024-05; echo $year $mon
string match -rg '(\w)(\d)' a1 b2
string match -ri 'ABC' abc
string match -e b abc xbz
string match -v a a b c
string match -n b abc
string match --index b abc
string match '*.fish' a.fish b.txt
string match -r '^(a)?b' b; echo $status
string replace -a ab X ababab
string replace -r '(a+)' '[$1]' baab
string replace -ri A x banana
string replace -r -a '\s+' ' ' 'a   b    c'
string replace '' X abc
string replace -r '^' X abc
string replace -r '$' X abc
string replace -r '(?<n>\d)' '<$n>' a1
string split '' abc
string split -f2 , a,b,c
string split -n , ',a,,b'
string split -r , a,b,c
string join / a b c
string join '' a b
string join0 a b | string escape
string length '' a abc
string pad -w 3 a ab abc abcd
string trim -r '  a  ' | string escape
string trim -- ' a '
string repeat -n 0 a
string repeat -n 2 -N ab; echo
string sub -s -2 abcd
string sub -l 0 abcd
string sub -s 5 abcd
string upper (echo a b)
string lower A B
string unescape 'a\ b\n'
string escape (printf 'a\nb')
string collect < /dev/null | count
printf 'a\nb\n' | string collect | count
printf 'a\nb' | string collect -N | string escape
string match -- -a -a
string length -- -a
