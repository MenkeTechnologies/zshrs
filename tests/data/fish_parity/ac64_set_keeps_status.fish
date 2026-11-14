# an assignment that works leaves $status as it was; erase, query, a failed
# assignment and a command substitution set it
false; set x 1; echo a $status
false; for i in a; set x 1; end; echo b $status
false; set -l y 1; echo c $status
false; set -a x 2; echo d $status
false; set x[1] 3; echo e $status
false; set -g z; echo f $status
false; set -e x; echo g $status
false; set -q x; echo h $status
false; set x (true); echo i $status
false; set -x X 1; echo j $status
function ff; false; set -l q 1; echo k $status; false; set w 2; echo l $status; end; ff
false; set MYPATH a:b; echo m $status
true; set x[0] 1 2>/dev/null; echo n $status
set -g zq_g G; set -e -U zq_g; echo o $status $zq_g
set -e -U zq_none; echo p $status
set -e -U zq_none[1]; echo q $status
