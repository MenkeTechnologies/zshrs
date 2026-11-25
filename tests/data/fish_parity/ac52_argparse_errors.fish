function t; argparse $argv[1..-2] -- $argv[-1] 2>/dev/null; echo "rc=$status argv=$argv"; end
function r; set -l spec $argv[1]; set -e argv[1]; argparse $spec -- $argv 2>/dev/null; echo "rc=$status"; end
r h/help --bogus
r 'n/name=' -n
r 'a' -a -a
r 'a/long-a' -x a,b -a
r h -h
r 'i=!_validate_int' -i x
r '#-num' -5
r 'a' --a
r 'a-b' -a
r '' -a
r 'x=+' -x 1 -x 2
r 'a&' -a
r 'long' --long
r 'long=' --long
r 'a' -a=3
r 'n/num=' --num
r 'n/num=?' --num 5
argparse -x a 2>/dev/null; echo "rc=$status"
argparse 2>/dev/null; echo "rc=$status"
argparse --bogus -- a 2>/dev/null; echo "rc=$status"
argparse --min-args 2 -- a 2>/dev/null; echo "rc=$status"
argparse --max-args 0 -- a 2>/dev/null; echo "rc=$status"
function d; argparse --min-args 1 --max-args 2 -- $argv; echo "rc=$status ok $argv"; end; d; d 1 2 3; d 1
function z1; argparse -- $argv; echo "rc=$status argv=$argv"; end
z1 -a; z1 a b; z1; z1 -- x
function z2; argparse -i -- $argv; echo "rc=$status argv=$argv"; end
z2 -a b
function z3; argparse -u -- $argv; echo "rc=$status argv=$argv"; end
z3 -a b
function g1; argparse 'a/long-a' 'b' -x a,b -- $argv; echo in=$status; end; g1 -a -b; echo st=$status
function g2; argparse 'a' 'b' -x a,b -- $argv; echo in=$status; end; g2 -a -b; echo st=$status
function g3; argparse 'a' 'b' -x a,b -- $argv; echo in=$status; end; g3 -a; echo st=$status
function c3; argparse 'h/help' 'v/verbose+' -- $argv; or return; end; c3 --bogus; echo st=$status
