printf 'a\nb\n' | read -z -l -a arr; printf '%s|' $arr; echo
printf 'a b\nc\n' | read -z -l x y; printf '[%s][%s]' $x $y | od -c
printf 'a b c\n' | read -z -l x y; printf '[%s][%s]' $x $y | od -c
printf 'a b c\n' | read -z -l -a z; printf '%s|' $z; echo
