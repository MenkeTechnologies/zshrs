# Expansion nesting
echo "${$(echo hi)}"
echo ${${(L)VAR}:-default}
