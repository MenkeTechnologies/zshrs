echo a$fish_bind_mode
set -l y $fish_private_mode; echo $y
for v in (set -n); set -q $v; or echo unset $v; end
set -q fish_history; echo $status; echo (count $fish_history)
string escape -- $IFS
echo [$fish_killring] [$history] $status_generation
set -S fish_history
