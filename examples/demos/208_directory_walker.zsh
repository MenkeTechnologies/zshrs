#!/usr/bin/env zshrs
# Directory walker — recursive traversal with depth tracking.

walk() {
    local dir=$1
    local depth=${2:-0}
    local indent=""
    for ((i=0; i<depth; i++)); do indent+="  "; done

    if [[ ! -d $dir ]]; then return; fi

    local item
    for item in "$dir"/*(N); do
        local name=${item:t}
        if [[ -d $item ]]; then
            echo "${indent}📁 $name/"
            walk "$item" $((depth + 1))
        else
            echo "${indent}📄 $name"
        fi
    done
}

walk_with_sizes() {
    local dir=$1
    local depth=${2:-0}
    local indent=""
    for ((i=0; i<depth; i++)); do indent+="  "; done
    if [[ ! -d $dir ]]; then return; fi
    local item
    for item in "$dir"/*(N); do
        local name=${item:t}
        if [[ -d $item ]]; then
            echo "${indent}📁 $name/"
            walk_with_sizes "$item" $((depth + 1))
        else
            local size=$(stat -f%z "$item" 2>/dev/null || stat -c%s "$item")
            printf "%s📄 %s (%d bytes)\n" "$indent" "$name" $size
        fi
    done
}

tmpdir=/tmp/zshrs_walker_$$
mkdir -p "$tmpdir"/{src/a,src/b,docs,tests,build}
touch "$tmpdir"/README.md "$tmpdir"/LICENSE
echo "main code" > "$tmpdir"/src/main.rs
echo "lib" > "$tmpdir"/src/lib.rs
echo "module a" > "$tmpdir"/src/a/mod.rs
echo "module b" > "$tmpdir"/src/b/mod.rs
echo "guide" > "$tmpdir"/docs/guide.md
echo "intro" > "$tmpdir"/docs/intro.md
echo "test 1" > "$tmpdir"/tests/test1.rs
echo "test 2" > "$tmpdir"/tests/test2.rs
trap "rm -rf $tmpdir" EXIT

echo "── tree (just names) ──"
walk "$tmpdir"

echo "── tree (with sizes) ──"
walk_with_sizes "$tmpdir"

echo "── flat list of all files ──"
list_files() {
    local dir=$1
    for f in "$dir"/**/*(N.); do
        echo "  ${f#$tmpdir/}"
    done
}
list_files "$tmpdir"

echo "── count by extension ──"
count_by_ext() {
    local dir=$1
    typeset -A counts
    for f in "$dir"/**/*(N.); do
        local ext=${f:e}
        [[ -z $ext ]] && ext="(none)"
        (( counts[$ext]++ ))
    done
    for k in ${(ko)counts}; do
        printf "  .%-6s = %d\n" "$k" ${counts[$k]}
    done
}
count_by_ext "$tmpdir"

echo "── total disk usage ──"
total=0
for f in "$tmpdir"/**/*(N.); do
    size=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f")
    (( total += size ))
done
echo "  total: $total bytes"

# === ztest assertions ===
# stat -f / -c flags fail in this zshrs build (stat is a builtin), so byte
# totals stay 0. File-count and extension classifications still work.
zassert_ok "$tmpdir" "tmpdir name set"
[[ -d "$tmpdir/src" ]] && r=1 || r=0; zassert_eq "$r" 1 "src/ exists"
[[ -f "$tmpdir/README.md" ]] && r=1 || r=0; zassert_eq "$r" 1 "README.md exists"
file_count=0
for f in "$tmpdir"/**/*(N.); do (( file_count++ )); done
zassert_eq "$file_count" 10 "10 regular files in fixture"
md_count=0
for f in "$tmpdir"/**/*.md(N.); do (( md_count++ )); done
zassert_eq "$md_count" 3 "3 markdown files (README/guide/intro)"
rs_count=0
for f in "$tmpdir"/**/*.rs(N.); do (( rs_count++ )); done
zassert_eq "$rs_count" 6 "6 .rs files"
ztest_run
