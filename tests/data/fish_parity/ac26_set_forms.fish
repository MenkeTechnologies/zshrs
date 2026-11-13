set -q PATH; echo $status
set -q PATH[1] PATH[2]; echo $status
set -q undefined1 undefined2; echo $status
set -q HOME undefined1; echo $status
set -l a 1 2 3; set -q a[2..3]; echo $status
set -q a[4]; echo $status
set -s a 2>&1 | head -3
set -l b; set -S b
set -g c x y; set -S c
set --show c
set -x D 1; set -S D
set -gx E 5 6; set -S E
set --names | string match -q '*c*'; echo $status
set -n | count | string match -qr '^\d+$'; echo $status
set -e c; set -q c; echo $status
set -e nonexist; echo $status
set a; echo "[$a]" (count $a)
set a b c; set a[1] z; echo $a
set a[-1] y; echo $a
set a[5] w 2>&1; echo $a
set -a a q; echo $a
set -p a p; echo $a
set a[2..3] m n; echo $a
set --append a r s; echo $a
set -l -a a2 1; set -l -a a2 2; echo $a2
set -ql a2; echo $status
set x 1; set -l x 2; echo $x; set -ql x; echo $status
begin; set -l x 3; echo $x; end; echo $x
function f; set x 4; echo $x; end; f; echo $x
set -g y1 1; function g; set y1 2; end; g; echo $y1
function h; set -l y2 1; function i; set -q y2; echo $status; end; i; end; h
set i 1 2 3; set -e i[2]; echo $i
set -e i[1 2]; echo $i
set --erase --global i
set fish_user_paths /a /b; echo $fish_user_paths; set -e fish_user_paths
set PATH /usr/bin /bin; echo $PATH; echo "$PATH"; echo (count $PATH)
set -x PATH $PATH /tmp/xx; env | string match -r '^PATH=.*xx$'
set -l p1 a:b:c; echo $p1; set -l p2 (string split : $p1); echo $p2[2]
set SHELL_X (echo a b); echo $SHELL_X[2]
set VAR 'a  b'; echo "$VAR" | string length
set 'weird name' 1 2>&1 | head -2
set -q -- -x; echo $status
