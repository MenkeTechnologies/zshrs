function greet
    argparse "n/name=" -- $argv
    or return 1
    if set -q _flag_name
        echo hello $_flag_name
    else
        echo hello anonymous
    end
end
greet
greet -n Bob
greet --name=Alice
