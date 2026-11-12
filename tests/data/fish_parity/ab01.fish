set -l n fish_pid
echo $$n | string match -qr '^\d+$'; echo $status
set -n | string match -q fish_pid; echo $status
echo $fish_pid | string match -qr '^\d+$'; echo $status
