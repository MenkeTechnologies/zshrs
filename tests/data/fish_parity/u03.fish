function f
    set -lx W a ""
    count $W
    echo "[$W[1]]"
    sh -c "printf \"<%s>\" \"\$W\""; echo
end
f
set -q W; echo $status
set -gx G 1 2 3
count $G
sh -c "echo \$G"
set -a G 4
echo $G
sh -c "echo \$G"
set -gx GP /a /b
sh -c "echo \$GP"
set -gx PH /x /y
echo $PH[2]
set -e G
set -q G; echo $status
set -gx KEEP v
set KEEP w
sh -c "echo \$KEEP"
