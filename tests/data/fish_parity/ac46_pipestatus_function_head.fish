function f; set -g G 1; return 1; end
f | cat; echo $pipestatus
function h; echo x; return 3; end
h | cat; echo $pipestatus
h | string upper | cat; echo $pipestatus
h | cat; set -n | string match -r '^__fish_ip'; echo done
