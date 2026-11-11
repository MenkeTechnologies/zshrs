set -l sorted (printf "%s\n" c a b | sort)
echo $sorted
echo (printf "%s\n" c a b | sort | string join ,)
