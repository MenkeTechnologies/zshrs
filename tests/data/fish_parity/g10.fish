set -l words one two three
for w in $words
    printf "%s:%d\n" $w (string length $w)
end
echo (string join " " (string upper $words))
