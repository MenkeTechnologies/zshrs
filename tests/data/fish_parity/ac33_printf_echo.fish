printf '%5.1f|%-8s|%+d|% d|%05d\n' 3.14159 ab 5 5 42
printf '%x %X %o %c\n' 255 255 8 65
printf '%s\n' "a b" c
printf '%.3s|\n' abcdef
printf '%10.4s|\n' abcdef
printf '%e\n' 12345.678
printf '%g %g\n' 0.0001 100000000
printf '%i\n' 12
printf '%u\n' 5
printf '%5%|\n'
printf '\\n literal\n'
printf '%b' 'a\n'
printf '%s %s\n' 1
printf '%d %d\n' 1 2 3
printf '%s' ''; echo
printf 'é\U0001F600\n'
printf -- '-x\n'
printf '%-5d|\n' 3
printf '%*s|\n' 6 ab
printf '%.*f\n' 2 3.14159
printf '%ld %lld\n' 5 6
printf '\0101\n'
printf 'a\tb\n' | od -c | head -1
printf '%s\n' 'a\tb'
printf "%s\n" "multi
line"
printf '%d%%\n' 50
printf '%s=%s\n' a 1 b 2
printf '\e[1m' | od -c | head -1
echo -n; echo -e '\a' | od -c | head -1
echo -E 'a\tb'
echo 'a\tb'
echo -ne 'a\n'
echo -sn a b; echo
echo -- -e
echo -e -n 'x\ty'; echo
echo -e 'a\c'; echo b
echo -e 'x\\y'
echo -e '\x4a\112J'
