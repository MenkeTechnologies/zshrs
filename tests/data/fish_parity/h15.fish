echo $PATH | string split : | count | string match -qr "^[1-9]"; echo $status
set -l old $PATH
set PATH /nonexistent_a $PATH
echo $PATH[1]
echo (count $PATH) (math (count $old) + 1)
