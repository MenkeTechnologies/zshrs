#!/usr/bin/env zshrs
# File locking with mkdir-as-lock (atomic on POSIX) + cleanup trap.

tmpdir=/tmp/zshrs_lock_$$
mkdir -p "$tmpdir"
trap "rm -rf $tmpdir" EXIT

LOCK_DIR="$tmpdir/job.lock"

acquire_lock() {
    if mkdir "$LOCK_DIR" 2>/dev/null; then
        echo $$ > "$LOCK_DIR/pid"
        echo "lock acquired by $$"
        return 0
    else
        local owner=$(cat "$LOCK_DIR/pid" 2>/dev/null || echo unknown)
        echo "lock held by $owner"
        return 1
    fi
}

release_lock() {
    rm -rf "$LOCK_DIR"
    echo "lock released by $$"
}

with_lock() {
    if ! acquire_lock; then
        echo "  could not acquire — exiting"
        return 1
    fi
    trap "release_lock" RETURN
    "$@"
    release_lock
    trap - RETURN
}

job_work() {
    echo "  doing work (sleep 0.1)..."
    sleep 0.1
    echo "  work done"
}

echo "── simple acquire/release ──"
with_lock job_work
echo "still locked? $([[ -d $LOCK_DIR ]] && echo yes || echo no)"

echo "── second acquisition while held (simulate concurrent) ──"
# Hold the lock manually.
acquire_lock
# Try again — should fail.
acquire_lock
release_lock

echo "── lock-with-timeout ──"
acquire_lock_with_timeout() {
    local max_ms=$1 elapsed=0 wait_ms=20
    while (( elapsed < max_ms )); do
        if mkdir "$LOCK_DIR" 2>/dev/null; then
            echo $$ > "$LOCK_DIR/pid"
            return 0
        fi
        sleep 0.02
        (( elapsed += wait_ms ))
    done
    return 1
}

# Hold the lock from a bg process.
(
    mkdir "$LOCK_DIR" 2>/dev/null
    echo $$ > "$LOCK_DIR/pid"
    sleep 0.15
    rm -rf "$LOCK_DIR"
) &
sleep 0.02
# Try to acquire with 1s timeout.
if acquire_lock_with_timeout 1000; then
    echo "got lock after waiting"
    rm -rf "$LOCK_DIR"
else
    echo "timed out"
fi
wait

echo "── stale lock detection ──"
mkdir "$LOCK_DIR"
echo 99999 > "$LOCK_DIR/pid"   # fake stale pid
check_stale() {
    local owner=$(cat "$LOCK_DIR/pid" 2>/dev/null)
    if [[ -n $owner ]] && ! kill -0 $owner 2>/dev/null; then
        echo "lock held by dead pid $owner — clearing"
        rm -rf "$LOCK_DIR"
        return 0
    fi
    return 1
}
check_stale && echo "lock cleared"

# === ztest assertions ===
# Make sure lock_dir is clean to start each assert.
rm -rf "$LOCK_DIR"
zassert_ok "$(acquire_lock >/dev/null && echo 1)" "first acquire succeeds"
# Second attempt should fail (lock held); $? becomes non-zero.
acquire_lock >/dev/null 2>&1
rc=$?
zassert_ne "$rc" "0" "double acquire fails (non-zero rc)"
release_lock >/dev/null
zassert_eq "$(test -d "$LOCK_DIR" && echo y || echo n)" "n" "release_lock removes dir"
# Re-acquire after release.
zassert_ok "$(acquire_lock >/dev/null && echo 1)" "re-acquire after release"
release_lock >/dev/null
# stale detection on a fake pid
mkdir -p "$LOCK_DIR"
echo 99999 > "$LOCK_DIR/pid"
check_stale_out=$(check_stale)
zassert_contains "$check_stale_out" "clearing" "stale lock detected"
zassert_eq "$(test -d "$LOCK_DIR" && echo y || echo n)" "n" "stale lock cleared"
ztest_run
