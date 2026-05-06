# Parameter expansion with command substitution and flags
echo ${(L)$(echo HI)}
echo ${(U)${VAR:-default}}
echo ${(qq)$(echo "quoted's")}
