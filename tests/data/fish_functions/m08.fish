source @FN@/_validate_int.fish
function t
    argparse "x/xx=!_validate_int --min 1 --max 5" "n/num=" -- $argv
    or return
    echo ok x=$_flag_x n=$_flag_n
end
t -x 3 -n 7
t -x 9 2>/dev/null
echo st=$status
t -x abc 2>/dev/null
echo st=$status
t -x 0 2>/dev/null; echo st=$status
