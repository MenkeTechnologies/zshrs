count (echo); count (printf ""); count (printf "\n\n"); count (printf "a\n\nb\n")
echo "[" (printf "zz\n")[2..-1] "]"
echo (printf "a\nb\nc\n")[2]
echo (printf "abc\n")[1]
echo (printf "a\nb\nc\n")[-1..1]
set -l v (printf "x\n\ny\n")
echo (count $v)
set -l s (false; true)
echo $status
set -l t (sh -c "exit 3")
echo $status
