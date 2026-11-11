function f
    set -g created_in_f yes
end
f
echo $created_in_f
function h
    set -q undefined_zzz; or echo undefined
end
h
