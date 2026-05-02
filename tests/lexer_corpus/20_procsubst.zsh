diff <(ls /tmp) <(ls /var/tmp)
cat <(echo a) <(echo b)
tee >(grep foo) >(grep bar) <<<input
sort -u <(curl -s a) <(curl -s b)
