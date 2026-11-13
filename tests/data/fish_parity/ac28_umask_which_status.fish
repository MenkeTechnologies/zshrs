echo --
umask
umask 022; umask; umask -S; umask -p
umask 027; umask; umask -S
umask u=rwx,g=rx,o=; umask
echo --
type -P ls
which ls
function mine; end
which mine
echo --
echo --
status is-command-substitution; echo $status
echo (status is-command-substitution; echo $status)
status basename
status dirname | path basename
status line-number
