set -l out (false)
echo $status
set -l out2 (sh -c "exit 4")
echo $status
if set -l r (echo ok)
    echo got $r
end
