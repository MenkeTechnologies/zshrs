set -l p /usr/local/bin/tool.sh
echo (basename $p) (dirname $p)
echo (string replace -r "\\.sh\$" "" (basename $p))
echo (path basename $p) (path dirname $p) (path extension $p)
