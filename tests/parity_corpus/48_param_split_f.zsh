x=$'a\nb\nc'
echo ${(f)x}
y="${(f)x}"
echo $y
