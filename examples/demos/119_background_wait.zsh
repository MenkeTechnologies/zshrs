#!/usr/bin/env zshrs
# Background jobs + wait + job control.
# Ported from Src/jobs.c (job table) + Src/signals.c handletrap.

echo "── start a background job ──"
(sleep 0.1; echo "bg done") &
bg_pid=$!
echo "background PID: $bg_pid"

echo "── wait for it ──"
wait $bg_pid
echo "after wait: \$? = $?"

echo "── multiple bg jobs ──"
for i in 1 2 3; do
    (sleep 0.05; echo "job $i done") &
done
wait
echo "all jobs done"

echo "── pid capture ──"
sleep 0.2 &
p1=$!
sleep 0.2 &
p2=$!
echo "p1=$p1 p2=$p2"
wait $p1 $p2
echo "both waits returned"

echo "── async pipeline (each stage backgrounded) ──"
{ printf 'A\nB\nC\n'; sleep 0.1; printf 'D\nE\n'; } | sort

echo "── exit code propagation through wait ──"
(exit 7) &
wait $!
echo "got exit code: $?"

(exit 0) &
wait $!
echo "got exit code: $?"

echo "── multiple producers, one consumer ──"
{
    (echo "from A") &
    (echo "from B") &
    (echo "from C") &
    wait
} | sort

echo "── timed-out subshell pattern ──"
timeout_run() {
    local cmd="$1" max=$2
    (eval "$cmd") &
    local pid=$!
    local elapsed=0
    while kill -0 $pid 2>/dev/null; do
        sleep 0.02
        (( elapsed += 20 ))
        if (( elapsed >= max )); then
            kill -9 $pid 2>/dev/null
            echo "killed after ${max}ms"
            return
        fi
    done
    wait $pid
    echo "finished naturally"
}
timeout_run "sleep 0.05" 500
timeout_run "sleep 5" 200

echo "── all done ──"

# === ztest assertions ===
# Smoke: bg + wait returns success and captures exit code.
(true) &
wait $!
zassert_eq "$?" "0"  "wait success bg"
(exit 7) &
wait $!
zassert_eq "$?" "7"  "wait propagates exit 7"
(exit 0) &
wait $!
zassert_eq "$?" "0"  "wait propagates exit 0"
# $! is set after bg
(sleep 0.05) &
zassert_ne "$!" ""  "\$! set after bg"
wait
# Async pipeline produces all 5 sorted lines
out=$({ printf 'A\nB\nC\n'; printf 'D\nE\n'; } | sort | tr '\n' ' ')
zassert_eq "$out" "A B C D E "  "async pipeline sort"
ztest_run
