function f; argparse "l/long=+&" -- $argv; for l in $_flag_l; end; end; f -l x; set -q l; and echo leak1
function g; set -l tokens a b; for t in $tokens; for l in x; if true; return 0; end; end; end; end; g; set -q t; and echo leak2
set v a b; switch b; case $v; echo m2; end
set v "a*" b; switch abc; case $v; echo m3; end
set v a; switch x; case $v; echo no; case '*'; echo dflt; end
