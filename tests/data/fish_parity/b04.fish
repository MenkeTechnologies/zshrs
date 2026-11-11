set PATH /custom/bin $PATH
echo $PATH[1]
set -l first $PATH[1]
echo $first
contains /custom/bin $PATH; and echo found
set -e PATH[1]
echo $PATH[1] | string match -q /custom/bin; echo $status
