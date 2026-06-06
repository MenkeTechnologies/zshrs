#!/usr/bin/env zshrs
# Persistent KV-store mock — load/save assoc to a flat file.

tmpfile=/tmp/zshrs_kvs_$$
trap "rm -f $tmpfile" EXIT

save_kvs() {
    local file=$1
    > "$file"
    for k v in "${(@kv)KVS}"; do
        # Escape | in keys and values.
        local sk=${k//|/\\\\|}
        local sv=${v//|/\\\\|}
        echo "${sk}|${sv}" >> "$file"
    done
}

load_kvs() {
    local file=$1
    KVS=()
    if [[ ! -f $file ]]; then return; fi
    while IFS= read -r line; do
        local k=${line%%|*}
        local v=${line#*|}
        # Unescape.
        k=${k//\\|/|}
        v=${v//\\|/|}
        KVS[$k]=$v
    done < "$file"
}

dump_kvs() {
    echo "  size: ${#KVS[@]}"
    for k in ${(ko)KVS}; do
        printf "    %-12s = %s\n" "$k" "${KVS[$k]}"
    done
}

echo "── populate in-memory ──"
typeset -A KVS
KVS[name]=zshrs
KVS[version]=0.11.22
KVS[author]=MenkeTechnologies
KVS[license]=MIT
KVS[count]=210

dump_kvs

echo
echo "── save to disk ──"
save_kvs "$tmpfile"
echo "file content:"
cat "$tmpfile" | sort

echo
echo "── clear in-memory + reload ──"
KVS=()
echo "after clear: ${#KVS[@]} entries"
load_kvs "$tmpfile"
echo "after load:"
dump_kvs

echo
echo "── update + resave ──"
KVS[version]=0.12.0
KVS[count]=234
save_kvs "$tmpfile"
echo "after update:"
cat "$tmpfile" | sort

echo
echo "── round-trip with special chars ──"
KVS=()
KVS["with space"]="value with space"
KVS["with|pipe"]="raw|value"
KVS["normal"]="ok"
save_kvs "$tmpfile"
KVS=()
load_kvs "$tmpfile"
echo "after round-trip:"
dump_kvs

echo
echo "── batch operations ──"
KVS=()
for ((i=1; i<=10; i++)); do
    KVS["key$i"]="value$i"
done
save_kvs "$tmpfile"
echo "saved 10 entries to $tmpfile ($(wc -l < $tmpfile) lines)"
KVS=()
load_kvs "$tmpfile"
echo "loaded back ${#KVS[@]} entries"

# === ztest assertions ===
# Re-populate to a known state for testing.
KVS=()
KVS[a]=1
KVS[b]=2
KVS[c]=3
zassert_eq "${#KVS[@]}" 3       "3 entries inserted"
zassert_eq "${KVS[a]}"  1       "KVS[a]=1"
zassert_eq "${KVS[b]}"  2       "KVS[b]=2"
save_kvs "$tmpfile"
zassert_ok "$(test -f "$tmpfile" && echo y)" "save_kvs wrote file"
KVS=()
zassert_eq "${#KVS[@]}" 0       "KVS cleared"
load_kvs "$tmpfile"
zassert_ge "${#KVS[@]}" 1       "load_kvs hydrated at least 1 entry"
ztest_run
