set -g __zq_x 1; set -n | string match '__zq*'
set -g __fish_ls_command ls; set -n | string match '__fish_ls*'
set -n | string match -r '^__' | count
set -g -x __zq_y 2; set -nx | string match '__zq*'
