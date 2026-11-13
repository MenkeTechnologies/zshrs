function f --argument-names a b; echo $a $b (count $argv); end; f 1 2 3
function g --description 'desc' --inherit-variable HOME; echo ok; end; g
functions -D g | path basename
functions -v f | string match -r 'f$' | count
function h; return 3; end; h; echo $status
function i; set -q argv[1]; or return 1; echo $argv[1]; end; i; echo $status; i z
begin; set -l q 1; echo $q; end; set -q q; echo $status
if true; echo a; else if false; echo b; else; echo c; end
if false; echo a; else if true; echo b; else; echo c; end
while true; break; end; echo after
set n 0; while test $n -lt 3; set n (math $n + 1); continue; end; echo $n
for x in a b c; test $x = b; and continue; echo $x; end
switch foo; case 'f*'; echo glob; case foo; echo exact; end
switch 5; case 1 2 3; echo low; case '*'; echo hi; end
echo a | read -l v; echo $v
echo 'a b c' | read -l x y; echo $x/$y
echo 'a b c' | read -la arr; echo $arr[2]
printf 'l1\nl2\n' | while read -l line; echo "[$line]"; end
echo abc | read -n 2 -l p; echo $p
read -l -P '' z < /dev/null; echo $status
echo x | read --delimiter , -l w; echo $w
echo 'a,b' | read -d , -l a1 a2; echo $a1 $a2
test -n ""; echo $status
test 1 -eq 1 -a 2 -gt 1; echo $status
test -z "" -o -f /nonexistent; echo $status
[ a = a ]; echo $status
test "a" = "b"; echo $status
test ! -e /nonexistent; echo $status
set -q undefined_x; echo $status
not false; echo $status
false; or echo or-ran
true; and echo and-ran
command -q ls; echo $status
type -q ls; echo $status
type -t f; type -t if; type -t nothing_x
builtin -n | count | string match -qr '^\d+$'; echo $status
status is-interactive; echo $status
status is-login; echo $status
echo $status_generation | string match -qr '^\d+$'; echo $status
echo $_
echo (status current-command)
echo $fish_pid | string match -qr '^\d+$'; echo $status
echo $version | string match -qr '^\d'; echo $status
echo $SHLVL | string match -qr '^\d+$'; echo $status
echo $PWD | string match -q "*/*"; echo $status
echo $USER | count
echo $hostname | count
echo $CMD_DURATION
echo $COLUMNS $LINES
echo "sg=$status_generation"
echo "cd=$CMD_DURATION"
echo "u=$_"
type -t if
type -t for
type -t begin
type -t function
echo 'a b c' | read -la arr; echo $arr[2]
echo 'a b c' | read -a arr; echo $arr[2]
echo 'a b c' | read -l -a arr; echo $arr[2] (count $arr)
