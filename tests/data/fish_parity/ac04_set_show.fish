set -S PATH | string match -r '^\$PATH: .*' | string replace -r '[0-9]+ elements' N
set -S HOME | string match -r '^\$HOME: .*'
set -x FOO a
set -S FOO
set -S nope
set -g G 1 2
set -S G
