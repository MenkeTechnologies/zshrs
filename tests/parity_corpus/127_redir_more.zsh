exec 3>&1
echo hi >&3
exec 3>&-
exec 4<&0
cat <&4
exec 4<&-
echo hi >| clobber
echo hi >>! append_clobber
