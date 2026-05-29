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
