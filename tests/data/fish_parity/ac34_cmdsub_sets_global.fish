function f; set -g zzg (echo hi); echo out1; echo out2; return 3; end
set r (f); echo "r=$r" $status; set -q zzg; echo $status; echo $zzg
echo (f) | count
function h; set -g zzh 1; end
echo [(h)]; echo $zzh
function cache; if not set -q __cc; set -g __cc (echo computed); end; echo $__cc; end
echo (cache) (cache); echo $__cc
function plain; echo p; end; echo (plain) [(plain)]
echo (__fish_uname); set -q __fish_uname; echo $status
if test (__fish_uname) = Darwin; echo darwin; end
echo (ls /nonexistent_zz 2>/dev/null) | count
