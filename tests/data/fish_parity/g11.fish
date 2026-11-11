set -l f /tmp/x.tar.gz
echo (string replace -r "\\.tar\\.gz\$" "" $f)
echo (string match -r "[^/]+\$" $f)
echo (path basename $f)
echo (path dirname $f)
echo (path change-extension txt $f)
echo (path extension $f)
echo (path normalize /a/b/../c//d)
path is /tmp; echo $status
path filter /tmp /nonexistent_zz
