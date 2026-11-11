source @FN@/export.fish
export FOO=bar
echo $FOO
export LST=a:b
sh -c 'echo $FOO $LST'
