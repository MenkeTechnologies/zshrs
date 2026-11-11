sleep 0.1 &
wait
echo waited
set -l pid (sh -c "echo \$\$")
echo (string match -qr "^\\d+\$" $pid; echo $status)
