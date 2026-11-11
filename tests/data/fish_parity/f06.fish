set -l v 1
switch $v
    case 1 2
        echo low
    case 3
        echo three
    case "*"
        echo other
end
switch (echo hello)
    case "h*o"
        echo glob
end
