echo `cat /tmp/x`
echo `echo "double inside"`
echo `echo 'single inside'`
echo "outer `inner`"
echo "`echo 'nested'`"
result=`grep pat file`
