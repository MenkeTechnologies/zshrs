set -l s "a:b:c"
set -l parts (string split : $s)
echo $parts[2]
echo (count $parts)
set -l joined (string join / $parts)
echo $joined
echo $s | string split : | head -n 2
