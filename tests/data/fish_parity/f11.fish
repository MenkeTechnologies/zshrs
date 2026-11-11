set -l n 5
set -l fact 1
while test $n -gt 1
    set fact (math $fact \* $n)
    set n (math $n - 1)
end
echo $fact
