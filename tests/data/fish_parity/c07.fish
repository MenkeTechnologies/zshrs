set -l str "The quick brown fox"
set -l words (string split " " $str)
echo (count $words)
echo $words[-1]
echo (string join " " $words[2..3])
for w in $words[1..2]
    echo $w
end
