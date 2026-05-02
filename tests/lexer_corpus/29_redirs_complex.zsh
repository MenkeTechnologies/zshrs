echo a > /tmp/x 2>&1
echo b 2>/tmp/err >/tmp/out
echo c &> /tmp/all
echo d &>> /tmp/all
echo e {var}>/tmp/x
exec 3< /tmp/in
exec 4> /tmp/out
exec 3<&-
exec 4>&-
echo f >&3
echo g 0<&3
echo h 2>&1 1>&3 3>&-
