history append 'echo x'; echo A$status
echo S(history search -e 'echo x')E $status
history delete -e 'echo x' 2>/dev/null; echo D$status
history merge 2>/dev/null; echo M$status
for a in -b -t -j -p -P -C -S --is-valid -r -o
    commandline $a 2>/dev/null; echo "cl $a: $status"
end
commandline -L; echo $status
commandline -i foo 2>/dev/null; echo $status
commandline -f repaint; echo $status
fish_vi_key_bindings 2>/dev/null; echo $fish_key_bindings
fish_default_key_bindings; echo $fish_key_bindings
set -n | string match -r '^__fish_hist' | count
