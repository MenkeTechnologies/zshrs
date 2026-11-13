set v status; echo $$v
set v fish_bind_mode; echo $$v
set v status_generation; echo $$v
set -q status; echo $status
set -q fish_killring; echo $status
set -q status_generation; echo $status
set -q history; echo $status
set -q _; echo $status
set -q nosuchvar_zz; echo $status
set v fish_killring; echo (count $$v)
echo $COLUMNS $LINES
set -g LONGV (string repeat -n 300 a) b
set -g | string match 'LONGV*' | string length
set -g -L | string match 'LONGV*' | string length
set -g --long | string match 'LONGV*' | string length
