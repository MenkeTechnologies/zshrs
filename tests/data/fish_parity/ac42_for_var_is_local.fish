for q in a; set -ql q; echo $status; end
set -ql q; echo $status
bind -L
function f; for z in 1; set -ql z; echo $status; end; end; f
set -g gg 1; for i in 1; set -ql gg; echo $status; end
