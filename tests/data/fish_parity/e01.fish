function t
    argparse "h/help" "n/name=" "v/verbose" "x=+" "r=?" -- $argv
    or return
    set -q _flag_h; and echo help
    echo name=$_flag_name n=$_flag_n
    echo v=$_flag_v
    echo x=$_flag_x
    echo r=$_flag_r
    echo argv=$argv
end
t -h --name=bob -v -v -x a -x b pos1 -- -z pos2
t -n x
t --verbose rest
t --bad 2>/dev/null
echo st=$status
t -vh
