builtin -q string; echo $status
builtin -q nonexistent_zz; echo $status
builtin -n | string match -q string; echo $status
functions -n | string match -q alias; echo $status
