set -g zzq a b
set -g zzr hello
set -g zzs "x y"
set -g zzt
set -g | string match "zz*"
function f
    set -l loc a b
    set -l | string match "loc*"
    set -n | string match "loc*"
end
f
set -g -n | string match "zz*"
set -gx zzx 1
set -x | string match "zzx*"
