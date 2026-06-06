#!/usr/bin/env zshrs
# Hello world + basic variable assignment.
# Run: zshrs --zsh examples/demos/01_hello.zsh
name="zshrs"
greeting="Hello"
echo "$greeting, $name!"
echo "$greeting, world!"

# === ztest assertions ===
zassert_eq "$name"     "zshrs"  "name assignment"
zassert_eq "$greeting" "Hello"  "greeting assignment"
zassert_eq "$greeting, $name!"  "Hello, zshrs!"  "interpolation"
ztest_run
