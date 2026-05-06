# Complex redirects
exec 3>&1 >/tmp/log 2>&3
echo hi >&3
exec 3>&-
