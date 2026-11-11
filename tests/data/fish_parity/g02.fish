printf "%s\n"
printf "%s %s\n" a
printf "%5.2f|%-5s|%05d\n" 3.14159 ab 42
printf "%x %o %c\n" 255 8 65 2>/dev/null
printf "\\x41\\n"
printf "%s\n" "a b" c
printf "no newline"; echo
printf "%%\n"
printf "%b\n" "a\\tb"
