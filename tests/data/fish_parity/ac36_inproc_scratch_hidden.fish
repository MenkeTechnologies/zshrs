function g; set -g zg 1; echo o; end
echo (g); g | cat; set -n | string match -r '__fish_i[a-z]'; echo done
