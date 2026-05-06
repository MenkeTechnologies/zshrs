# Niche lexer constructs
echo $""
echo $"localized"
echo $'\0'
echo $'\u1234'
echo ${x:offset:length}
echo ${(q)$(echo hi)}
echo $(( [2+2]=5 )) # array assignment in arith? No, zsh doesn't support that but supports others.
echo $(( 0x123 ))
echo $(( 0o123 ))
echo $(( 0b101 ))
echo $(( [#16] 255 ))
echo $(( [##16] 255 ))
