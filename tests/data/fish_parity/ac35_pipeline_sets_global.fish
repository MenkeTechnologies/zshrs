function g; set -g zg 1; echo from-g; end; g | cat; set -q zg; echo $status
function g2; echo a; echo b; end; g2 | string upper
echo x | cat | cat
set -e zg; function g3; set -g zg 7; echo three; end; g3 | read -l v; echo $v $zg
ls /nonexistent_zz 2>&1 | count
echo one | string upper
function g; set -g zg 1; end; g 2>/dev/null | cat; set -q zg; echo $status
function g2; set -g zh 1; end; function h; g2; end; h | cat; set -q zh; echo $status
function h2; g2; echo out; end; echo (h2); echo $zh
function nog; echo plain; end; function callsnog; nog; end; callsnog | cat; echo (callsnog)
