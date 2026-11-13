math 'floor(2.5) + ceil(1.2) * round(2.5)'
math -s0 7 / 2
math -b hex 255
math --base=octal 8
math 'max(1,2,3)' + 'min(4,5)'
math 'sqrt(16)^2'
math 'pi' -s3
math 'log2(8)' 
math 'ncr(5,2)' 'npr(5,2)'
math 1e3
math 0x10 + 1
math '5 % 3'
math 'bitand(6,3)' 'bitor(6,3)' 'bitxor(6,3)'
math -s2 '10 / 3'
math 2 '**' 3
math '-5 % 3'
math 'sin(0)' 'cos(0)'
math 'abs(-3)'
math '1 / 3'
math 9007199254740993
math 'exp(0)'
math 'ln(e)'
printf '%s|%5s|%-5s|\n' a b c
printf '%d %i %x %o %e %f %g\n' 5 6 255 8 1.5 2.5 3.5
printf '%c%c\n' a b
printf '%b\n' 'a\tb'
printf '%%|%5.2f|\n' 3.14159
printf '%s\n' a b c
printf '%d\n' abc; echo $status
printf '%*d|\n' 5 3
printf '\x41\101é\n'
