#!/usr/bin/env zshrs
# zstyle — context-pattern-based config store.
# Ported from Src/Modules/zutil.c bin_zstyle.

echo "── set styles ──"
zstyle ':completion:*' menu select
zstyle ':completion:*' verbose yes
zstyle ':completion:*:*:git:*' show-options true
zstyle ':completion:*:*:ssh:*' hosts host1 host2 host3

echo "── list (-L) ──"
zstyle -L

echo "── query single style ──"
zstyle -t ':completion:*' menu select
echo "test menu=select: $?"

echo "── retrieve string value ──"
zstyle -s ':completion:*' menu out
echo "menu = '$out'"

echo "── retrieve array ──"
zstyle -a ':completion:*:*:ssh:*' hosts hosts_arr
echo "hosts: ${hosts_arr[@]}"

echo "── pattern matching by context ──"
zstyle -s ':completion:*:*:git:*:checkout' show-options test_show
echo "git checkout context show-options: '$test_show'"

echo "── delete a style ──"
zstyle -d ':completion:*' menu
zstyle -L ':completion:*' | head -5

echo "── default value pattern (zstyle -s with || fallback) ──"
zstyle -s ':my:plugin' theme my_theme || my_theme=dracula
echo "theme: $my_theme"

echo "── boolean (zstyle -b) ──"
zstyle ':plugin:options' colorize true
zstyle -b ':plugin:options' colorize is_color
echo "colorize bool: $is_color"

echo "── int (zstyle -i for retrieval) ──"
zstyle ':plugin:options' verbosity 3
# Read as int.
zstyle -s ':plugin:options' verbosity v_str
v=$v_str
echo "verbosity: $v"

echo "── context wildcards ──"
zstyle ':plugin:*' enabled yes
for ctx in plugin:a plugin:b plugin:c; do
    zstyle -s ":$ctx" enabled e
    echo "$ctx enabled: $e"
done

# === ztest assertions ===
zassert_eq "$out"          "select" "zstyle -s round-trip"
zassert_eq "$test_show"    "true"   "zstyle context-pattern match"
zassert_eq "$my_theme"     "dracula" "zstyle -s fallback default"
zassert_eq "$is_color"     "yes"    "zstyle -b colorize"
zassert_eq "$v"            "3"      "zstyle -s int round-trip"
zassert_eq "$e"            "yes"    "zstyle wildcard context match"
zassert_eq "${#hosts_arr[@]}" "3"   "zstyle -a hosts count"
zassert_eq "${hosts_arr[1]}"  "host1" "zstyle -a hosts[1]"
zstyle -s ':completion:*' menu out_after
zassert_eq "$out_after" "" "zstyle -d removes the style"
ztest_run
