set -l top 1
function h; set -l; echo --; set -l | string match -v argv; end
function g; set -l b 2; h; set -l; end
g
function k; set -l a 1; if true; set -l c 3; end; set -l; end; k
function m; set -l x 5; set -l; end; m; m a; m a "b c"
