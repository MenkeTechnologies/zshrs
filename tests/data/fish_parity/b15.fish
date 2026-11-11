set d (mktemp -d)
echo hi > $d/f.txt
cat $d/f.txt
echo more >> $d/f.txt
cat < $d/f.txt
for f in $d/*.txt
    echo (basename $f)
end
rm -r $d
