for p in /usr /bin
    echo $p
end
echo $PATH | string match -q "*/usr/bin*"; echo $status
