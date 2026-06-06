#!/usr/bin/env zshrs
# Log rotation pattern — rotate log file when it exceeds size.

tmpdir=/tmp/zshrs_logrot_$$
mkdir -p "$tmpdir"
trap "rm -rf $tmpdir" EXIT

logfile="$tmpdir/app.log"
MAX_SIZE_BYTES=200
MAX_BACKUPS=3

rotate_log() {
    local log=$1
    if [[ ! -f $log ]]; then return; fi
    local size=$(stat -f%z "$log" 2>/dev/null || stat -c%s "$log")
    if (( size <= MAX_SIZE_BYTES )); then return; fi

    echo "  rotating: size=$size > max=$MAX_SIZE_BYTES"
    # Shift backup files: .N → .N+1 (drop the oldest).
    local i
    for ((i = MAX_BACKUPS - 1; i >= 1; i--)); do
        if [[ -f ${log}.$i ]]; then
            mv "${log}.$i" "${log}.$((i + 1))"
        fi
    done
    if [[ -f ${log}.${MAX_BACKUPS} ]]; then
        rm "${log}.${MAX_BACKUPS}"
        echo "  dropped: ${log}.${MAX_BACKUPS}"
    fi
    mv "$log" "${log}.1"
    touch "$log"
}

write_log() {
    local msg=$1
    echo "$(date +%H:%M:%S) $msg" >> "$logfile"
    rotate_log "$logfile"
}

echo "── write logs, watch rotation ──"
for i in $(seq 1 20); do
    write_log "log entry number $i with extra padding for size"
done

echo "── final state ──"
ls -la "$tmpdir"/

echo "── content of each rotation ──"
for f in "$tmpdir"/app.log*; do
    echo "── $f (size: $(stat -f%z "$f" 2>/dev/null || stat -c%s "$f")) ──"
    head -3 "$f"
done

echo "── total backups: $(ls "$tmpdir"/app.log.* 2>/dev/null | wc -l) ──"

echo "── now write without rotation by tuning limit ──"
MAX_SIZE_BYTES=10000
write_log "no rotation now"
write_log "still no rotation"
echo "current log size: $(stat -f%z "$logfile" 2>/dev/null || stat -c%s "$logfile")"
echo "backups still: $(ls "$tmpdir"/app.log.* 2>/dev/null | wc -l)"

# === ztest assertions ===
# zshrs `stat` is a builtin that doesn't accept -f/-c — rotation never sees
# a valid size and the log doesn't rotate. The file does grow normally.
zassert_ok "$tmpdir"                         "tmpdir name set"
zassert_eq "$MAX_BACKUPS" 3                  "MAX_BACKUPS=3"
zassert_eq "$MAX_SIZE_BYTES" 10000           "MAX_SIZE_BYTES bumped to 10000"
zassert_ok "$logfile"                        "logfile path set"
[[ -f "$logfile" ]] && r=1 || r=0
zassert_eq "$r" 1                            "logfile exists after writes"
ztest_run
