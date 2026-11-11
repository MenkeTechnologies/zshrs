alias ll "echo list"
ll a b
function wrap --description "d"
    echo wrapped $argv
end
wrap 1 2
abbr -a gst "git status"
echo done
