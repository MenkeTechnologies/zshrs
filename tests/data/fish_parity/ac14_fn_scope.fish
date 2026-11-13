function f; for i in a b; end; argparse x 'y/yy=' -- -x --yy=3 z; echo $_flag_x $_flag_yy $argv; end
f; set -q i; and echo i-leak; set -q argv_opts; and echo leak2; set -q _flag_x; and echo leak3
set -g j 0
function g; for j in a b; end; end; g; echo j=$j
echo 'set -l zz 1; echo $zz' > zz.fish
source zz.fish; set -q zz; and echo leak
set -g | string match 'IFS*'
set -g | string match '__fish_cache*'
for q in 1 2; end; echo q=$q
function h; for k in 1 2; echo $k; end; for k in 3; echo $k; end; end; h; set -q k; and echo kleak
