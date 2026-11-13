function foo --on-event myev; echo got $argv; end; emit myev 1 2
function bar --on-signal USR1; echo usr1; end; kill -USR1 $fish_pid; sleep 0.1
function w --wraps ls --description d; end; functions -q w; echo $status
function cleanup --on-process-exit $fish_pid; echo bye; end
abbr -a gco 'git checkout'; abbr --show | string match -q '*gco*'; echo $status
abbr -l | string match gco
abbr -q gco; echo $status
abbr -e gco; abbr -q gco; echo $status
alias ll 'ls -l'; alias ll | string match -q '*ls -l*'; echo $status
alias | string match -q 'alias ll*'; echo $status
functions -e ll; functions -q ll; echo $status
function a1; end; function a2; end; functions -n | string match -q a1; echo $status
functions --copy a1 a3; functions -q a3; echo $status
functions -a | count | string match -qr '^\d+$'; echo $status
set -l c 0; function inc; set c (math $c + 1); end; inc; echo $c
function lv; set -l z 1; function inner; echo $z; end; inner; end; lv
function sv; set -g gg 5; end; sv; echo $gg
function sc; set -e gg; end; sc; set -q gg; echo $status
function ar -a x y; echo $x $y; end; ar 1; ar 1 2 3
function dflt; echo (count $argv); end; dflt; dflt ''; dflt a b
function rets; return; end; rets; echo $status
function rs; false; return; end; rs; echo $status
function nested; function innerf; echo i; end; end; nested; innerf
function sh2; status current-function; end; sh2
function sf; status function; end; sf 2>/dev/null
function rec; set -l n $argv[1]; test $n -le 0; and return; echo $n; rec (math $n - 1); end; rec 3
function varcheck; set -q argv[2]; and echo two; or echo notwo; end; varcheck a; varcheck a b
function pipe_f; read -l x; echo got $x; end; echo hello | pipe_f
function status_f; true; end; status_f; echo $status
echo (function ff; end; type ff | head -1)
