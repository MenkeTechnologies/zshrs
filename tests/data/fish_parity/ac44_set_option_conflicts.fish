set -U --show x 2>/dev/null; echo $status
set -l --show x 2>/dev/null; echo $status
set -x --show x 2>/dev/null; echo $status
set -e --show x 2>/dev/null; echo $status
set -q -e x 2>/dev/null; echo $status
set -g -U x 1 2>/dev/null; echo $status
set -l -g x 1 2>/dev/null; echo $status
set -x -u x 1 2>/dev/null; echo $status
set --path --unpath x 1 2>/dev/null; echo $status
set -q --show x 2>/dev/null; echo $status
set -gx --path PP a:b; echo $status $PP
set -q -l nosuch; echo $status
set --show --no-event x >/dev/null; echo $status
set -lx v 1; echo $v
