printf "a/x\0b/y\0" | path basename
printf "a/x\nb/y\n" | path dirname
printf "a.txt\0" | path extension
printf "x\0y\n" | path basename | count
printf "" | path basename; echo $status
path filter -xZ /bin/sh /nonexistent_zz | string split0 | path basename
