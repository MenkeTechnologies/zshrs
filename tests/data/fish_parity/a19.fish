echo a > /dev/null
echo b 2>&1 | cat
echo c >&2 2>/dev/null
echo d &> /dev/null; echo $status
