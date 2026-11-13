set y a b
echo $y[0] 2>/dev/null; echo s=$status
echo "$y[0]" 2>/dev/null; echo s=$status
echo x $y[1..0] 2>/dev/null; echo s=$status
echo $y[1] $y[-1]
