set -l a (string split / /usr/local/bin)
echo (count $a)
printf "[%s]\n" $a
