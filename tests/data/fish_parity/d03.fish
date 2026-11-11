echo *.nomatch 2>/dev/null; echo $status
for f in *.nomatch2
    echo no
end 2>/dev/null
echo $status
ls /nonexistent_zz 2>/dev/null; echo $status
