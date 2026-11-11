set s "hello"
echo (string sub -s 1 -e 3 $s)
echo (string upper (string sub -l 1 $s))(string sub -s 2 $s)
echo "$s world"
echo "$s"" again"
echo $s$s
echo "$s"x
