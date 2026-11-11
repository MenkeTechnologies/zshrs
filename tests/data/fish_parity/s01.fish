printf "%d\n" 27.0.0 2>/dev/null; echo $status
printf "%d\n" abc 2>/dev/null; echo $status
printf "%s|%5s|%-5s|%.2s\n" a b c defg
printf "%05d|%+d|%x|%X|%o|%u\n" 42 7 255 255 8 3
printf "%.3f|%10.2f|%e|%g\n" 3.14159 2.5 1500 0.0001
printf "%c%c\n" hello world
printf "%b\n" "a\\tb\\n"
printf "%s\n" a b c
printf "%s %s\n" 1 2 3
printf "x\\ty\\n"
printf "%%|%5%|\n" 2>/dev/null
printf "%*d|%-*d|\n" 5 42 4 7
printf "%d %d\n" 1
printf "no args\n"
printf "%s\n"
printf "\\x41\\u00e9\\101\n"
printf "%i\n" 0x1f 010
