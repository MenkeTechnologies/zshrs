if test -d /tmp; and test -d /usr
    echo both
end
if test -d /nonexist; or test -d /usr
    echo either
end
while false; end
echo after
if not test -d /nonexist
    echo negated
end
