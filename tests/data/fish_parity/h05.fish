function hello --on-event greet
    echo hello got $argv
end
emit greet a b
emit greet c
