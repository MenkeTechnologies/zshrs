status is-interactive; echo $status
status is-login; echo $status
status current-command 2>/dev/null; echo ok
echo $fish_pid | string match -qr "^\\d+\$"; echo $status
echo $PWD | string length -q; echo $status
