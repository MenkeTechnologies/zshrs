function w --wraps ls --description 'd e' -a one two --on-event ev -S
  echo w
end
functions w
function d; echo $argv[1]; set -q argv[2]; and d $argv[2..-1]; end
d 1 2 3
set --path q a:b:c
echo $q[2] (count $q)
echo a:b:c | read -d : x y
echo "[$x][$y]"
echo a:b:c | read -d : x
echo "[$x]"
echo a:b:c | read -d : -a l
echo $l
echo abc | read -n 2 p
echo $p
function pw --on-variable PWD
  echo moved $argv
end
cd /
function o; argparse a -- $argv; echo st=$status; end
o --bogus 2>/dev/null
