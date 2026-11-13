function g; set -g zg 1; echo from-g; end; g | cat; set -q zg; echo $status
function g2; echo a; echo b; end; g2 | string upper
echo x | cat | cat
set -e zg; function g3; set -g zg 7; echo three; end; g3 | read -l v; echo $v $zg
ls /nonexistent_zz 2>&1 | count
echo one | string upper
