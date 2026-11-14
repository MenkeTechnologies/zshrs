# A for loop that runs no command (no items, or an empty body), an empty begin block and a
# switch with no case run leave $status as it was.
false
for i in
    echo never
end
echo for-none $status
false
for i in a b
end
echo for-empty-body $status
set -l e
false
for i in $e
end
echo for-empty-var $status
false
for i in (true)
end
echo for-cmdsub $status
for i in a b
    false
    break
end
echo break $status
for i in a b
    false
    continue
end
echo continue $status
function g
    false
    for i in
    end
end
g
echo function $status
false
begin
end
echo begin $status
false
switch a
    case b
        echo no
end
echo switch-none $status
false
switch a
    case a
end
echo switch-empty $status
false
while false
end
echo while $status
printf '%5.2f\n' 'x y' 19 '  pad  '
for i in $e
    echo $i
end
