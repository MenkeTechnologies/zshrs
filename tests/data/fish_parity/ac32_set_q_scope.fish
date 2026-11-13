set -l a2 1; set -qg a2; echo $status
set -g g2 1; set -qg g2; echo $status
function f; set -l loc 1; set -qg loc; echo $status; set -ql loc; echo $status; set -g gl 2; set -qg gl; echo $status; end; f
set -qU nonexist_u; echo $status
set -q a2; echo $status
