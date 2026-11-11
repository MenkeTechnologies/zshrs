function f; echo called $argv; end
f a b
command echo via-command
builtin echo via-builtin
set -l cmd echo
$cmd dynamic
$cmd "two words"
