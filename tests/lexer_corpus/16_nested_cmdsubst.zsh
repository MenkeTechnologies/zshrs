echo $(echo $(echo $(date)))
echo `echo \`echo nested\``
x=$(uname -s)
y=$(($(date +%s) + 10))
