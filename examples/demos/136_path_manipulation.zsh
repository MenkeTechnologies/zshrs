#!/usr/bin/env zshrs
# PATH manipulation — tied path[] array + colon-string PATH.
# Ported from Src/params.c (PM_TIED path↔PATH).

echo "── PATH (scalar) ──"
echo "PATH=$PATH" | head -c 200; echo "..."

echo "── path[] array (tied) ──"
echo "size=${#path[@]}"
echo "first 5:"
for i in 1 2 3 4 5; do
    [[ -n ${path[$i]} ]] && echo "  [$i] ${path[$i]}"
done

echo "── append to PATH ──"
PATH=$PATH:/usr/local/MyBin
echo "after append, last segment: ${path[-1]}"
PATH=${PATH%:/usr/local/MyBin}  # restore

echo "── prepend (typical for tool prefixes) ──"
path=(/opt/mybin $path)
echo "first: ${path[1]}"
echo "size: ${#path[@]}"
path=( "${path[@]:1}" )  # remove first

echo "── dedupe PATH ──"
typeset -U new_path
new_path=( /usr/bin /usr/local/bin /usr/bin /opt/bin /usr/local/bin )
echo "after typeset -U dedupe: ${#new_path[@]} entries"
print -l ${new_path[@]}

echo "── filter PATH for existing dirs only ──"
filter_existing() {
    local p
    for p in "$@"; do
        [[ -d $p ]] && echo "$p"
    done
}
echo "existing in current path:"
filter_existing "${path[@]}" | head -5

echo "── PATH segment count ──"
echo "default PATH has ${#path[@]} segments"

echo "── search a command's location ──"
which echo 2>&1
type ls

echo "── add to PATH conditionally ──"
add_to_path() {
    local dir=$1
    if [[ -d $dir ]]; then
        # Already in path?
        local p
        for p in "${path[@]}"; do
            [[ $p == $dir ]] && return
        done
        path=($dir $path)
        echo "added: $dir"
    else
        echo "skip (not a dir): $dir"
    fi
}
add_to_path /tmp
add_to_path /nonexistent_xyz
add_to_path /tmp  # second add should skip

# === ztest assertions ===
# PATH ↔ path[] is tied; PATH must be non-empty and contain at least one entry.
zassert_ne "$PATH" ""        "PATH set"
zassert_gt "${#path[@]}" "0" "path[] has entries"
# typeset -U dedupe
typeset -U dedupe_test=( a b a c b a )
zassert_eq "${#dedupe_test[@]}" "3" "typeset -U dedupes literals"
# add_to_path behavior
zassert_eq "$(add_to_path /nonexistent_xyz)" "skip (not a dir): /nonexistent_xyz" "add_to_path rejects missing"
# After first add_to_path /tmp, /tmp should be at path[1] (re-add path also lands there)
zassert_eq "${path[1]}" "/tmp" "add_to_path prepends /tmp"
# Second add_to_path /tmp returns silently — no echo
zassert_eq "$(add_to_path /tmp)" "" "second add_to_path /tmp is silent (already in path)"
# Filter for existing dirs
zassert_eq "$(filter_existing /tmp /nonexistent_xyz_zzz)" "/tmp" "filter_existing keeps only real dirs"
ztest_run
