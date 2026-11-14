# fish's slice ranges (parse_slice in expand.rs): a range that starts outside the list
# is clamped to it, both ends past the end is nothing, and a range whose ends have
# different signs runs in the direction the negative end forces.
set w 'a b' c
echo a $w[-3..-2] / "$w[-3..-2]" /
set s one two three four five
echo b $s[2..-1] / $s[-1..2] / $s[4..2] / $s[7..9] / $s[7..2] / $s[2..7]
echo c $s[-9..2] / $s[2..-9] / $s[-9..-7] / $s[-7..-1] / $s[-1..-7] / $s[1..-7] / $s[-7..1]
echo d $s[..] / $s[3..] / $s[..3] / $s[-2..] / $s[..-2] / $s[-7..] / $s[..-7]
echo e $s[1 ..2] / $s[.. 2] / $s[2 .. ] / "$s[1..3]" /
set i 2
echo f $s[$i..-1] / $s[-1..$i] / "$s[$i..]" / $s[(echo 3)..]
set e
echo g $e[1..2] / $e[-1..1] / $e[..] / "$e[2..3]" /
