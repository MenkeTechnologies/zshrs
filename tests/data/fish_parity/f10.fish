echo a >/dev/null; echo $status
echo b 1>&2 2>/dev/null; echo $status
echo c 2>&1 >/dev/null | cat
echo x >> /dev/null
