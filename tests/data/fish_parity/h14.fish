function p; for a in $argv; echo "[$a]"; end; end
p a "b c" ""
p (echo "x y"; echo z)
p "$HOME" | string length -q; echo $status
