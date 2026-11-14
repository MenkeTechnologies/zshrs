# The order of a word's cartesian product: command substitutions first, the leftmost
# varying slowest; then variables, the last varying slowest; then brace groups, the
# last varying slowest and all of them faster than any list.
set a 1 2
set b x y
set l one two
echo a $a$b
echo b {a,b}$l / $l{a,b}
echo c {a,b}$a{x,y}$b
echo d {a,b}{c,d}$a / $a{x,y}$b
echo e {a,b}-$l-{c,d}
echo f (printf '%s\n' 1 2)(printf '%s\n' x y) / $a(printf '%s\n' p q) / (printf '%s\n' p q)$a
echo g {a,b}(printf '%s\n' p q) / (printf '%s\n' p q){a,b}
echo h {$a,b}x / {a,{b,c}$a}
set e
echo i {$e,b} / {a,b}$e / x$e{a,b} /
