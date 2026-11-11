argparse --move-unknown "s/short=+&" "o/old=+&" "l/long=+&" "w/windows=+&" -- -s e -l erase
echo st=$status s=$_flag_s l=$_flag_l argv=$argv
argparse --move-unknown "h/help" "v/verbose&" -- -h -v --foo bar -x
echo h=$_flag_h v=$_flag_v argv=$argv opts=$argv_opts
