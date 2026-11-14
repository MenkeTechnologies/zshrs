# A function that runs no command at all returns 0 (exec.rs get_performer_for_function,
# was_empty), while an empty loop, block or switch elsewhere keeps $status; return, a
# command in a body, a condition or a command substitution counts as running one.
set -l e
function f1; for i in $e; end; end
function f2; begin; end; end
function f3; switch a; case b; echo no; end; end
function f4; end
function f5; begin; end; return; end
function f6; for i in a; true; end; for i in $e; end; end
function f6b; for i in a; false; end; for i in $e; end; end
function f7; for i in (true); end; end
function f8; switch (echo a); case b; end; end
function f9; for i in $e; echo $i; end; end
function f10; switch a; case a; end; end
function f11; for i in a b; end; end
function f12; function inner; end; end
function f13; for i in $e; end; return; end
function f15; begin; for i in $e; end; end; end
function f16; for i in a; for j in $e; end; end; end
function f17; echo $status; for i in $e; end; end
for fn in f1 f2 f3 f4 f5 f6 f6b f7 f8 f9 f10 f11 f12 f13 f15 f16 f17
    false; $fn; echo $fn $status
end
false; for i in $e; end; echo top $status
false; if begin; end; echo ifyes; else; echo ifno; end; echo if $status
false; if for i in $e; end; echo ifyes2; else; echo ifno2; end
true; if begin; end; echo ifyes3; else; echo ifno3; end
false; while begin; end; echo w; break; end; echo while $status
false; and begin; end; echo and $status
false; f1; and echo f1-and
false; not begin; end; echo not $status
set -l one a
function g1; for i in $argv; echo s=$status; end; end
false; g1; echo g1-none $status
false; g1 x; echo g1-x $status
function g2; for i in $argv; false; end; end
true; g2 a; echo g2 $status
function g3; for i in $argv; return 5; end; end
false; g3; echo g3-none $status
g3 a; echo g3-a $status
function g4; switch $argv[1]; case a; false; case b; end; end
true; g4 a; echo g4-a $status
false; g4 b; echo g4-b $status
false; g4 c; echo g4-c $status
function g5 -a x; for i in $x; end; end
false; g5; echo g5 $status
function g6; for i in $argv; g6; end; end
false; g6 a; echo g6 $status
function g7; begin; for i in $argv; false; end; end; end
false; g7; echo g7-none $status
true; g7 a; echo g7-a $status
function g8; for i in $argv; begin; end; end; end
false; g8 a b; echo g8 $status
set r (false; g1); echo cmdsub $status
function g9; for i in $argv; for j in $argv; end; true; end; end
false; g9 a; echo g9 $status
if false; g1; echo cond-yes; else; echo cond-no; end
if false; for i in $e; end; echo c2-yes; else; echo c2-no; end
