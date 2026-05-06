exec {myfd}>/tmp/zshrs_test
echo "hi" >&$myfd
exec {myfd}>&-
