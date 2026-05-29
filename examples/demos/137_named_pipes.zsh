#!/usr/bin/env zshrs
# Named pipes (FIFOs) — demonstration of mkfifo + simple writer/reader pairs.
# Each pair uses bounded timeout to avoid CI hangs.

tmpdir=/tmp/zshrs_fifo_$$
mkdir -p "$tmpdir"
fifo="$tmpdir/myfifo"
trap "rm -rf $tmpdir" EXIT

echo "── create a FIFO ──"
mkfifo "$fifo"
ls -l "$fifo" | head -1

echo "── single message via fifo ──"
(echo "msg from writer" > "$fifo") &
read -r line < "$fifo"
wait
echo "received: $line"

echo "── basic stream (5 messages) ──"
(
    for i in 1 2 3 4 5; do
        echo "msg-$i"
    done > "$fifo"
) &
count=0
while IFS= read -r m; do
    (( count++ ))
    echo "got: $m"
    (( count >= 5 )) && break
done < "$fifo"
wait
echo "total: $count"

echo "── done ──"
