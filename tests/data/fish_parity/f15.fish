set -l x 3
echo (test $x -gt 2 && echo yes || echo no)
echo (not true; echo $status)
true; and false; or echo or-ran
false; and echo skipped; or echo recovered
