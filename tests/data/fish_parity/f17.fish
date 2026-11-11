set -l t (printf "a\nb\nc\n")
echo (count $t)
set -l u (printf "a b\nc d\n")
echo (count $u) $u[2]
for l in (printf "x\n\ny\n")
    echo "<$l>"
end
echo (echo) | count
