function f --argument-names a b
    echo a=$a b=$b argc=(count $argv)
end
f 1 2 3
