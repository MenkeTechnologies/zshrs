set -l s "a,b,,c"
for part in (string split , $s)
    echo "[$part]"
end
string split -n , $s | count
string join "|" (string split , $s)
