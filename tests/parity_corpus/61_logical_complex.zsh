[[ $foo == "a" ]] && echo a || { [[ $foo == "b" ]] && echo b || echo other; }
