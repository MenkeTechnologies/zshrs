# An unquoted space inside a live brace group is fish's BRACE_SPACE. In a variable's
# index it is neither whitespace nor a digit to parse_slice, so the word is the
# expansion error "Invalid index value": the command does not run, $status is 121, and
# the script goes on. A tab, a quoted index, a space inside a `(cmd)` in the index, or a
# group made literal by an enclosing one is fine.
set x a b c
echo {$x[1 9],z}
echo 1 $status
echo {$x[1 .. 2],z} {a,{b,$x[1 2]}}c
echo 2 $status
set y {$x[1 2],z}
echo 3 $status
count $y
for i in {,$x[ 1]}
    echo never $i
end
echo 4 $status
echo {$x[1 2],z} | cat
echo 5 $status
not echo {$x[2 ],z}
echo 6 $status
if echo {"x"$x[1 2]""}
    echo never
else
    echo 7 $status
end
echo 8 {$x[1	2],z} {"$x[1 2]",z} {$x[(echo 1 3)],z} {{$x[1 2]}} {$x[1..2],z}
echo 9 {(printf '%s\n' a b)[1 2],z} $x[1 2]{p,q}
