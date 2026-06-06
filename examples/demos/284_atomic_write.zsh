#!/usr/bin/env zshrs
# Atomic file write via tmpfile + rename — crash-safe IO pattern.

tmpdir=$(mktemp -d)

atomic_write() {
    local target=$1 content=$2
    local tmp="${target}.tmp.$$.$RANDOM"
    print -r -- "$content" > "$tmp"
    if [[ ! -s $tmp ]]; then
        command rm -f "$tmp"
        echo "  ✗ tmp write failed: $tmp"
        return 1
    fi
    # mv = atomic on POSIX same-FS.
    mv "$tmp" "$target"
}

read_file() {
    [[ -r $1 ]] && cat "$1"
}

echo "── basic atomic write ──"
atomic_write "$tmpdir/config.txt" "version=1
debug=false
port=8080"
echo "  file size: $(wc -c < $tmpdir/config.txt) bytes"
echo "  content:"
read_file "$tmpdir/config.txt" | sed 's/^/    /'

echo
echo "── overwrite preserves until commit ──"
atomic_write "$tmpdir/config.txt" "version=2
debug=true
port=9090
new_field=added"
echo "  size after update: $(wc -c < $tmpdir/config.txt) bytes"
echo "  content:"
read_file "$tmpdir/config.txt" | sed 's/^/    /'

echo
echo "── multiple writers don't corrupt ──"
for i in 1 2 3 4 5; do
    atomic_write "$tmpdir/counter.txt" "iteration $i at $(date +%s)" 2>/dev/null
done
echo "  final content:"
read_file "$tmpdir/counter.txt" | sed 's/^/    /'

echo
echo "── leftover tmp cleanup ──"
# Create orphan tmp files to simulate a crashed writer.
touch "$tmpdir/data.txt.tmp.99999.123"
touch "$tmpdir/data.txt.tmp.99998.456"
echo "  before cleanup:"
ls "$tmpdir"/*.tmp.* 2>/dev/null | sed 's/^/    /'

# Cleanup pattern: remove old .tmp files.
for f in "$tmpdir"/*.tmp.*; do
    [[ -e $f ]] && command rm -f "$f"
done
echo "  after cleanup:"
if compgen -G "$tmpdir/*.tmp.*" > /dev/null 2>&1; then
    ls "$tmpdir"/*.tmp.* 2>/dev/null | sed 's/^/    /'
else
    echo "    (none)"
fi

echo
echo "── lock-file pattern (mkdir-as-mutex) ──"
acquire_lock() {
    local lockdir=$1
    if mkdir "$lockdir" 2>/dev/null; then
        echo "  ✓ acquired lock: $lockdir"
        return 0
    else
        echo "  ✗ lock held: $lockdir"
        return 1
    fi
}
release_lock() {
    rmdir "$1" 2>/dev/null && echo "  ✓ released: $1"
}

lock="$tmpdir/critical.lock"
acquire_lock "$lock"
acquire_lock "$lock"  # second try fails
release_lock "$lock"
acquire_lock "$lock"  # works again
release_lock "$lock"

echo
echo "── compare-and-swap via mv-based file ──"
cas_file() {
    local target=$1 expected_hash=$2 new_content=$3
    if [[ ! -e $target ]]; then
        echo "  ✗ no file to CAS"
        return 1
    fi
    local cur=$(cat "$target")
    local cur_hash=$(echo -n "$cur" | wc -c)  # toy "hash" = byte count
    if [[ $cur_hash == $expected_hash ]]; then
        atomic_write "$target" "$new_content"
        echo "  ✓ CAS succeeded (hash $cur_hash → updated)"
        return 0
    else
        echo "  ✗ CAS failed: expected hash $expected_hash, got $cur_hash"
        return 1
    fi
}

atomic_write "$tmpdir/cas.txt" "initial state"
init_hash=$(wc -c < "$tmpdir/cas.txt")
echo "  initial hash: $init_hash"
cas_file "$tmpdir/cas.txt" "$init_hash" "updated state"
cas_file "$tmpdir/cas.txt" "$init_hash" "should fail"  # stale hash now
echo "  final:"
read_file "$tmpdir/cas.txt" | sed 's/^/    /'

# === ztest assertions ===
testdir=$(mktemp -d)
# atomic_write writes file
atomic_write "$testdir/x.txt" "hello world"
zassert_eq "$(cat "$testdir/x.txt")" "hello world" "atomic_write content"
# File is readable
zassert_ok "$([[ -r $testdir/x.txt ]] && echo 1)" "file readable"
# Overwrite works
atomic_write "$testdir/x.txt" "overwritten"
zassert_eq "$(cat "$testdir/x.txt")" "overwritten" "atomic_write overwrites"
# read_file helper
zassert_eq "$(read_file "$testdir/x.txt")" "overwritten" "read_file returns content"
zassert_eq "$(read_file "$testdir/nonexistent")" "" "read_file empty on missing"
# Lock acquire/release
mylock="$testdir/m.lock"
acquire_lock "$mylock" > /dev/null
zassert_ok "$([[ -d $mylock ]] && echo 1)" "lock dir created on acquire"
acquire_lock "$mylock" > /dev/null
zassert_eq "$?" 1 "second acquire fails (lock held)"
release_lock "$mylock" > /dev/null
zassert_err "$([[ -d $mylock ]] && echo 1)" "lock dir removed on release"
command rm -rf "$testdir"

command rm -rf "$tmpdir"
ztest_run
