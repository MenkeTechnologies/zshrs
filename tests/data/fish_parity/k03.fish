MYVAR=env if test "$MYVAR" = env
    echo has-env
end
echo [$MYVAR]
set -l t "x$(echo "nested $(echo inner)")y"
echo $t
set -l u (
    # comment with ) paren and (
    echo cmt
)
echo $u
